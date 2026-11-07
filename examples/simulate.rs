//! `npm run simulate -- --seed 494 [--hunters 300] [--hunts 60] [--hp0-bps 300] [--burn-bps 1000] [--hoard-bps 100]`
//!
//! Runs the strategy simulation (hunts 1..=60; hunt 0 is an empty setup hunt) on a local sandbox against the committed res/calydon.wasm, once per
//! reward rule (A = HpCap, B = EarlyWeight), and writes sim/seed-<seed>-hp<bps>.json.
//!
//! Model: each hunt has ROUNDS rounds of STEP blocks. In each round the hunters act in a seeded
//! order, each seeing what was committed before it (as if transactions landed one by one); the
//! commits of a round are then sent together. Under rule A the simulator trims a round's commits to
//! the HP still missing in that order, which is what the contract would do if they landed in that
//! order, so results do not depend on how the sandbox packs blocks. Every contract return value is
//! checked against the simulator's own arithmetic, so the run fails if the two ever disagree.
#[path = "../tests/common/mod.rs"]
mod common;

use anyhow::{Context, Result, ensure};
use calydon::ceil_bps;
use common::*;
use futures::stream::{self, StreamExt, TryStreamExt};
use near_workspaces::network::Sandbox;
use near_workspaces::operations::Function;
use near_workspaces::types::{Gas, NearToken};
use near_workspaces::{Account, Contract, Worker};
use serde_json::{Value, json};

const C: u128 = 1_000_000_000_000_000_000;
const SUPPLY: u128 = 100_000_000 * C;
const ROUNDS: u64 = 4;
const STEP: u64 = 25;
const SIT_OUT: u64 = 3;

#[derive(Clone, Copy, PartialEq, Debug)]
enum Strategy {
    Early,
    Watcher,
    Quitter,
    LateWhale,
    SoloWhale,
}

impl Strategy {
    const ALL: [Strategy; 5] = [Self::Early, Self::Watcher, Self::Quitter, Self::LateWhale, Self::SoloWhale];
    fn name(self) -> &'static str {
        match self {
            Self::Early => "early committer",
            Self::Watcher => "threshold watcher",
            Self::Quitter => "quitter",
            Self::LateWhale => "late whale (wait-and-fill)",
            Self::SoloWhale => "solo whale",
        }
    }
}

struct Rng(u64);
impl Rng {
    fn new(parts: &[u64]) -> Self {
        let mut s = 0x9E37_79B9_7F4A_7C15u64;
        for p in parts {
            s = (s ^ p).wrapping_mul(0xBF58_476D_1CE4_E5B9).rotate_left(31) | 1;
        }
        let mut r = Rng(s);
        r.next();
        r
    }
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    /// Uniform in [lo, hi), in basis points.
    fn bps(&mut self, lo: u64, hi: u64) -> u128 {
        (lo + self.next() % (hi - lo)) as u128
    }
}

/// Per-hunter traits, drawn once from the seed; identical under both rules.
struct Hunter {
    strategy: Strategy,
    start: u128,
    /// Share of balance committed when it acts, in bps.
    fraction: u128,
    /// Chance of joining a given hunt (early committer, quitter), in bps.
    join: u128,
    /// Watcher: acts once committed/HP reaches this, in bps.
    threshold: u128,
}

fn population(seed: u64, n: usize) -> Vec<Hunter> {
    // 5% whales (half wait-and-fill, half solo), the rest split 35/35/25 early/watcher/quitter.
    let whales = (n / 20).max(2);
    let mut out = vec![];
    for i in 0..n {
        let mut r = Rng::new(&[seed, 1, i as u64]);
        let strategy = if i < whales / 2 {
            Strategy::LateWhale
        } else if i < whales {
            Strategy::SoloWhale
        } else {
            [Strategy::Early, Strategy::Watcher, Strategy::Early, Strategy::Watcher, Strategy::Quitter][(i - whales) % 5]
        };
        // Whales hold 30% of the supply between them; the others 30%; the launch holder keeps 40%.
        let whale = matches!(strategy, Strategy::LateWhale | Strategy::SoloWhale);
        let base = if whale { SUPPLY * 3 / 10 / whales as u128 } else { SUPPLY * 3 / 10 / (n - whales) as u128 };
        out.push(Hunter {
            strategy,
            start: base * r.bps(5_000, 15_000) / 10_000,
            fraction: if whale { 10_000 } else { r.bps(1_000, 4_000) },
            join: r.bps(4_000, 9_000),
            threshold: r.bps(3_000, 9_000),
        });
    }
    out
}

/// Largest stake whose tribute still fits in `balance`.
fn max_stake(balance: u128, tribute_bps: u32) -> u128 {
    let mut s = balance * 10_000 / (10_000 + tribute_bps as u128);
    while s > 0 && s + ceil_bps(s, tribute_bps) > balance {
        s -= 1;
    }
    s
}

struct Sim<'a> {
    worker: &'a Worker<Sandbox>,
    cal: Contract,
    rule_a: bool,
    burn_bps: u32,
    accounts: &'a [Account],
    hunters: &'a [Hunter],
    balance: Vec<u128>,
    staked: Vec<u128>,
    joined: Vec<u32>,
    sit_out_until: Vec<u64>,
}

impl Sim<'_> {
    async fn hunt(&mut self, seed: u64, n: u64) -> Result<Value> {
        let h = view(&self.cal, "current_hunt", json!({})).await?;
        ensure!(h["id"] == n);
        let (hp, deadline) = (num(&h["hp"])?, h["deadline"].as_u64().unwrap());
        let open = deadline - ROUNDS * STEP;
        let mut running = 0u128;
        let mut weight = 0u128;
        let mut entries: Vec<(usize, u128)> = vec![];
        let mut acted = vec![false; self.hunters.len()];
        for round in 0..ROUNDS {
            let now = height(self.worker).await?;
            let start = open + round * STEP;
            if now < start {
                self.worker.fast_forward(start - now).await?;
            }
            let mut order: Vec<usize> = (0..self.hunters.len()).collect();
            let mut r = Rng::new(&[seed, 2, n, round]);
            for i in (1..order.len()).rev() {
                order.swap(i, (r.next() % (i as u64 + 1)) as usize);
            }
            let mut batch = vec![];
            for &i in &order {
                if acted[i] {
                    continue;
                }
                let hunter = &self.hunters[i];
                let cap = max_stake(self.balance[i], 100);
                let gap = hp.saturating_sub(running);
                let joins = Rng::new(&[seed, 3, n, i as u64]).bps(0, 10_000) < hunter.join;
                let want = match hunter.strategy {
                    Strategy::Early if round == 0 && joins => cap * hunter.fraction / 10_000,
                    Strategy::Quitter if round == 0 && joins && n >= self.sit_out_until[i] => cap * hunter.fraction / 10_000,
                    Strategy::Watcher if round > 0 && running * 10_000 >= hp * hunter.threshold => cap * hunter.fraction / 10_000,
                    Strategy::LateWhale if round == ROUNDS - 1 && gap > 0 && gap <= cap => gap,
                    Strategy::SoloWhale if round == 0 && gap > 0 && gap <= cap => gap,
                    _ => 0,
                };
                let stake = if self.rule_a { want.min(gap) } else { want };
                if stake == 0 {
                    continue;
                }
                acted[i] = true;
                running += stake;
                weight += if self.rule_a { stake } else { stake * (ROUNDS - round) as u128 };
                batch.push((i, stake));
            }
            let cal = &self.cal;
            let accounts = self.accounts;
            let landed: Vec<(usize, u128, u128)> = stream::iter(batch)
                .map(|(i, stake)| async move { Ok::<_, anyhow::Error>((i, stake, returned(&commit(cal, &accounts[i], stake).await?)?)) })
                .buffer_unordered(64)
                .try_collect()
                .await?;
            for (i, stake, got) in landed {
                ensure!(got == stake, "hunt {n}: hunter {i} meant to lock {stake}, contract locked {got}");
                self.balance[i] -= stake + ceil_bps(stake, 100);
                self.staked[i] += stake;
                self.joined[i] += 1;
                entries.push((i, stake));
            }
        }
        let h = view(&self.cal, "current_hunt", json!({})).await?;
        ensure!(num(&h["committed"])? == running && num(&h["weight"])? == weight,
            "hunt {n}: contract committed/weight {}/{} != simulated {running}/{weight} (a round spilled into the next step)", h["committed"], h["weight"]);
        let now = height(self.worker).await?;
        if now < deadline {
            self.worker.fast_forward(deadline - now).await?;
        }
        let outcome: String = resolve(&self.cal, &self.accounts[0], n).await?.json()?;
        let settled = view(&self.cal, "hunt", json!({"id": n})).await?;
        let pot = num(&settled["pot"])?;
        let cal = &self.cal;
        let accounts = self.accounts;
        let paid: Vec<(usize, u128)> = stream::iter(entries.iter().map(|&(i, _)| i))
            .map(|i| async move { Ok::<_, anyhow::Error>((i, returned(&claim(cal, &accounts[i], n).await?)?)) })
            .buffer_unordered(64)
            .try_collect()
            .await?;
        for (i, got) in paid {
            let stake = entries.iter().find(|e| e.0 == i).unwrap().1;
            if outcome == "Escaped" {
                ensure!(got == stake - ceil_bps(stake, self.burn_bps), "hunt {n}: escaped refund mismatch for {i}");
                if self.hunters[i].strategy == Strategy::Quitter {
                    self.sit_out_until[i] = n + 1 + SIT_OUT;
                }
            }
            self.balance[i] += got;
        }
        let totals = view(&self.cal, "totals", json!({})).await?;
        Ok(json!({
            "hunt": n, "hp_cal": hp / C, "committed_cal": running / C, "hunters": entries.len(),
            "outcome": outcome, "pot_cal": pot / C, "hoard_cal": num(&totals["hoard"])? / C,
            "supply_cal": num(&totals["supply"])? / C,
        }))
    }
}

async fn setup_hunters(root: &Account, hunters: &[Hunter]) -> Result<Vec<Account>> {
    // Groups of 20 created in parallel; within a group one signer, so one transaction at a time.
    let groups: Vec<Account> = {
        let mut g = vec![];
        for k in 0..hunters.len().div_ceil(20) {
            g.push(subaccount(root, &format!("g{k:02}"), NearToken::from_near(20 * 3 + 2)).await?);
        }
        g
    };
    let per_group: Vec<Vec<Account>> = futures::future::try_join_all(groups.iter().enumerate().map(|(k, g)| async move {
        let mut out = vec![];
        for j in 0..20.min(hunters.len() - k * 20) {
            out.push(subaccount(g, &format!("h{:03}", k * 20 + j), NearToken::from_near(3)).await?);
        }
        Ok::<_, anyhow::Error>(out)
    }))
    .await?;
    Ok(per_group.into_iter().flatten().collect())
}

async fn fund(token: &Contract, holder: &Account, accounts: &[Account], hunters: &[Hunter]) -> Result<()> {
    for chunk in accounts.iter().zip(hunters).collect::<Vec<_>>().chunks(15) {
        let mut tx = holder.batch(token.id());
        for (a, h) in chunk {
            tx = tx
                .call(Function::new("storage_deposit").args_json(json!({"account_id": a.id()})).deposit(NearToken::from_millinear(2)).gas(Gas::from_tgas(8)))
                .call(Function::new("ft_transfer").args_json(json!({"receiver_id": a.id(), "amount": h.start.to_string()})).deposit(ONE).gas(Gas::from_tgas(8)));
        }
        let r = tx.transact().await?;
        ensure!(r.is_success(), "funding failed: {:?}", failures(&r));
    }
    Ok(())
}

fn arg(args: &[String], name: &str, default: u64) -> Result<u64> {
    match args.iter().position(|a| a == name) {
        Some(i) => args.get(i + 1).with_context(|| format!("{name} needs a value"))?.parse().with_context(|| name.to_string()),
        None => Ok(default),
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let seed = arg(&args, "--seed", 494)?;
    let n_hunters = arg(&args, "--hunters", 300)? as usize;
    let n_hunts = arg(&args, "--hunts", 60)?;
    let hp0_bps = arg(&args, "--hp0-bps", 300)? as u128;
    let burn_bps = arg(&args, "--burn-bps", 1_000)? as u32;
    let hoard_bps = arg(&args, "--hoard-bps", 100)? as u128;
    let started = std::time::Instant::now();

    let hunters = population(seed, n_hunters);
    let worker = near_workspaces::sandbox().await?;
    let root = worker.root_account()?;
    let holder = subaccount(&root, "holder", NearToken::from_near(50)).await?;
    let accounts = setup_hunters(&root, &hunters).await?;
    eprintln!("{} hunters created in {:?}", accounts.len(), started.elapsed());

    let mut results = vec![];
    for (rule, label) in [("HpCap", "A (HP cap)"), ("EarlyWeight", "B (early-commit weighting)")] {
        let hoard = SUPPLY * hoard_bps / 10_000;
        let mut cfg = config(rule, SUPPLY * hp0_bps / 10_000 * 5 / 4, ROUNDS * STEP, STEP);
        cfg["burn_bps"] = json!(burn_bps);
        let (cal, _) = deploy(&root, &format!("cal-{}", rule.to_lowercase()), holder.id(), SUPPLY - hoard, hoard, cfg).await?;
        fund(&cal, &holder, &accounts, &hunters).await?;
        // Hunt 0 opened at `new` and its blocks went to funding; it resolves empty (escaped,
        // nothing burns), so hunt 1 opens at exactly the first HP asked for (x1.25 then x0.8).
        let h0 = view(&cal, "current_hunt", json!({})).await?;
        pass_deadline(&worker, h0["deadline"].as_u64().unwrap()).await?;
        ensure!(h0["hunters"] == 0 && resolve(&cal, &holder, 0).await?.json::<String>()? == "Escaped");
        let mut sim = Sim {
            worker: &worker, cal, rule_a: rule == "HpCap", burn_bps, accounts: &accounts, hunters: &hunters,
            balance: hunters.iter().map(|h| h.start).collect(), staked: vec![0; hunters.len()],
            joined: vec![0; hunters.len()], sit_out_until: vec![0; hunters.len()],
        };
        let mut log = vec![];
        for n in 1..=n_hunts {
            let row = sim.hunt(seed, n).await?;
            eprintln!("rule {rule} {row} ({:?})", started.elapsed());
            log.push(row);
        }
        // The simulator's balances must equal the chain's, and I1 must hold, at the end.
        let mut ids: Vec<_> = accounts.iter().map(|a| a.id()).collect();
        ids.push(holder.id());
        let totals = conservation(&sim.cal, &ids, SUPPLY).await?;
        for (i, a) in accounts.iter().enumerate() {
            ensure!(balance(&sim.cal, a.id()).await? == sim.balance[i], "balance drift for {}", a.id());
        }
        let kills = log.iter().filter(|r| r["outcome"] == "Slain").count();
        let supply_end = num(&totals["supply"])?;
        let strategies: Vec<Value> = Strategy::ALL.iter().map(|&s| {
            let idx: Vec<usize> = (0..hunters.len()).filter(|&i| hunters[i].strategy == s).collect();
            let start: u128 = idx.iter().map(|&i| hunters[i].start).sum();
            let end: u128 = idx.iter().map(|&i| sim.balance[i]).sum();
            let staked: u128 = idx.iter().map(|&i| sim.staked[i]).sum();
            let joined: u32 = idx.iter().map(|&i| sim.joined[i]).sum();
            let net = end as f64 - start as f64;
            // Share of supply: burns raise everyone's share, including people who never play.
            let share = (end as f64 / supply_end as f64) / (start as f64 / SUPPLY as f64) - 1.0;
            json!({
                "strategy": s.name(), "hunters": idx.len(), "joins": joined,
                "net_cal": (net / C as f64).round(), "net_pct_of_start": round2(100.0 * net / start as f64),
                "net_per_100_staked": round2(if staked == 0 { 0.0 } else { 100.0 * net / staked as f64 }),
                "supply_share_change_pct": round2(100.0 * share),
            })
        }).collect();
        results.push(json!({
            "rule": label, "hunts": n_hunts, "kills": kills,
            "kill_rate_pct": round2(100.0 * kills as f64 / n_hunts as f64),
            "supply_end_cal": supply_end / C, "burned_cal": num(&totals["burned"])? / C,
            "strategies": strategies, "log": log,
        }));
    }

    let report = json!({
        "seed": seed, "hunters": n_hunters, "hp0_bps_of_supply": hp0_bps,
        "wasm_sha256": std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/res/calydon.wasm.sha256"))?.split_whitespace().next(),
        "model": {"rounds": ROUNDS, "step_blocks": STEP, "quitter_sits_out": SIT_OUT, "supply_cal": SUPPLY / C,
            "burn_bps": burn_bps, "tribute_bps": 100,
            "holdings": format!("whales 30% (5% of hunters), others 30%, non-playing launch holder {}%, launch hoard {}%", 40.0 - hoard_bps as f64 / 100.0, hoard_bps as f64 / 100.0)},
        "results": results,
    });
    let levers = if (burn_bps, hoard_bps) == (1_000, 100) { String::new() } else { format!("-burn{burn_bps}-hoard{hoard_bps}") };
    let name = format!("seed-{seed}-hp{hp0_bps}{levers}.json");
    let path = format!("{}/sim/{name}", env!("CARGO_MANIFEST_DIR"));
    std::fs::create_dir_all(format!("{}/sim", env!("CARGO_MANIFEST_DIR")))?;
    std::fs::write(&path, serde_json::to_string_pretty(&report)? + "\n")?;

    println!("Calydon simulation: seed {seed}, {n_hunters} hunters, {n_hunts} hunts, first HP {}% of supply, burn {}%, launch hoard {}%, wasm {}",
        hp0_bps as f64 / 100.0, burn_bps as f64 / 100.0, hoard_bps as f64 / 100.0, report["wasm_sha256"].as_str().unwrap_or("?"));
    for r in report["results"].as_array().unwrap() {
        println!("\nRule {}: kill rate {}% ({}/{}), supply {} -> {} CAL", r["rule"].as_str().unwrap(), r["kill_rate_pct"], r["kills"], r["hunts"], SUPPLY / C, r["supply_end_cal"]);
        println!("  {:<28} {:>7} {:>6} {:>12} {:>12} {:>14}", "strategy", "hunters", "joins", "net % start", "net/100 stk", "supply share");
        for s in r["strategies"].as_array().unwrap() {
            let cell = |k: &str| s[k].to_string();
            println!("  {:<28} {:>7} {:>6} {:>11}% {:>12} {:>13}%", s["strategy"].as_str().unwrap(), cell("hunters"), cell("joins"),
                cell("net_pct_of_start"), cell("net_per_100_staked"), cell("supply_share_change_pct"));
        }
    }
    println!("\nWritten: sim/{name} ({:?})", started.elapsed());
    Ok(())
}

fn round2(x: f64) -> f64 {
    (x * 100.0).round() / 100.0
}

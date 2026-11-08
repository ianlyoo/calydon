//! `npm run simulate -- --seed 494 [--hunters 300] [--hunts 60] [--hp0-bps 300] [--burn-bps 1000] [--hoard-bps 100]
//!   [--hp-up 11000 --hp-down 7000] [--split-k 8] [--set config_field=value ...]`
//!
//! Runs the strategy simulation (hunts 1..=60; hunt 0 is an empty setup hunt) on a local sandbox against the committed
//! res/calydon.wasm with the v1.0 launch values (`launch_config`), once per reward rule (B = EarlyWeight, the launch rule,
//! and A = HpCap for comparison), and writes sim/seed-<seed>-hp<bps>[-burn..-hoard..][-ctl<up>-<down>][-<field><value>].json.
//! `--set fill_fee_bps=0` runs the same wasm without the fill fee (the pre-v1.0 entry fee).
//!
//! Model: each hunt has ROUNDS rounds of STEP blocks. In each round the hunters act in a seeded
//! order, each seeing what was committed before it (as if transactions landed one by one); the
//! commits of a round are then sent together. Under rule A the simulator trims a round's commits to
//! the HP still missing in that order, so the contract never has to trim and the stakes do not
//! depend on how the sandbox packs blocks. Weights and fees may depend on what was committed before
//! a commit landed, so the simulator takes each account's balance from the chain after every round
//! and every claim instead of predicting it; stakes, escaped refunds and the hunt's committed total
//! are still checked against the simulator's own arithmetic.
//!
//! A split whale is a solo whale's twin (same drawn holding) spread evenly over `--split-k` accounts;
//! when its first account comes up in round 1 and its accounts together can fill the HP still missing,
//! it fills it with one commit per account. Each extra account needs its own NEP-145 storage deposit (NEAR,
//! reported next to the CAL results).
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
const TRIBUTE_BPS: u32 = 100;
const VARIANT: &str = "v1.0";

#[derive(Clone, Copy, PartialEq, Debug)]
enum Strategy {
    Early,
    Watcher,
    Quitter,
    LateWhale,
    SoloWhale,
    SplitWhale,
}

impl Strategy {
    const ALL: [Strategy; 6] = [Self::Early, Self::Watcher, Self::Quitter, Self::LateWhale, Self::SoloWhale, Self::SplitWhale];
    fn name(self) -> &'static str {
        match self {
            Self::Early => "early committer",
            Self::Watcher => "threshold watcher",
            Self::Quitter => "quitter",
            Self::LateWhale => "late whale (wait-and-fill)",
            Self::SoloWhale => "solo whale",
            Self::SplitWhale => "split whale",
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

/// Per-account traits, drawn once from the seed; identical under both rules.
struct Hunter {
    strategy: Strategy,
    start: u128,
    /// Share of balance committed when it acts, in bps.
    fraction: u128,
    /// Chance of joining a given hunt (early committer, quitter), in bps.
    join: u128,
    /// Watcher: acts once committed/HP reaches this, in bps.
    threshold: u128,
    /// The account that decides for this one: itself, or a split whale's first account.
    leader: usize,
}

fn population(seed: u64, n: usize, split_k: usize) -> Vec<Hunter> {
    // 5% whales (about half wait-and-fill, a quarter solo, a quarter split whales that are the solo
    // whales' twins), the rest split
    // 35/35/25 early/watcher/quitter. A split whale's extra accounts are appended after the n hunters.
    let whales = (n / 20).max(2);
    let late = whales / 2;
    let solo = late + (whales - late) / 2;
    let mut out = vec![];
    let mut extra = vec![];
    for i in 0..n {
        // A split whale is the twin of a solo whale: it draws the same holding.
        let twin = if (solo..whales).contains(&i) { i - (whales - solo) } else { i };
        let mut r = Rng::new(&[seed, 1, twin as u64]);
        let strategy = if i < late {
            Strategy::LateWhale
        } else if i < solo {
            Strategy::SoloWhale
        } else if i < whales {
            Strategy::SplitWhale
        } else {
            [Strategy::Early, Strategy::Watcher, Strategy::Early, Strategy::Watcher, Strategy::Quitter][(i - whales) % 5]
        };
        // Whales hold 30% of the supply between them; the others 30%; the launch holder keeps 40%.
        let whale = matches!(strategy, Strategy::LateWhale | Strategy::SoloWhale | Strategy::SplitWhale);
        let base = if whale { SUPPLY * 3 / 10 / whales as u128 } else { SUPPLY * 3 / 10 / (n - whales) as u128 };
        let start = base * r.bps(5_000, 15_000) / 10_000;
        let fraction = if whale { 10_000 } else { r.bps(1_000, 4_000) };
        let (join, threshold) = (r.bps(4_000, 9_000), r.bps(3_000, 9_000));
        let mut h = Hunter { strategy, start, fraction, join, threshold, leader: i };
        if strategy == Strategy::SplitWhale {
            let part = start / split_k as u128;
            h.start = start - part * (split_k as u128 - 1);
            for _ in 1..split_k {
                extra.push(Hunter { strategy, start: part, fraction, join, threshold, leader: i });
            }
        }
        out.push(h);
    }
    out.extend(extra);
    out
}

/// Largest stake whose fee (at `fee_bps`) still fits in `balance`.
fn max_stake(balance: u128, fee_bps: u32) -> u128 {
    let mut s = balance * 10_000 / (10_000 + fee_bps as u128);
    while s > 0 && s + ceil_bps(s, fee_bps) > balance {
        s -= 1;
    }
    s
}

struct Sim<'a> {
    worker: &'a Worker<Sandbox>,
    cal: Contract,
    rule_a: bool,
    burn_bps: u32,
    fee_max_bps: u32,
    accounts: &'a [Account],
    hunters: &'a [Hunter],
    members: Vec<Vec<usize>>,
    balance: Vec<u128>,
    staked: Vec<u128>,
    fees: Vec<u128>,
    received: Vec<u128>,
    joined: Vec<u32>,
    sit_out_until: Vec<u64>,
}

impl Sim<'_> {
    async fn chain_balances(&self, idx: &[usize]) -> Result<Vec<(usize, u128)>> {
        let cal = &self.cal;
        let accounts = self.accounts;
        stream::iter(idx.iter().copied())
            .map(|i| async move { Ok::<_, anyhow::Error>((i, balance(cal, accounts[i].id()).await?)) })
            .buffer_unordered(64)
            .try_collect()
            .await
    }

    async fn hunt(&mut self, seed: u64, n: u64) -> Result<Value> {
        let h = view(&self.cal, "current_hunt", json!({})).await?;
        ensure!(h["id"] == n);
        let (hp, deadline) = (num(&h["hp"])?, h["deadline"].as_u64().unwrap());
        let open = deadline - ROUNDS * STEP;
        let mut running = 0u128;
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
                let gap = hp.saturating_sub(running);
                if hunter.strategy == Strategy::SplitWhale {
                    // It decides once, at its first account's place in the order, like its solo twin.
                    if hunter.leader != i {
                        continue;
                    }
                    let group = &self.members[hunter.leader];
                    for &m in group {
                        acted[m] = true;
                    }
                    let caps: Vec<u128> = group.iter().map(|&m| max_stake(self.balance[m], self.fee_max_bps)).collect();
                    let total: u128 = caps.iter().sum();
                    if round != 0 || gap == 0 || gap > total {
                        continue;
                    }
                    // Even parts in proportion to what each account can stake; the rounding rest goes
                    // to the first account with room.
                    let mut parts: Vec<u128> = caps.iter().map(|&c| calydon::share(gap, c, total)).collect();
                    let mut rest = gap - parts.iter().sum::<u128>();
                    for (p, &c) in parts.iter_mut().zip(&caps) {
                        let add = rest.min(c - *p);
                        *p += add;
                        rest -= add;
                    }
                    for (&m, &p) in group.iter().zip(&parts) {
                        if p > 0 {
                            running += p;
                            batch.push((m, p));
                        }
                    }
                    continue;
                }
                let cap = max_stake(self.balance[i], self.fee_max_bps);
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
                batch.push((i, stake));
            }
            let cal = &self.cal;
            let accounts = self.accounts;
            let landed: Vec<(usize, u128, u128)> = stream::iter(batch)
                .map(|(i, stake)| async move { Ok::<_, anyhow::Error>((i, stake, returned(&commit(cal, &accounts[i], stake).await?)?)) })
                .buffer_unordered(64)
                .try_collect()
                .await?;
            let idx: Vec<usize> = landed.iter().map(|l| l.0).collect();
            let after = self.chain_balances(&idx).await?;
            for (i, stake, got) in landed {
                ensure!(got == stake, "hunt {n}: account {i} meant to lock {stake}, contract locked {got}");
                let now = after.iter().find(|a| a.0 == i).unwrap().1;
                let paid = self.balance[i] - now;
                ensure!(paid >= stake + ceil_bps(stake, TRIBUTE_BPS), "hunt {n}: account {i} paid {paid} for stake {stake}");
                ensure!(paid <= stake + ceil_bps(stake, self.fee_max_bps), "hunt {n}: account {i} paid {paid}, above --fee-max-bps");
                self.fees[i] += paid - stake;
                self.balance[i] = now;
                self.staked[i] += stake;
                if self.hunters[i].leader == i {
                    self.joined[i] += 1;
                }
                entries.push((i, stake));
            }
        }
        let h = view(&self.cal, "current_hunt", json!({})).await?;
        ensure!(num(&h["committed"])? == running,
            "hunt {n}: contract committed {} != simulated {running} (a round spilled into the next step)", h["committed"]);
        let weight = num(&h["weight"])?;
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
            } else {
                ensure!(got >= stake, "hunt {n}: slain payout below stake for {i}");
                self.received[i] += got - stake;
            }
            self.balance[i] += got;
        }
        let totals = view(&self.cal, "totals", json!({})).await?;
        Ok(json!({
            "hunt": n, "hp_cal": hp / C, "committed_cal": running / C, "hunters": entries.len(),
            "weight_per_committed": if running == 0 { 0.0 } else { round2(weight as f64 / running as f64) },
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

/// Every `--set field=value` pair (integer launch constants added by a variant).
fn sets(args: &[String]) -> Result<Vec<(String, u64)>> {
    let mut out = vec![];
    for (i, a) in args.iter().enumerate() {
        if a == "--set" {
            let kv = args.get(i + 1).context("--set needs field=value")?;
            let (k, v) = kv.split_once('=').context("--set needs field=value")?;
            out.push((k.to_string(), v.parse().with_context(|| kv.clone())?));
        }
    }
    Ok(out)
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
    let hp_up = arg(&args, "--hp-up", 11_000)? as u128;
    let hp_down = arg(&args, "--hp-down", 7_000)? as u128;
    let split_k = arg(&args, "--split-k", 8)? as usize;
    let overrides = sets(&args)?;
    let fill_fee_bps = overrides.iter().find(|(k, _)| k == "fill_fee_bps").map_or(1_000, |o| o.1) as u32;
    // The most a commit can pay on top of its stake: every token past a full bar.
    let fee_max_bps = TRIBUTE_BPS + fill_fee_bps;
    let started = std::time::Instant::now();

    let hunters = population(seed, n_hunters, split_k);
    let mut members = vec![vec![]; hunters.len()];
    for (i, h) in hunters.iter().enumerate() {
        members[h.leader].push(i);
    }
    let worker = near_workspaces::sandbox().await?;
    let root = worker.root_account()?;
    let holder = subaccount(&root, "holder", NearToken::from_near(50)).await?;
    let accounts = setup_hunters(&root, &hunters).await?;
    eprintln!("{} accounts ({} hunters) created in {:?}", accounts.len(), n_hunters, started.elapsed());

    let first_hp = SUPPLY * hp0_bps / 10_000;
    let mut results = vec![];
    let mut storage_min = 0u128;
    for (rule, label) in [("EarlyWeight", "B (early-commit weighting, launch rule)"), ("HpCap", "A (HP cap)")] {
        let hoard = SUPPLY * hoard_bps / 10_000;
        // Hunt 0 resolves empty (escaped, x hp_down rounded down); launching at ceil(first HP / hp_down)
        // makes hunt 1 open at exactly the first HP.
        let mut cfg = launch_config((first_hp * 10_000).div_ceil(hp_down), ROUNDS * STEP, STEP);
        cfg["rule"] = json!(rule);
        cfg["burn_bps"] = json!(burn_bps);
        cfg["hp_up_bps"] = json!(hp_up);
        cfg["hp_down_bps"] = json!(hp_down);
        for (k, v) in &overrides {
            cfg[k.as_str()] = json!(v);
        }
        let (cal, _) = deploy(&root, &format!("cal-{}", rule.to_lowercase()), holder.id(), SUPPLY - hoard, hoard, cfg).await?;
        storage_min = num(&view(&cal, "storage_balance_bounds", json!({})).await?["min"])?;
        fund(&cal, &holder, &accounts, &hunters).await?;
        let h0 = view(&cal, "current_hunt", json!({})).await?;
        pass_deadline(&worker, h0["deadline"].as_u64().unwrap()).await?;
        ensure!(h0["hunters"] == 0 && resolve(&cal, &holder, 0).await?.json::<String>()? == "Escaped");
        ensure!(num(&view(&cal, "current_hunt", json!({})).await?["hp"])? == first_hp, "hunt 1 HP != first HP");
        let n = hunters.len();
        let mut sim = Sim {
            worker: &worker, cal, rule_a: rule == "HpCap", burn_bps, fee_max_bps, accounts: &accounts, hunters: &hunters,
            members: members.clone(), balance: hunters.iter().map(|h| h.start).collect(), staked: vec![0; n],
            fees: vec![0; n], received: vec![0; n], joined: vec![0; n], sit_out_until: vec![0; n],
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
            let sum = |v: &[u128]| idx.iter().map(|&i| v[i]).sum::<u128>();
            let start: u128 = idx.iter().map(|&i| hunters[i].start).sum();
            let end = sum(&sim.balance);
            let staked = sum(&sim.staked);
            let joined: u32 = idx.iter().map(|&i| sim.joined[i]).sum();
            let net = end as f64 - start as f64;
            // Share of supply: burns raise everyone's share, including people who never play.
            let share = (end as f64 / supply_end as f64) / (start as f64 / SUPPLY as f64) - 1.0;
            json!({
                "strategy": s.name(), "hunters": idx.iter().filter(|&&i| hunters[i].leader == i).count(), "accounts": idx.len(),
                "storage_deposit_near": idx.len() as f64 * storage_min as f64 / 1e24,
                "joins": joined, "fees_paid_cal": (sum(&sim.fees) as f64 / C as f64).round(),
                "hoard_received_cal": (sum(&sim.received) as f64 / C as f64).round(),
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
            "burn_bps": burn_bps, "tribute_bps": TRIBUTE_BPS, "hp_up_bps": hp_up, "hp_down_bps": hp_down, "variant": VARIANT,
            "config_overrides": overrides.iter().map(|(k, v)| (k.clone(), json!(v))).collect::<serde_json::Map<_, _>>(),
            "split_whale_accounts_each": split_k, "storage_deposit_min_near": storage_min as f64 / 1e24, "fee_max_bps": fee_max_bps,
            "holdings": format!("whales 30% (5% of hunters), others 30%, non-playing launch holder {}%, launch hoard {}%", 40.0 - hoard_bps as f64 / 100.0, hoard_bps as f64 / 100.0)},
        "results": results,
    });
    let levers = if (burn_bps, hoard_bps) == (1_000, 100) { String::new() } else { format!("-burn{burn_bps}-hoard{hoard_bps}") };
    let ctl = if (hp_up, hp_down) == (11_000, 7_000) { String::new() } else { format!("-ctl{hp_up}-{hp_down}") };
    let tags: String = overrides.iter().map(|(k, v)| format!("-{}{v}", k.trim_end_matches("_bps").replace('_', ""))).collect();
    let name = format!("seed-{seed}-hp{hp0_bps}{levers}{ctl}{tags}.json");
    let path = format!("{}/sim/{name}", env!("CARGO_MANIFEST_DIR"));
    std::fs::create_dir_all(format!("{}/sim", env!("CARGO_MANIFEST_DIR")))?;
    std::fs::write(&path, serde_json::to_string_pretty(&report)? + "\n")?;

    println!("Calydon simulation ({VARIANT}): seed {seed}, {n_hunters} hunters, {n_hunts} hunts, first HP {}% of supply, HP x{}/x{}, burn {}%, launch hoard {}%, wasm {}",
        hp0_bps as f64 / 100.0, hp_up as f64 / 10_000.0, hp_down as f64 / 10_000.0, burn_bps as f64 / 100.0, hoard_bps as f64 / 100.0,
        report["wasm_sha256"].as_str().unwrap_or("?"));
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

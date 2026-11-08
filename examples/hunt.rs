//! `npm run hunt -- <command>`: play Calydon on NEAR testnet from faucet accounts.
//!
//!   launch [--hunt-blocks 600]   your own instance (no access key on the contract account): a faucet
//!                                account is the launch holder; alice and bob take their 1000-CAL
//!                                starters (keys under .credentials/testnet/)
//!   status                       the current hunt, the hoard and the supply (read-only)
//!   starter --as <name>          take the one-time 1000-CAL starter (registers the account first)
//!   commit <CAL> --as <name>     lock a stake in the current hunt; the entry fee is paid on top
//!   resolve [--as <name>]        settle the current hunt once its deadline has passed (anyone)
//!   claim <hunt> --as <name>     take your settlement for a resolved hunt
//!
//! The instance is `--contract <id>`, else the last `launch` (.credentials/testnet/play.json), else
//! the recorded v1.1 instance in testnet/run.json (any testnet account can take a starter and play).
//! `<name>` is alice, bob, holder or a full account id with a key file in .credentials/testnet/.
#[path = "../tests/common/mod.rs"]
mod common;

use anyhow::{Context, Result, bail};
use calydon::fill_fee;
use common::*;
use near_workspaces::network::Testnet;
use near_workspaces::{Account, Contract, Worker};
use serde_json::{Value, json};

const C: u128 = 1_000_000_000_000_000_000;
const KEYS: &str = ".credentials/testnet";

fn flag<'a>(args: &'a [String], name: &str) -> Option<&'a str> {
    args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).map(String::as_str)
}

/// "12.5" CAL -> yocto-CAL (18 decimals).
fn parse_cal(text: &str) -> Result<u128> {
    let (whole, frac) = text.split_once('.').unwrap_or((text, ""));
    if frac.len() > 18 {
        bail!("at most 18 decimals: {text}");
    }
    let frac = format!("{frac:0<18}");
    Ok(whole.parse::<u128>().context("amount")? * C + frac.parse::<u128>().context("amount")?)
}

fn show(yocto: u128) -> String {
    let frac = format!("{:018}", yocto % C);
    let frac = frac.trim_end_matches('0');
    if frac.is_empty() { format!("{}", yocto / C) } else { format!("{}.{frac}", yocto / C) }
}

fn read_json(path: &str) -> Option<Value> {
    std::fs::read_to_string(path).ok().and_then(|s| serde_json::from_str(&s).ok())
}

fn instance(args: &[String]) -> Result<String> {
    if let Some(id) = flag(args, "--contract") {
        return Ok(id.to_string());
    }
    for path in [&format!("{KEYS}/play.json"), "testnet/run.json"] {
        if let Some(v) = read_json(path) {
            return Ok(v["contract"].as_str().context("contract")?.to_string());
        }
    }
    bail!("no instance: run `npm run hunt -- launch` or pass --contract <id>")
}

fn signer(worker: &Worker<Testnet>, contract: &str, name: &str) -> Result<Account> {
    // alice -> alice.<root> where the contract is cal.<root>; holder -> <root>.
    let root = contract.strip_prefix("cal.").unwrap_or(contract);
    let id = match name {
        "holder" => root.to_string(),
        n if n.contains('.') => n.to_string(),
        n => format!("{n}.{root}"),
    };
    let path = format!("{KEYS}/{id}.json");
    Account::from_file(&path, worker).with_context(|| format!("no key for {id} at {path}"))
}

async fn status(cal: &Contract, worker: &Worker<Testnet>) -> Result<()> {
    let (h, t, cfg) = (view(cal, "current_hunt", json!({})).await?, view(cal, "totals", json!({})).await?, view(cal, "config", json!({})).await?);
    let (hp, committed) = (num(&h["hp"])?, num(&h["committed"])?);
    let now = height(worker).await?;
    let deadline = h["deadline"].as_u64().context("deadline")?;
    let filled = (committed.min(hp) * 100 / hp) as usize;
    println!("contract {} (rule {}, tribute {} bps + fill fee up to {} bps, burn {} bps, HP x{} / x{})", cal.id(), cfg["rule"].as_str().unwrap_or("?"),
        cfg["tribute_bps"], cfg["fill_fee_bps"], cfg["burn_bps"], cfg["hp_up_bps"].as_f64().unwrap_or(0.0) / 1e4, cfg["hp_down_bps"].as_f64().unwrap_or(0.0) / 1e4);
    println!("hunt {}: [{}{}] {} / {} CAL committed by {} hunter(s)", h["id"], "#".repeat(filled / 5), ".".repeat(20 - filled / 5), show(committed), show(hp), h["hunters"]);
    if now < deadline {
        let fee = fill_fee(committed, C, hp, cfg["tribute_bps"].as_u64().unwrap_or(0) as u32, cfg["fill_fee_bps"].as_u64().unwrap_or(0) as u32);
        println!("  open: {} blocks left (deadline {deadline}); the next 1 CAL committed pays {} CAL fee (a larger stake pays the fee integrated over the fill it covers)", deadline - now, show(fee));
    } else {
        println!("  deadline {deadline} passed: `npm run hunt -- resolve` settles it ({})", if committed >= hp && committed > 0 { "will be Slain" } else { "will be Escaped" });
    }
    println!("hoard {} CAL · supply {} CAL · burned {} CAL · unclaimed {} CAL · starter bucket {} CAL", show(num(&t["hoard"])?), show(num(&t["supply"])?), show(num(&t["burned"])?), show(num(&t["unclaimed"])?), show(num(&t["starter_left"])?));
    Ok(())
}

fn done(what: &str, r: &near_workspaces::result::ExecutionFinalResult) -> Result<Value> {
    let link = format!("https://testnet.nearblocks.io/txns/{}", r.outcome().transaction_hash);
    if r.is_failure() {
        bail!("{what} refused: {}\n  {link}", failures(r).join("; "));
    }
    println!("{what}\n  {link}");
    Ok(r.clone().json().unwrap_or(Value::Null))
}

#[tokio::main]
async fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let worker = testnet().await?;
    match args.first().map(String::as_str) {
        Some("launch") => {
            let blocks: u64 = flag(&args, "--hunt-blocks").unwrap_or("600").parse().context("--hunt-blocks")?;
            let cfg = launch_config(1_000 * C, blocks, blocks.div_ceil(4));
            let l = launch_testnet(&worker, "calydon-play", &["alice", "bob"], cfg, 10_000 * C, 10_000 * C, 980_000 * C, KEYS).await?;
            std::fs::write(format!("{KEYS}/play.json"), serde_json::to_string_pretty(&json!({"contract": l.cal.id(), "holder": l.root.id()}))?)?;
            println!("launched {} (no access key); holder {}; alice and bob took 1000-CAL starters", l.cal.id(), l.root.id());
            status(&l.cal, &worker).await
        }
        Some(cmd @ ("status" | "starter" | "commit" | "resolve" | "claim")) => {
            let id = instance(&args)?;
            let cal = Contract::from_secret_key(id.parse()?, near_workspaces::types::SecretKey::from_random(near_workspaces::types::KeyType::ED25519), &worker);
            match cmd {
                "status" => status(&cal, &worker).await,
                "starter" => {
                    let who = signer(&worker, &id, flag(&args, "--as").context("starter needs --as <name>")?)?;
                    if view(&cal, "storage_balance_of", json!({"account_id": who.id()})).await?.is_null() {
                        done(&format!("{} registered (storage_deposit)", who.id()), &register(&cal, &who).await?)?;
                    }
                    let got = num(&done(&format!("{} took the starter", who.id()), &starter(&cal, &who).await?)?)?;
                    println!("  got {} CAL; balance now {} CAL", show(got), show(balance(&cal, who.id()).await?));
                    Ok(())
                }
                "commit" => {
                    let amount = parse_cal(args.get(1).context("commit <CAL> --as <name>")?)?;
                    let who = signer(&worker, &id, flag(&args, "--as").context("commit needs --as <name>")?)?;
                    let locked = num(&done(&format!("{} committed {} CAL", who.id(), show(amount)), &commit(&cal, &who, amount).await?)?)?;
                    println!("  locked {} CAL", show(locked));
                    status(&cal, &worker).await
                }
                "resolve" => {
                    let who = signer(&worker, &id, flag(&args, "--as").unwrap_or("holder"))?;
                    let hunt = view(&cal, "current_hunt", json!({})).await?["id"].as_u64().context("id")?;
                    let outcome = done(&format!("hunt {hunt} resolved by {}", who.id()), &resolve(&cal, &who, hunt).await?)?;
                    println!("  outcome {outcome}");
                    status(&cal, &worker).await
                }
                _ => {
                    let hunt: u64 = args.get(1).context("claim <hunt> --as <name>")?.parse().context("hunt id")?;
                    let who = signer(&worker, &id, flag(&args, "--as").context("claim needs --as <name>")?)?;
                    let paid = num(&done(&format!("{} claimed hunt {hunt}", who.id()), &claim(&cal, &who, hunt).await?)?)?;
                    println!("  paid {} CAL; balance now {} CAL", show(paid), show(balance(&cal, who.id()).await?));
                    Ok(())
                }
            }
        }
        _ => bail!("usage: npm run hunt -- launch [--hunt-blocks N] | status | starter --as <name> | commit <CAL> --as <name> | resolve [--as <name>] | claim <hunt> --as <name>  [--contract <id>]"),
    }
}


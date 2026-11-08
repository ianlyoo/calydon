//! `npm run execute-testnet`: launches a fresh v1.1 instance on NEAR testnet with the launch values
//! (created, deployed and initialised in one transaction, so the contract account never has an
//! access key), gives three hunters their starter CAL, then runs three hunts: slain, escaped (with a
//! plain transfer to the contract that resolve sweeps into the hoard), slain. Every transaction is
//! checked against the contract's arithmetic and written with its link to testnet/run.json.
//! Keys are random, saved under the ignored .credentials/, never printed.
mod common;

use anyhow::{Result, ensure};
use calydon::{fill_fee, share};
use common::*;
use near_workspaces::result::ExecutionFinalResult;
use near_workspaces::{Account, Contract};
use serde_json::{Value, json};

const C: u128 = 1_000_000_000_000_000_000;
// 300 blocks is about 3 minutes on testnet; 4 steps of 75 blocks for rule B's weights.
const HUNT_BLOCKS: u64 = 300;
const STEP_BLOCKS: u64 = 75;

struct Log(Vec<Value>);
impl Log {
    fn tx(&mut self, step: &str, r: &ExecutionFinalResult) -> Result<Value> {
        ensure!(r.is_success(), "{step} failed: {:?}", failures(r));
        let tx = r.outcome().transaction_hash.to_string();
        let value: Value = r.clone().json().unwrap_or(Value::Null);
        let link = format!("https://testnet.nearblocks.io/txns/{tx}");
        println!("{step}: returned {value}\n  {link}");
        self.0.push(json!({"step": step, "tx": tx, "link": link, "signer": r.outcome().executor_id, "returned": value, "events": events(r)}));
        Ok(value)
    }
}

/// Commits `stake` and checks the fee the contract took against `fill_fee` at the bar's fill before it.
async fn commit_checked(log: &mut Log, cal: &Contract, who: &Account, stake: u128, hunt: u64) -> Result<u128> {
    let h = view(cal, "current_hunt", json!({})).await?;
    let (before, hp) = (num(&h["committed"])?, num(&h["hp"])?);
    let cfg = view(cal, "config", json!({})).await?;
    let fee = fill_fee(before, stake, hp, cfg["tribute_bps"].as_u64().unwrap() as u32, cfg["fill_fee_bps"].as_u64().unwrap() as u32);
    let had = balance(cal, who.id()).await?;
    let name = who.id().as_str().split('.').next().unwrap();
    let r = commit(cal, who, stake).await?;
    log.tx(&format!("hunt {hunt}: {name} commit {} CAL (bar {}/{} CAL, fee {:.4} CAL)", stake / C, before / C, hp / C, fee as f64 / C as f64), &r)?;
    ensure!(had - balance(cal, who.id()).await? == stake + fee, "{name}: paid {} for stake {stake} + fee {fee}", had - balance(cal, who.id()).await?);
    Ok(fee)
}

#[tokio::test]
#[ignore = "uses NEAR testnet and the faucet; run with `npm run execute-testnet`"]
async fn execute_testnet() -> Result<()> {
    let worker = testnet().await?;
    // 1,000,000 CAL: 1% to the launch holder, 1% in the launch hoard, 98% in the starter bucket.
    let supply = 10_000 * C;
    let hoard = 10_000 * C;
    let bucket = 980_000 * C;
    let launched = supply + hoard + bucket;
    let cfg = launch_config(1_000 * C, HUNT_BLOCKS, STEP_BLOCKS);
    let l = launch_testnet(&worker, "calydon-v11", &["alice", "bob", "carol"], cfg.clone(), supply, hoard, bucket, ".credentials/testnet").await?;
    let (cal, [alice, bob, carol]) = (&l.cal, [&l.hunters[0], &l.hunters[1], &l.hunters[2]]);
    println!("root {} (launch holder), contract {} (no access key)", l.root.id(), cal.id());
    let mut log = Log(vec![]);
    for (step, r) in &l.steps {
        log.tx(step, r)?;
    }
    let ids = [l.root.id(), alice.id(), bob.id(), carol.id()];
    let resolver = &l.root;

    // Hunt 0, HP 1000: alice 400 into an empty bar, bob 300, carol 400 past the HP -> slain.
    let h0 = view(cal, "current_hunt", json!({})).await?;
    for (who, stake) in [(alice, 400), (bob, 300), (carol, 400)] {
        commit_checked(&mut log, cal, who, stake * C, 0).await?;
    }
    ensure!(view(cal, "current_hunt", json!({})).await?["first_spear"] == json!(alice.id()));
    let weights: Vec<u128> = futures::future::try_join_all([alice, bob, carol].map(|h| async move {
        num(&view(cal, "entry", json!({"hunt": 0, "account_id": h.id()})).await?["weight"])
    })).await?;
    wait_for(&worker, h0["deadline"].as_u64().unwrap()).await?;
    ensure!(log.tx("hunt 0: resolve", &resolve(cal, resolver, 0).await?)? == "Slain");
    let settled = view(cal, "hunt", json!({"id": 0})).await?;
    let (pot, total) = (num(&settled["pot"])?, num(&settled["weight"])?);
    for ((who, stake), w) in [(alice, 400), (bob, 300), (carol, 400)].into_iter().zip(&weights) {
        let name = who.id().as_str().split('.').next().unwrap();
        let paid = num(&log.tx(&format!("hunt 0: {name} claim"), &claim(cal, who, 0).await?)?)?;
        ensure!(paid == stake * C + share(pot, *w, total), "{name} slain payout {paid}");
    }
    conservation(cal, &ids, launched).await?;

    // Hunt 1, HP 1100: alice alone commits 200 -> escaped; 20 CAL burn, her fee stays in the hoard.
    let h1 = view(cal, "current_hunt", json!({})).await?;
    ensure!(num(&h1["hp"])? == 1_100 * C);
    commit_checked(&mut log, cal, alice, 200 * C, 1).await?;
    // A plain transfer to the contract: outside hoard + locked + unclaimed + starter bucket until resolve.
    log.tx("hunt 1: holder ft_transfer 50 CAL to the contract (a plain transfer)", &send(cal, &l.root, cal.id(), 50 * C).await?)?;
    let t = view(cal, "totals", json!({})).await?;
    ensure!(num(&t["held"])? - parts(&t)? == 50 * C, "surplus before resolve");
    let hoard1 = num(&t["hoard"])?;
    wait_for(&worker, h1["deadline"].as_u64().unwrap()).await?;
    let r1 = resolve(cal, resolver, 1).await?;
    ensure!(log.tx("hunt 1: resolve (sweeps the 50 CAL into the hoard)", &r1)? == "Escaped");
    ensure!(events(&r1).iter().any(|e| e["event"] == "sweep" && e["data"][0]["amount"] == json!((50 * C).to_string())));
    ensure!(num(&view(cal, "totals", json!({})).await?["hoard"])? == hoard1 + 50 * C);
    let paid = num(&log.tx("hunt 1: alice claim", &claim(cal, alice, 1).await?)?)?;
    ensure!(paid == 180 * C, "escaped payout {paid}");
    let t = conservation(cal, &ids, launched).await?;
    ensure!(num(&t["burned"])? == 20 * C);

    // Hunt 2, HP 770: bob 500, carol 300 -> slain; the pot includes hunt 1's fee.
    let h2 = view(cal, "current_hunt", json!({})).await?;
    ensure!(num(&h2["hp"])? == 770 * C);
    for (who, stake) in [(bob, 500), (carol, 300)] {
        commit_checked(&mut log, cal, who, stake * C, 2).await?;
    }
    wait_for(&worker, h2["deadline"].as_u64().unwrap()).await?;
    ensure!(log.tx("hunt 2: resolve", &resolve(cal, resolver, 2).await?)? == "Slain");
    for who in [bob, carol] {
        let name = who.id().as_str().split('.').next().unwrap();
        log.tx(&format!("hunt 2: {name} claim"), &claim(cal, who, 2).await?)?;
    }
    let totals = conservation(cal, &ids, launched).await?;
    ensure!(cal.view_access_keys().await?.is_empty(), "the contract account has an access key");

    let wasm_sha = std::process::Command::new("shasum").args(["-a", "256", "res/calydon.wasm"]).output()?;
    let record = json!({
        "network": "testnet",
        "recorded_at_unix": std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)?.as_secs(),
        "contract": cal.id(), "holder": l.root.id(), "hunters": [alice.id(), bob.id(), carol.id()],
        "wasm_sha256": String::from_utf8_lossy(&wasm_sha.stdout).split_whitespace().next(),
        "launch": {"holder_supply": supply.to_string(), "hoard": hoard.to_string(), "starter_bucket": bucket.to_string(), "config": cfg},
        "steps": log.0, "end_totals": totals,
        "end_hunt": view(cal, "current_hunt", json!({})).await?,
    });
    std::fs::create_dir_all("testnet")?;
    std::fs::write("testnet/run.json", serde_json::to_string_pretty(&record)? + "\n")?;
    println!("Calydon testnet: no access key; starters taken; hunts 0 slain, 1 escaped (50 CAL swept), 2 slain; every fee and payout as computed; I1 held. Record: testnet/run.json");
    Ok(())
}

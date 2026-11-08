//! Reproduction, on the v1.1 wasm, of the round-2 screener's self-transfer finding: `ft_transfer_call`
//! to the contract itself credits the contract at once and refunds two receipts later; a `resolve`
//! in between sweeps the transient CAL into the hoard, the refund then leaves the contract holding
//! less than hoard + locked + unclaimed + starter bucket, and the next `resolve` halts on the
//! subtraction. Not part of `npm run verify` (it needs the old wasm):
//!   git show v1.1:res/calydon.wasm > /tmp/calydon-v1.1.wasm
//!   CALYDON_V11_WASM=/tmp/calydon-v1.1.wasm cargo test --test repro_v11 -- --ignored --nocapture
mod common;

use anyhow::{Context, Result, ensure};
use common::*;
use near_workspaces::types::NearToken;
use serde_json::json;

const C: u128 = 1_000_000_000_000_000_000;

#[tokio::test]
#[ignore]
async fn v11_self_transfer_halts_resolve() -> Result<()> {
    let code = std::fs::read(std::env::var("CALYDON_V11_WASM").context("set CALYDON_V11_WASM")?)?;
    let worker = near_workspaces::sandbox().await?;
    let root = worker.root_account()?;
    let big = NearToken::from_near(20);
    let holder = subaccount(&root, "holder", big).await?;
    let alice = subaccount(&root, "alice", big).await?;
    let carol = subaccount(&root, "carol", big).await?;
    let (cal, _) = deploy_code(&worker, &root, "cal", holder.id(), 1_000_000 * C, 1_000 * C, 0,
        launch_config(1_000 * C, 40, 10), &code).await?;
    ensure!(register(&cal, &alice).await?.is_success());
    ensure!(send(&cal, &holder, alice.id(), 10_000 * C).await?.is_success());
    let h0 = view(&cal, "current_hunt", json!({})).await?;
    pass_deadline(&worker, h0["deadline"].as_u64().unwrap()).await?;

    // Alice: ft_transfer_call of 500 CAL to the contract itself; carol: resolve, sent `delay` blocks
    // later. Tried with delays 0..=3 until resolve lands between the transfer and its refund.
    let x = 500 * C;
    let mut hunt = 0u64;
    let mut reproduced = false;
    for delay in 0..=3u64 {
        let t1 = alice.call(cal.id(), "ft_transfer_call")
            .args_json(json!({"receiver_id": cal.id(), "amount": x.to_string(), "msg": ""}))
            .deposit(ONE).gas(near_workspaces::types::Gas::from_tgas(100)).transact_async().await?;
        let start = height(&worker).await?;
        while height(&worker).await? < start + delay {
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
        let t2 = carol.call(cal.id(), "resolve").args_json(json!({"hunt": hunt}))
            .deposit(RECORD_DEPOSIT).transact_async().await?;
        let (r1, r2) = (t1.await?, t2.await?);
        let mut blocks = vec![];
        for o in r1.outcomes() {
            blocks.push(worker.view_block().block_hash(o.block_hash).await?.height());
        }
        let mut rb = vec![];
        for o in r2.outcomes() {
            rb.push(worker.view_block().block_hash(o.block_hash).await?.height());
        }
        let sweep: Vec<_> = events(&r2).into_iter().filter(|e| e["event"] == "sweep").map(|e| e["data"][0]["amount"].clone()).collect();
        let t = view(&cal, "totals", json!({})).await?;
        let (held, parts) = (num(&t["held"])?, parts(&t)?);
        println!("delay {delay}: ft_transfer_call outcomes at blocks {blocks:?} (refund {}), resolve({hunt}) at {rb:?} ok={} sweep {sweep:?}; held {held} vs parts {parts}",
            if r1.failures().is_empty() { "none" } else { "after ft_on_transfer failed" }, r2.is_success());
        if r2.is_success() {
            hunt += 1;
        }
        if !sweep.is_empty() && held < parts {
            reproduced = true;
            break;
        }
        let h = view(&cal, "current_hunt", json!({})).await?;
        pass_deadline(&worker, h["deadline"].as_u64().unwrap()).await?;
    }
    ensure!(reproduced, "not reproduced with delays 0..=3");

    let h1 = view(&cal, "current_hunt", json!({})).await?;
    pass_deadline(&worker, h1["deadline"].as_u64().unwrap()).await?;
    let r3 = resolve(&cal, &carol, hunt).await?;
    println!("next resolve({hunt}): success={} panic {:?}", r3.is_success(), failures(&r3));
    ensure!(r3.is_failure(), "next resolve did not fail");
    let c = commit(&cal, &alice, 10 * C).await?;
    println!("commit into hunt {hunt} after its deadline: {:?} (no resolve can open the next hunt)", failures(&c));
    println!("REPRODUCED");
    Ok(())
}

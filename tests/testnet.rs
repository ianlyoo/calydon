//! `npm run execute-testnet`: one slain hunt and one escaped hunt on NEAR testnet, with links.
//! Creates a fresh faucet-funded account (random key, saved under the ignored .credentials/, never
//! printed) and writes the public record to testnet/run.json. Pattern adapted from Preavis tests/testnet.rs.
mod common;

use anyhow::{Context, Result, ensure};
use common::*;
use near_workspaces::result::ExecutionFinalResult;
use near_workspaces::types::{KeyType, NearToken, SecretKey};
use serde_json::{Value, json};

const C: u128 = 1_000_000_000_000_000_000;

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

async fn wait_for(worker: &near_workspaces::Worker<near_workspaces::network::Testnet>, deadline: u64) -> Result<()> {
    while height(worker).await? < deadline {
        tokio::time::sleep(std::time::Duration::from_secs(5)).await;
    }
    Ok(())
}

#[tokio::test]
#[ignore = "uses NEAR testnet and the faucet; run with `npm run execute-testnet`"]
async fn execute_testnet() -> Result<()> {
    let rpc = std::env::var("CALYDON_TESTNET_RPC").unwrap_or_else(|_| "https://test.rpc.fastnear.com".into());
    let worker = near_workspaces::testnet().rpc_addr(&rpc).await?;
    let id = format!("calydon-{}", std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)?.as_secs());
    let owner = worker
        .create_root_account_subaccount(id.parse()?, SecretKey::from_random(KeyType::ED25519))
        .await
        .context("faucet account creation failed")?
        .into_result()?;
    let keys = format!(".credentials/testnet/{}", owner.id());
    std::fs::create_dir_all(&keys)?;
    owner.store_credentials(&keys).await?;
    println!("owner {} (credentials under {keys}/, not printed)", owner.id());

    let small = NearToken::from_millinear(300);
    let alice = subaccount(&owner, "alice", small).await?;
    let bob = subaccount(&owner, "bob", small).await?;
    alice.store_credentials(&keys).await?;
    bob.store_credentials(&keys).await?;
    let supply = 1_000_000 * C;
    let launched = supply + 1_000 * C;
    // 150 blocks is about 1.5-2 minutes on testnet.
    let cfg = config("HpCap", 1_000 * C, 150, 50);
    let (cal, init) = deploy(&owner, "cal", owner.id(), supply, 1_000 * C, cfg.clone()).await?;
    let wasm_sha = std::process::Command::new("shasum").args(["-a", "256", "res/calydon.wasm"]).output()?;
    let mut log = Log(vec![]);
    log.tx("init", &init)?;
    for h in [&alice, &bob] {
        log.tx(&format!("storage_deposit {}", h.id()), &register(&cal, h).await?)?;
        log.tx(&format!("ft_transfer 5000 CAL -> {}", h.id()), &send(&cal, &owner, h.id(), 5_000 * C).await?)?;
    }
    let ids = [owner.id(), alice.id(), bob.id()];

    // Hunt 0, HP 1000 CAL: alice 600 + bob offers 500 (400 locked under the cap) -> slain.
    let h0 = view(&cal, "current_hunt", json!({})).await?;
    log.tx("hunt 0: alice commit 600", &commit(&cal, &alice, 600 * C).await?)?;
    log.tx("hunt 0: bob commit 500 (cap keeps 400)", &commit(&cal, &bob, 500 * C).await?)?;
    wait_for(&worker, h0["deadline"].as_u64().unwrap()).await?;
    let out = log.tx("hunt 0: resolve", &resolve(&cal, &owner, 0).await?)?;
    ensure!(out == "Slain");
    let pa = num(&log.tx("hunt 0: alice claim", &claim(&cal, &alice, 0).await?)?)?;
    let pb = num(&log.tx("hunt 0: bob claim", &claim(&cal, &bob, 0).await?)?)?;
    ensure!(pa == 600 * C + 606 * C && pb == 400 * C + 404 * C, "slain payouts {pa} {pb}");
    conservation(&cal, &ids, launched).await?;

    // Hunt 1, HP 1250 CAL: alice alone commits 300 -> escaped, 30 burn.
    let h1 = view(&cal, "current_hunt", json!({})).await?;
    ensure!(num(&h1["hp"])? == 1_250 * C);
    log.tx("hunt 1: alice commit 300", &commit(&cal, &alice, 300 * C).await?)?;
    wait_for(&worker, h1["deadline"].as_u64().unwrap()).await?;
    let out = log.tx("hunt 1: resolve", &resolve(&cal, &bob, 1).await?)?;
    ensure!(out == "Escaped");
    let pa = num(&log.tx("hunt 1: alice claim", &claim(&cal, &alice, 1).await?)?)?;
    ensure!(pa == 270 * C, "escaped payout {pa}");
    let totals = conservation(&cal, &ids, launched).await?;
    ensure!(num(&totals["burned"])? == 30 * C);

    let record = json!({
        "network": "testnet", "rpc": rpc,
        "recorded_at_unix": std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)?.as_secs(),
        "contract": cal.id(), "holder": owner.id(), "hunters": [alice.id(), bob.id()],
        "wasm_sha256": String::from_utf8_lossy(&wasm_sha.stdout).split_whitespace().next(),
        "config": cfg, "steps": log.0, "end_totals": totals,
        "end_hunt": view(&cal, "current_hunt", json!({})).await?,
    });
    std::fs::create_dir_all("testnet")?;
    std::fs::write("testnet/run.json", serde_json::to_string_pretty(&record)? + "\n")?;
    println!("Calydon testnet: slain + escaped hunts settled, I1 held. Record: testnet/run.json");
    Ok(())
}

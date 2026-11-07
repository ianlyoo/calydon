//! Shared sandbox/testnet helpers for the tests, the simulation and the testnet run.
//! Pattern adapted from Preavis tests/common (random keys for every account, panic extraction).
#![allow(dead_code)] // each binary uses a different subset

use anyhow::{Context, Result, ensure};
use near_workspaces::result::ExecutionFinalResult;
use near_workspaces::types::{KeyType, NearToken, SecretKey};
use near_workspaces::{Account, AccountId, Contract, Network, Worker};
use serde_json::{Value, json};

pub const ONE: NearToken = NearToken::from_yoctonear(1);

/// The committed `res/calydon.wasm`: the bytes the tests, the simulation and testnet run.
pub fn wasm() -> Result<Vec<u8>> {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/res/calydon.wasm");
    std::fs::read(path).context("res/calydon.wasm missing; run scripts/build-wasm.sh")
}

pub fn config(rule: &str, first_hp: u128, hunt_blocks: u64, step_blocks: u64) -> Value {
    json!({
        "name": "Calydon", "symbol": "CAL", "decimals": 18, "rule": rule,
        "first_hp": first_hp.to_string(), "hunt_blocks": hunt_blocks, "step_blocks": step_blocks,
        "tribute_bps": 100, "burn_bps": 1_000, "hp_up_bps": 12_500, "hp_down_bps": 8_000,
    })
}

/// Always with a fresh random key: near-workspaces' default key derives from a public seed.
pub async fn subaccount(parent: &Account, name: &str, balance: NearToken) -> Result<Account> {
    Ok(parent
        .create_subaccount(name)
        .keys(SecretKey::from_random(KeyType::ED25519))
        .initial_balance(balance)
        .transact()
        .await?
        .into_result()?)
}

pub async fn deploy(
    parent: &Account,
    name: &str,
    holder: &AccountId,
    supply: u128,
    hoard: u128,
    config: Value,
) -> Result<(Contract, ExecutionFinalResult)> {
    let account = subaccount(parent, name, NearToken::from_near(5)).await?;
    let token = account.deploy(&wasm()?).await?.into_result()?;
    let init = token
        .call("new")
        .args_json(json!({"holder": holder, "supply": supply.to_string(), "hoard": hoard.to_string(), "config": config}))
        .max_gas()
        .transact()
        .await?;
    ensure!(init.is_success(), "init failed: {init:?}");
    Ok((token, init))
}

pub async fn register(token: &Contract, account: &Account) -> Result<ExecutionFinalResult> {
    Ok(account
        .call(token.id(), "storage_deposit")
        .args_json(json!({}))
        .deposit(NearToken::from_millinear(2))
        .transact()
        .await?)
}

pub async fn send(
    token: &Contract,
    from: &Account,
    to: &AccountId,
    amount: u128,
) -> Result<ExecutionFinalResult> {
    Ok(from
        .call(token.id(), "ft_transfer")
        .args_json(json!({"receiver_id": to, "amount": amount.to_string()}))
        .deposit(ONE)
        .transact()
        .await?)
}

pub async fn commit(token: &Contract, hunter: &Account, amount: u128) -> Result<ExecutionFinalResult> {
    Ok(hunter
        .call(token.id(), "commit")
        .args_json(json!({"amount": amount.to_string()}))
        .deposit(ONE)
        .transact()
        .await?)
}

pub async fn resolve(token: &Contract, caller: &Account, hunt: u64) -> Result<ExecutionFinalResult> {
    Ok(caller
        .call(token.id(), "resolve")
        .args_json(json!({"hunt": hunt}))
        .transact()
        .await?)
}

pub async fn claim(token: &Contract, hunter: &Account, hunt: u64) -> Result<ExecutionFinalResult> {
    Ok(hunter
        .call(token.id(), "claim")
        .args_json(json!({"hunt": hunt}))
        .transact()
        .await?)
}

pub async fn view(token: &Contract, method: &str, args: Value) -> Result<Value> {
    Ok(token.view(method).args_json(args).await?.json()?)
}

pub async fn balance(token: &Contract, account: &AccountId) -> Result<u128> {
    num(&view(token, "ft_balance_of", json!({"account_id": account})).await?)
}

pub fn num(v: &Value) -> Result<u128> {
    v.as_str()
        .with_context(|| format!("not a U128: {v}"))?
        .parse()
        .context("U128")
}

/// The U128 a successful call returned.
pub fn returned(r: &ExecutionFinalResult) -> Result<u128> {
    ensure!(r.is_success(), "call failed: {:?}", failures(r));
    num(&r.clone().json::<Value>()?)
}

/// Each failed receipt's panic message (the text after "Smart contract panicked: ").
pub fn failures(r: &ExecutionFinalResult) -> Vec<String> {
    r.failures()
        .iter()
        .map(|f| {
            let text = format!("{f:?}");
            match text.split_once("panicked: ") {
                Some((_, rest)) => rest.split(['"', '\\']).next().unwrap_or("").to_string(),
                None => text,
            }
        })
        .collect()
}

/// The single panic message of a call that must fail.
pub fn panic_of(r: &ExecutionFinalResult) -> String {
    assert!(r.is_failure(), "expected a panic, got success");
    failures(r).join("; ")
}

/// Every NEP-297 event the transaction logged, in order.
pub fn events(r: &ExecutionFinalResult) -> Vec<Value> {
    r.logs()
        .iter()
        .filter_map(|l| l.strip_prefix("EVENT_JSON:"))
        .filter_map(|l| serde_json::from_str(l).ok())
        .collect()
}

pub fn event_names(r: &ExecutionFinalResult) -> Vec<String> {
    events(r)
        .iter()
        .map(|e| format!("{}:{}", e["standard"].as_str().unwrap_or(""), e["event"].as_str().unwrap_or("")))
        .collect()
}

/// I1: every holder's balance plus the contract's own (= hoard + locked + unclaimed) is the
/// supply, and the supply is the launch amount minus escape burns. Returns the totals view.
pub async fn conservation(token: &Contract, holders: &[&AccountId], launched: u128) -> Result<Value> {
    let t = view(token, "totals", json!({})).await?;
    let (supply, held) = (num(&t["supply"])?, num(&t["held"])?);
    let parts = num(&t["hoard"])? + num(&t["locked"])? + num(&t["unclaimed"])?;
    ensure!(held == parts, "I1: contract holds {held} but hoard+locked+unclaimed = {parts}");
    let mut sum = held;
    for h in holders {
        sum += balance(token, h).await?;
    }
    ensure!(sum == supply, "I1: balances {sum} != supply {supply}");
    ensure!(supply + num(&t["burned"])? == launched, "I1: supply {supply} + burned != launched {launched}");
    Ok(t)
}

pub async fn height<N: Network + 'static>(worker: &Worker<N>) -> Result<u64> {
    Ok(worker.view_block().await?.height())
}

/// Sandbox only: advance past `deadline`.
pub async fn pass_deadline(worker: &Worker<near_workspaces::network::Sandbox>, deadline: u64) -> Result<()> {
    let now = height(worker).await?;
    if now < deadline {
        worker.fast_forward(deadline - now).await?;
    }
    Ok(())
}

/// Names of the functions the wasm exports, read from its export section.
pub fn exports(wasm: &[u8]) -> Vec<String> {
    fn leb(b: &[u8], i: &mut usize) -> usize {
        let (mut v, mut shift) = (0usize, 0);
        loop {
            let byte = b[*i];
            *i += 1;
            v |= ((byte & 0x7f) as usize) << shift;
            if byte & 0x80 == 0 {
                return v;
            }
            shift += 7;
        }
    }
    let mut i = 8;
    let mut out = vec![];
    while i < wasm.len() {
        let id = wasm[i];
        i += 1;
        let size = leb(wasm, &mut i);
        let end = i + size;
        if id == 7 {
            let n = leb(wasm, &mut i);
            for _ in 0..n {
                let len = leb(wasm, &mut i);
                let name = String::from_utf8_lossy(&wasm[i..i + len]).to_string();
                i += len;
                let kind = wasm[i];
                i += 1;
                leb(wasm, &mut i);
                if kind == 0 {
                    out.push(name);
                }
            }
        }
        i = end;
    }
    out.sort();
    out
}

//! `npm run verify`: scripted hunts on a local sandbox against the committed res/calydon.wasm,
//! with I1 (conservation) checked after every state change and I2 (exactly-once claim) by replay.
mod common;

use anyhow::{Result, ensure};
use common::*;
use near_workspaces::types::NearToken;
use serde_json::json;

const C: u128 = 1_000_000_000_000_000_000; // 1 CAL (18 decimals)

fn held(n: &mut u32, claim: &str, observed: &str) {
    *n += 1;
    println!("HELD {n:>2}  {claim}\n         observed: {observed}");
}

#[tokio::test]
async fn rule_a_hunts() -> Result<()> {
    let worker = near_workspaces::sandbox().await?;
    let root = worker.root_account()?;
    let big = NearToken::from_near(20);
    let holder = subaccount(&root, "holder", big).await?;
    let [alice, bob, carol, dave] = [
        subaccount(&root, "alice", big).await?,
        subaccount(&root, "bob", big).await?,
        subaccount(&root, "carol", big).await?,
        subaccount(&root, "dave", big).await?,
    ];
    // Launch: 1,000,000 CAL to the holder, 1,000 CAL + 1 yocto in the hoard (the yocto makes rounding visible).
    let supply = 1_000_000 * C;
    let launched = supply + 1_000 * C + 1;
    let cfg = config("HpCap", 1_000 * C, 200, 50);
    let (cal, init) = deploy(&worker, &root, "cal", holder.id(), supply, 1_000 * C + 1, 0, cfg.clone()).await?;
    let ids = [holder.id(), alice.id(), bob.id(), carol.id(), dave.id()];
    let mut n = 0;

    let read = view(&cal, "config", json!({})).await?;
    ensure!(read == cfg, "config view {read} != init {cfg}");
    ensure!(event_names(&init) == ["nep141:ft_mint", "calydon:open"], "init events {:?}", event_names(&init));
    let keys = cal.view_access_keys().await?;
    ensure!(keys.is_empty(), "contract account has {} keys", keys.len());
    let exported = exports(&wasm()?);
    let expected = [
        "claim", "commit", "config", "contract_source_metadata", "current_hunt", "entry", "ft_balance_of", "ft_metadata",
        "ft_resolve_transfer", "ft_total_supply", "ft_transfer", "ft_transfer_call", "hunt", "new",
        "resolve", "starter", "starter_taken", "storage_balance_bounds", "storage_balance_of", "storage_deposit",
        "storage_unregister", "storage_withdraw", "totals",
    ];
    ensure!(exported == expected, "exports {exported:?}");
    held(&mut n, "every parameter is fixed at init and readable; the wasm exports no setter, admin or pause method (I4; contract_source_metadata is the sdk's read-only NEP-330 view); create + deploy + new is one transaction and the contract account has no access key",
        &format!("config() == init args; 0 access keys; {} exports: {}", exported.len(), exported.join(", ")));

    for h in [&alice, &bob, &carol] {
        ensure!(register(&cal, h).await?.is_success());
        ensure!(send(&cal, &holder, h.id(), 10_000 * C).await?.is_success());
        ensure!(balance(&cal, h.id()).await? == 10_000 * C);
    }
    let unregistered = send(&cal, &holder, dave.id(), C).await?;
    let unreg = storage_unregister(&cal, &alice).await?;
    held(&mut n, "NEP-141/NEP-145: storage_deposit registers; ft_transfer moves the exact amount (no tribute on transfers); unregistering is disabled",
        &format!("3 x 10000 CAL received exactly; to unregistered dave: panic {:?}; storage_unregister: panic {}", panic_of(&unregistered), panic_of(&unreg)));
    conservation(&cal, &ids, launched).await?;

    // Hunt 0: HP 1000 CAL.
    let early_claim = claim(&cal, &alice, 0).await?;
    let early_resolve = resolve(&cal, &carol, 0).await?;
    held(&mut n, "claim before resolve and resolve before the deadline are refused",
        &format!("claim(0): panic {}; resolve(0): panic {}", panic_of(&early_claim), panic_of(&early_resolve)));

    let a = commit(&cal, &alice, 600 * C).await?;
    ensure!(returned(&a)? == 600 * C && balance(&cal, alice.id()).await? == 10_000 * C - 606 * C);
    let names = event_names(&a);
    ensure!(names == ["nep141:ft_transfer", "calydon:commit"], "events {names:?}");
    let b = commit(&cal, &bob, 500 * C).await?;
    ensure!(returned(&b)? == 400 * C && balance(&cal, bob.id()).await? == 10_000 * C - 404 * C);
    let full = commit(&cal, &carol, C).await?;
    held(&mut n, "commit locks the stake and charges the 1% tribute on top into the hoard; under rule A stake past the HP is not taken",
        &format!("alice 600 locked, paid 606; bob offered 500, locked 400, paid 404; carol at HP: panic {}; events {names:?}", panic_of(&full)));
    let t = conservation(&cal, &ids, launched).await?;
    ensure!(num(&t["locked"])? == 1_000 * C && num(&t["hoard"])? == 1_010 * C + 1);

    let deadline = view(&cal, "current_hunt", json!({})).await?["deadline"].as_u64().unwrap();
    pass_deadline(&worker, deadline).await?;
    let late = commit(&cal, &carol, C).await?;
    let r = resolve(&cal, &carol, 0).await?;
    ensure!(r.json::<String>()? == "Slain");
    let again = resolve(&cal, &carol, 0).await?;
    let future = resolve(&cal, &carol, 5).await?;
    let next = view(&cal, "current_hunt", json!({})).await?;
    ensure!(num(&next["hp"])? == 1_250 * C && next["id"] == 1);
    held(&mut n, "at the deadline anyone resolves once: committed 1000 >= HP 1000, slain; next HP x1.25",
        &format!("commit after deadline: panic {}; resolve(0) by carol (not a hunter) -> Slain; again: panic {}; resolve(5): panic {}; hunt 1 HP {} CAL",
            panic_of(&late), panic_of(&again), panic_of(&future), num(&next["hp"])? / C));
    conservation(&cal, &ids, launched).await?;

    // Pot = 1000 CAL + 1 yocto + 10 CAL tribute. Shares round down; the 1 yocto stays in the hoard.
    let pa = returned(&claim(&cal, &alice, 0).await?)?;
    let twice = claim(&cal, &alice, 0).await?;
    let stranger = claim(&cal, &carol, 0).await?;
    let t = conservation(&cal, &ids, launched).await?;
    ensure!(num(&t["hoard"])? == 0, "dust returns only after the last claim");
    let pb = returned(&claim(&cal, &bob, 0).await?)?;
    let t = conservation(&cal, &ids, launched).await?;
    ensure!(pa == 600 * C + 606 * C && pb == 400 * C + 404 * C, "payouts {pa} {pb}");
    ensure!(num(&t["hoard"])? == 1 && num(&t["unclaimed"])? == 0);
    held(&mut n, "slain: each hunter claims stake + pot x stake / committed, rounded down, exactly once; dust goes back to the hoard (I2)",
        &format!("alice {pa} = 600 + 606 CAL; second claim: panic {}; non-hunter: panic {}; bob {pb} = 400 + 404 CAL; hoard after last claim = 1 yocto, unclaimed 0",
            panic_of(&twice), panic_of(&stranger)));

    // Hunt 1: HP 1250 CAL, under-committed -> escaped. Carol's odd yocto amount shows the round-up.
    ensure!(returned(&commit(&cal, &alice, 300 * C).await?)? == 300 * C);
    ensure!(returned(&commit(&cal, &carol, 1_001).await?)? == 1_001);
    let t = conservation(&cal, &ids, launched).await?;
    let hoard_before = num(&t["hoard"])?;
    ensure!(hoard_before == 1 + 3 * C + 11, "carol's tribute ceil(10.01) = 11, hoard {hoard_before}");
    let supply_before = num(&t["supply"])?;
    pass_deadline(&worker, next["deadline"].as_u64().unwrap()).await?;
    ensure!(resolve(&cal, &bob, 1).await?.json::<String>()? == "Escaped");
    let h2 = view(&cal, "current_hunt", json!({})).await?;
    ensure!(num(&h2["hp"])? == 1_000 * C);
    let ca = claim(&cal, &alice, 1).await?;
    let names = event_names(&ca);
    ensure!(names == ["nep141:ft_transfer", "nep141:ft_burn", "calydon:claim"], "events {names:?}");
    let pa = returned(&ca)?;
    let pc = returned(&claim(&cal, &carol, 1).await?)?;
    let t = conservation(&cal, &ids, launched).await?;
    let burned = supply_before - num(&t["supply"])?;
    ensure!(pa == 270 * C && pc == 900 && burned == 30 * C + 101);
    ensure!(num(&t["hoard"])? == hoard_before, "hoard rolls over");
    held(&mut n, "escaped: each hunter claims stake - 10% (burn rounded up); the supply falls by exactly the burns; hoard rolls over; next HP x0.8",
        &format!("alice 270 CAL back; carol 1001 yocto -> 900 back, 101 burned (ceil 100.1); supply -{burned} yocto; hoard unchanged {hoard_before}; hunt 2 HP 1000 CAL; claim events {names:?}"));

    // Hunt 2: a tie at exactly the HP is a kill. Hunt 3: nobody commits; it still resolves.
    ensure!(returned(&commit(&cal, &alice, 1_000 * C).await?)? == 1_000 * C);
    pass_deadline(&worker, h2["deadline"].as_u64().unwrap()).await?;
    ensure!(resolve(&cal, &dave, 2).await?.json::<String>()? == "Slain");
    let pot = num(&view(&cal, "hunt", json!({"id": 2})).await?["pot"])?;
    let pa = returned(&claim(&cal, &alice, 2).await?)?;
    ensure!(pa == 1_000 * C + pot && pot == hoard_before + 10 * C);
    let h3 = view(&cal, "current_hunt", json!({})).await?;
    pass_deadline(&worker, h3["deadline"].as_u64().unwrap()).await?;
    ensure!(resolve(&cal, &dave, 3).await?.json::<String>()? == "Escaped");
    let h4 = view(&cal, "current_hunt", json!({})).await?;
    let t = conservation(&cal, &ids, launched).await?;
    ensure!(num(&h4["hp"])? == 1_000 * C && h4["id"] == 4 && num(&t["hoard"])? == 0);
    held(&mut n, "a tie at exactly HP is slain; an empty hunt resolves (escaped, nothing to burn) and the next opens (I5)",
        &format!("hunt 2: 1000 committed vs HP 1000 -> Slain, alice gets 1000 + whole pot {pot}; hunt 3: 0 hunters -> Escaped; hunt 4 HP {} CAL", num(&h4["hp"])? / C));
    held(&mut n, "I1 conservation held after every step: holder balances + hoard + locked + unclaimed = supply = launch - burns",
        &format!("final supply {} = launched {launched} - burned {}", num(&t["supply"])?, num(&t["burned"])?));
    println!("Calydon verify (rule A, arithmetic-test values: no fill fee, HP x1.25/x0.8): {n} claims held, 0 broken.");
    Ok(())
}

async fn storage_unregister(
    cal: &near_workspaces::Contract,
    who: &near_workspaces::Account,
) -> Result<near_workspaces::result::ExecutionFinalResult> {
    Ok(who
        .call(cal.id(), "storage_unregister")
        .args_json(json!({"force": true}))
        .deposit(ONE)
        .transact()
        .await?)
}

#[tokio::test]
async fn rule_b_and_splitting() -> Result<()> {
    let worker = near_workspaces::sandbox().await?;
    let root = worker.root_account()?;
    let big = NearToken::from_near(20);
    let holder = subaccount(&root, "holder", big).await?;
    let [alice, bob, carol, dave] = [
        subaccount(&root, "alice", big).await?,
        subaccount(&root, "bob", big).await?,
        subaccount(&root, "carol", big).await?,
        subaccount(&root, "dave", big).await?,
    ];
    let ids = [holder.id(), alice.id(), bob.id(), carol.id(), dave.id()];
    let mut n = 0;

    // Rule B: 300-block hunt, weight = stake x 100-block steps left (3, 2, 1). No cap.
    let supply = 1_000_000 * C;
    let (cal, _) = deploy(&worker, &root, "calb", holder.id(), supply, 1_000 * C, 0, config("EarlyWeight", 1_000 * C, 300, 100)).await?;
    let launched = supply + 1_000 * C;
    for h in [&alice, &bob] {
        ensure!(register(&cal, h).await?.is_success());
        ensure!(send(&cal, &holder, h.id(), 10_000 * C).await?.is_success());
    }
    let hunt = view(&cal, "current_hunt", json!({})).await?;
    let deadline = hunt["deadline"].as_u64().unwrap();
    ensure!(returned(&commit(&cal, &alice, 500 * C).await?)? == 500 * C);
    pass_deadline(&worker, deadline - 10).await?;
    ensure!(returned(&commit(&cal, &bob, 700 * C).await?)? == 700 * C, "rule B takes stake past HP");
    let wa = num(&view(&cal, "entry", json!({"hunt": 0, "account_id": alice.id()})).await?["weight"])?;
    let wb = num(&view(&cal, "entry", json!({"hunt": 0, "account_id": bob.id()})).await?["weight"])?;
    ensure!(wa == 3 * 500 * C && wb == 700 * C, "weights {wa} {wb}");
    conservation(&cal, &ids, launched).await?;
    pass_deadline(&worker, deadline).await?;
    ensure!(resolve(&cal, &bob, 0).await?.json::<String>()? == "Slain");
    let pot = 1_000 * C + 12 * C;
    let pa = returned(&claim(&cal, &alice, 0).await?)?;
    let pb = returned(&claim(&cal, &bob, 0).await?)?;
    ensure!(pa == 500 * C + calydon::share(pot, wa, wa + wb) && pb == 700 * C + calydon::share(pot, wb, wa + wb));
    let t = conservation(&cal, &ids, launched).await?;
    held(&mut n, "rule B: no cap; the hoard is shared by stake x steps left, so an earlier commit earns more per token",
        &format!("alice 500 at 3 steps left -> share {} CAL; bob 700 at 1 step left -> share {} CAL; dust back in hoard: {} yocto",
            (pa - 500 * C) / C, (pb - 700 * C) / C, num(&t["hoard"])?));

    // I3 on chain (rule A, raw units, HP 3, hoard 2 yocto): one account staking 2 vs two accounts staking 1.
    let mut nets = vec![];
    for (name, split) in [("whole", false), ("split", true)] {
        let (cal, _) = deploy(&worker, &root, name, holder.id(), 1_000, 2, 0, config("HpCap", 3, 200, 50)).await?;
        let mine: Vec<_> = if split { vec![&bob, &dave] } else { vec![&alice] };
        for h in mine.iter().chain([&&carol]) {
            ensure!(register(&cal, h).await?.is_success());
            ensure!(send(&cal, &holder, h.id(), 100).await?.is_success());
        }
        ensure!(returned(&commit(&cal, &carol, 1).await?)? == 1);
        for h in &mine {
            ensure!(returned(&commit(&cal, h, 2 / mine.len() as u128).await?)? == 2 / mine.len() as u128);
        }
        let d = view(&cal, "current_hunt", json!({})).await?["deadline"].as_u64().unwrap();
        pass_deadline(&worker, d).await?;
        ensure!(resolve(&cal, &carol, 0).await?.json::<String>()? == "Slain");
        let mut net = 0i128;
        for h in &mine {
            ensure!(claim(&cal, h, 0).await?.is_success());
            net += balance(&cal, h.id()).await? as i128 - 100;
        }
        nets.push(net);
    }
    ensure!(nets[1] <= nets[0], "split {} > whole {}", nets[1], nets[0]);
    held(&mut n, "I3 on chain: splitting a stake of 2 into 1 + 1 does not pay more (shares round down, tributes round up)",
        &format!("one account: net {:+} yocto; two accounts: net {:+} yocto in total (the 20,000-case property test is in src/tests.rs)", nets[0], nets[1]));
    println!("Calydon verify (rule B and splitting, arithmetic-test values): {n} claims held, 0 broken.");
    Ok(())
}

async fn call(cal: &near_workspaces::Contract, who: &near_workspaces::Account, method: &str, args: serde_json::Value, deposit: NearToken)
    -> Result<near_workspaces::result::ExecutionFinalResult> {
    Ok(who.call(cal.id(), method).args_json(args).deposit(deposit).transact().await?)
}

#[tokio::test]
async fn launch_values() -> Result<()> {
    let worker = near_workspaces::sandbox().await?;
    let root = worker.root_account()?;
    let big = NearToken::from_near(20);
    let holder = subaccount(&root, "holder", big).await?;
    let [alice, bob, carol, dave] = [
        subaccount(&root, "alice", big).await?,
        subaccount(&root, "bob", big).await?,
        subaccount(&root, "carol", big).await?,
        subaccount(&root, "dave", big).await?,
    ];
    let ids = [holder.id(), alice.id(), bob.id(), carol.id(), dave.id()];
    let mut n = 0;

    // The v1.0 launch values; 400-block hunts in 100-block steps (4, 3, 2, 1 steps left).
    let supply = 1_000_000 * C;
    let launched = supply + 1_000 * C;
    let cfg = launch_config(1_000 * C, 400, 100);
    let (cal, _) = deploy(&worker, &root, "cal", holder.id(), supply, 1_000 * C, 0, cfg.clone()).await?;
    ensure!(view(&cal, "config", json!({})).await? == cfg);
    for h in [&alice, &bob, &carol] {
        ensure!(register(&cal, h).await?.is_success());
        ensure!(send(&cal, &holder, h.id(), 10_000 * C).await?.is_success());
    }
    held(&mut n, "launch values are fixed at init and readable: rule B, tribute 1%, fill fee up to +10%, burn 10%, HP x1.1 / x0.7",
        &format!("config() == {cfg}"));

    // Edge cases at the boundary: under the minimum stake, no entry deposit, more than the balance. None changes state.
    let zero = commit(&cal, &alice, C - 1).await?;
    let no_yocto = call(&cal, &alice, "commit", json!({"amount": (10 * C).to_string()}), ONE).await?;
    let too_much = commit(&cal, &alice, 10_000 * C).await?;
    ensure!(balance(&cal, alice.id()).await? == 10_000 * C);
    let t = conservation(&cal, &ids, launched).await?;
    ensure!(num(&t["locked"])? == 0 && num(&t["hoard"])? == 1_000 * C);
    held(&mut n, "refused commits change nothing: below the 1-CAL minimum stake, a new entry without its storage deposit, stake + fee above the balance",
        &format!("commit(1 CAL - 1 yocto): panic {}; new entry with 1 yocto attached: panic {}; 10000 of 10000 (fee on top): panic {}; alice still 10000 CAL, locked 0",
            panic_of(&zero), panic_of(&no_yocto), panic_of(&too_much)));

    // Hunt 0, HP 1000. Alice commits 600 into an empty bar; bob 500 later, 400 to fill it and 100 past it.
    let h0 = view(&cal, "current_hunt", json!({})).await?;
    let deadline = h0["deadline"].as_u64().unwrap();
    ensure!(returned(&commit(&cal, &alice, 600 * C).await?)? == 600 * C);
    let fee_a = 10_000 * C - 600 * C - balance(&cal, alice.id()).await?;
    pass_deadline(&worker, deadline - 150).await?;
    ensure!(returned(&commit(&cal, &bob, 500 * C).await?)? == 500 * C, "rule B takes stake past HP");
    let fee_b = 10_000 * C - 500 * C - balance(&cal, bob.id()).await?;
    ensure!(fee_a == 24 * C && fee_a == calydon::fill_fee(0, 600 * C, 1_000 * C, 100, 1_000), "alice fee {fee_a}");
    ensure!(fee_b == 47 * C && fee_b == calydon::fill_fee(600 * C, 500 * C, 1_000 * C, 100, 1_000), "bob fee {fee_b}");
    let entry = |who: &near_workspaces::Account| view(&cal, "entry", json!({"hunt": 0, "account_id": who.id()}));
    let (wa, wb) = (num(&entry(&alice).await?["weight"])?, num(&entry(&bob).await?["weight"])?);
    ensure!(wa == 4 * 600 * C && wb == 2 * 500 * C, "weights {wa} {wb}");
    let t = conservation(&cal, &ids, launched).await?;
    ensure!(num(&t["hoard"])? == 1_071 * C);
    held(&mut n, "the entry fee rises with the bar's fill and goes into the hoard; rule B weights a commit by the steps left",
        &format!("alice 600 into an empty bar: fee 24 CAL (1% + 10% x 0.3 average fill); bob 500 at 600/1000: fee 47 CAL (400 at 1% + 10% x 0.8, 100 past HP at 11%); weights 600x4, 500x2; hoard 1000 + 71 = 1071 CAL"));

    pass_deadline(&worker, deadline).await?;
    ensure!(resolve(&cal, &dave, 0).await?.json::<String>()? == "Slain");
    let next = view(&cal, "current_hunt", json!({})).await?;
    ensure!(num(&next["hp"])? == 1_100 * C);
    let pot = 1_071 * C;
    let pa = returned(&claim(&cal, &alice, 0).await?)?;
    let pb = returned(&claim(&cal, &bob, 0).await?)?;
    let twice = claim(&cal, &bob, 0).await?;
    ensure!(pa == 600 * C + calydon::share(pot, wa, wa + wb) && pb == 500 * C + calydon::share(pot, wb, wa + wb));
    let t = conservation(&cal, &ids, launched).await?;
    let dust = num(&t["hoard"])?;
    ensure!(dust == pot - (pa - 600 * C) - (pb - 500 * C) && num(&t["unclaimed"])? == 0);
    held(&mut n, "slain under the launch values: shares of the 1071 CAL pot by weight, rounded down, once each; next HP x1.1",
        &format!("alice {} = 600 + {:.4} CAL; bob {} = 500 + {:.4} CAL; second claim: panic {}; dust {dust} yocto back in the hoard; hunt 1 HP 1100 CAL",
            pa, (pa - 600 * C) as f64 / C as f64, pb, (pb - 500 * C) as f64 / C as f64, panic_of(&twice)));

    // Hunt 1, HP 1100: carol alone commits 300 -> escaped. Her fee stays in the hoard; 10% of the stake burns.
    ensure!(returned(&commit(&cal, &carol, 300 * C).await?)? == 300 * C);
    let fee_c = calydon::fill_fee(0, 300 * C, 1_100 * C, 100, 1_000);
    ensure!(balance(&cal, carol.id()).await? == 10_000 * C - 300 * C - fee_c);
    let supply_before = num(&view(&cal, "totals", json!({})).await?["supply"])?;
    pass_deadline(&worker, next["deadline"].as_u64().unwrap()).await?;
    ensure!(resolve(&cal, &dave, 1).await?.json::<String>()? == "Escaped");
    let h2 = view(&cal, "current_hunt", json!({})).await?;
    ensure!(num(&h2["hp"])? == 770 * C);
    let pc = returned(&claim(&cal, &carol, 1).await?)?;
    let t = conservation(&cal, &ids, launched).await?;
    ensure!(pc == 270 * C && supply_before - num(&t["supply"])? == 30 * C);
    ensure!(num(&t["hoard"])? == dust + fee_c, "the escaped hunt's fees stay in the hoard");
    held(&mut n, "escaped under the launch values: stake - 10% back, the 10% burns, the entry fee stays in the hoard; next HP x0.7",
        &format!("carol 300 -> 270 back, supply -30 CAL; hoard = dust + carol's fee {:.6} CAL; hunt 2 HP 770 CAL", fee_c as f64 / C as f64));

    // I3 on chain with the fill fee: 600 from one account vs 300 + 300 from two, after carol's 400.
    let mut raw_launch = launch_config(1_000, 400, 100);
    raw_launch["min_stake"] = json!("1"); // raw units: the 1-CAL minimum would refuse these stakes
    let mut nets = vec![];
    for (name, split) in [("whole", false), ("split", true)] {
        let (cal, _) = deploy(&worker, &root, name, holder.id(), 1_000_000, 2, 0, raw_launch.clone()).await?;
        let mine: Vec<_> = if split { vec![&bob, &dave] } else { vec![&alice] };
        for h in mine.iter().chain([&&carol]) {
            ensure!(register(&cal, h).await?.is_success());
            ensure!(send(&cal, &holder, h.id(), 10_000).await?.is_success());
        }
        ensure!(returned(&commit(&cal, &carol, 400).await?)? == 400);
        for h in &mine {
            let part = 600 / mine.len() as u128;
            ensure!(returned(&commit(&cal, h, part).await?)? == part);
        }
        let d = view(&cal, "current_hunt", json!({})).await?["deadline"].as_u64().unwrap();
        pass_deadline(&worker, d).await?;
        ensure!(resolve(&cal, &carol, 0).await?.json::<String>()? == "Slain");
        let mut net = 0i128;
        for h in &mine {
            ensure!(claim(&cal, h, 0).await?.is_success());
            net += balance(&cal, h.id()).await? as i128 - 10_000;
        }
        nets.push(net);
    }
    ensure!(nets[1] <= nets[0], "split {} > whole {}", nets[1], nets[0]);
    held(&mut n, "I3 on chain with the fill fee: 600 from one account vs 300 + 300 from two accounts does not pay more",
        &format!("one account: net {:+} yocto; two accounts: net {:+} yocto (fill fees add up the same and round up; shares round down)", nets[0], nets[1]));
    held(&mut n, "I1 conservation held after every step under the launch values",
        &format!("final supply {} = launched {launched} - burned {}", num(&t["supply"])?, num(&t["burned"])?));
    println!("Calydon verify (launch values: R2 fee, rule B, HP x1.1/x0.7): {n} claims held, 0 broken.");
    Ok(())
}

/// The contract account's NEAR not tied up by its storage (balance - storage_usage x 10^19 yocto/byte).
async fn free_near(cal: &near_workspaces::Contract) -> Result<i128> {
    let a = cal.view_account().await?;
    Ok(a.balance.as_yoctonear() as i128 - a.storage_usage as i128 * 10_000_000_000_000_000_000)
}

#[tokio::test]
async fn v1_1_storage_starter_sweep() -> Result<()> {
    let worker = near_workspaces::sandbox().await?;
    let root = worker.root_account()?;
    let big = NearToken::from_near(20);
    let holder = subaccount(&root, "holder", big).await?;
    let [alice, bob, carol, dave] = [
        subaccount(&root, "alice", big).await?,
        subaccount(&root, "bob", big).await?,
        subaccount(&root, "carol", big).await?,
        subaccount(&root, "dave", big).await?,
    ];
    let ids = [holder.id(), alice.id(), bob.id(), carol.id(), dave.id()];
    let mut n = 0;

    // Launch values with HP 30 CAL; the holder gets 10,000 CAL, the hoard 1,000, the starter bucket 2,500.
    let (supply, hoard, bucket) = (10_000 * C, 1_000 * C, 2_500 * C);
    let launched = supply + hoard + bucket;
    let (cal, _) = deploy(&worker, &root, "cal", holder.id(), supply, hoard, bucket, launch_config(30 * C, 200, 50)).await?;

    // Starter: registered accounts only, once each, the record deposit attached, until the bucket is empty.
    let unregistered = starter(&cal, &dave).await?;
    for h in [&alice, &bob, &carol, &dave] {
        ensure!(register(&cal, h).await?.is_success());
    }
    let no_deposit = call(&cal, &alice, "starter", json!({}), NearToken::from_yoctonear(0)).await?;
    let got: Vec<u128> = vec![
        returned(&starter(&cal, &alice).await?)?,
        returned(&starter(&cal, &bob).await?)?,
        returned(&starter(&cal, &carol).await?)?,
    ];
    let again = starter(&cal, &alice).await?;
    let empty = starter(&cal, &dave).await?;
    ensure!(got == [1_000 * C, 1_000 * C, 500 * C], "starters {got:?}");
    ensure!(view(&cal, "starter_taken", json!({"account_id": alice.id()})).await? == true);
    ensure!(view(&cal, "starter_taken", json!({"account_id": dave.id()})).await? == false);
    let t = conservation(&cal, &ids, launched).await?;
    ensure!(num(&t["starter_left"])? == 0);
    held(&mut n, "starter(): a registered account takes 1,000 CAL once from the starter bucket; the last one gets what is left, then it is empty",
        &format!("unregistered: panic {}; no deposit: panic {}; alice, bob, carol got 1000, 1000, 500 CAL; alice again: panic {}; dave: panic {}",
            panic_of(&unregistered), panic_of(&no_deposit), panic_of(&again), panic_of(&empty)));

    // Storage: a commit that creates an entry brings ENTRY_DEPOSIT; the contract's free NEAR never falls.
    let free0 = free_near(&cal).await?;
    let usage0 = cal.view_account().await?.storage_usage;
    ensure!(returned(&commit(&cal, &alice, 10 * C).await?)? == 10 * C);
    let usage1 = cal.view_account().await?.storage_usage;
    ensure!(free_near(&cal).await? >= free0);
    ensure!(returned(&commit(&cal, &bob, 20 * C).await?)? == 20 * C);
    let usage2 = cal.view_account().await?.storage_usage;
    let wrong_more = commit_with(&cal, &alice, 5 * C, ENTRY_DEPOSIT).await?;
    let wrong_less = commit_with(&cal, &carol, 5 * C, ONE).await?;
    ensure!(returned(&commit_with(&cal, &alice, 5 * C, ONE).await?)? == 5 * C);
    let usage3 = cal.view_account().await?.storage_usage;
    let free1 = free_near(&cal).await?;
    ensure!(free1 >= free0, "free NEAR fell: {free0} -> {free1}");
    ensure!(usage3 == usage2, "a second commit to the same entry adds no storage");
    let entry_bytes = usage2 - usage1;
    ensure!(entry_bytes as u128 * 10u128.pow(19) <= ENTRY_DEPOSIT.as_yoctonear());
    held(&mut n, "storage: a commit that creates an entry must attach the entry deposit (5 mNEAR), a later commit to the same entry 1 yocto; the contract's free NEAR does not fall",
        &format!("alice's first commit added {} bytes (with the hunt's First Spear), bob's {entry_bytes} bytes = {} yoctoNEAR of storage, under the 5 mNEAR attached; alice's second commit with 5 mNEAR: panic {}; carol's first with 1 yocto: panic {}; alice's second with 1 yocto: +0 bytes; free NEAR {} -> {} yocto",
            usage1 - usage0, entry_bytes as u128 * 10u128.pow(19), panic_of(&wrong_more), panic_of(&wrong_less), free0, free1));

    // The v1.0 halt: entries paid from the contract's own NEAR. An instance whose entry deposit is
    // 1 yocto (less than an entry costs) refuses the commit instead of paying for it.
    let mut cheap = launch_config(30 * C, 200, 50);
    cheap["entry_deposit"] = json!("1");
    let (cal2, _) = deploy(&worker, &root, "cheap", holder.id(), supply, hoard, 0, cheap).await?;
    ensure!(register(&cal2, &alice).await?.is_success());
    ensure!(send(&cal2, &holder, alice.id(), 100 * C).await?.is_success());
    let free_before = free_near(&cal2).await?;
    let uncovered = commit_with(&cal2, &alice, 10 * C, NearToken::from_yoctonear(1)).await?;
    ensure!(view(&cal2, "entry", json!({"hunt": 0, "account_id": alice.id()})).await?.is_null());
    ensure!(free_near(&cal2).await? >= free_before);
    held(&mut n, "the halt scenario: storage growth that the attached NEAR does not cover is refused, so commits cannot spend the contract's own NEAR",
        &format!("instance with entry_deposit = 1 yocto: first commit panics {}; no entry stored; contract free NEAR not lower", panic_of(&uncovered)));

    // Donation: a plain ft_transfer to the contract is held outside the parts until resolve sweeps it.
    let h0 = view(&cal, "current_hunt", json!({})).await?;
    ensure!(h0["first_spear"] == json!(alice.id()), "first spear {}", h0["first_spear"]);
    ensure!(send(&cal, &holder, cal.id(), 77 * C).await?.is_success());
    let t = view(&cal, "totals", json!({})).await?;
    let surplus = num(&t["held"])? - parts(&t)?;
    ensure!(surplus == 77 * C, "surplus {surplus}");
    let hoard_before = num(&t["hoard"])?;
    pass_deadline(&worker, h0["deadline"].as_u64().unwrap()).await?;
    let no_record = call(&cal, &dave, "resolve", json!({"hunt": 0}), NearToken::from_yoctonear(0)).await?;
    let r = resolve(&cal, &dave, 0).await?;
    ensure!(r.clone().json::<String>()? == "Slain");
    let sweep: Vec<_> = events(&r).into_iter().filter(|e| e["event"] == "sweep").collect();
    ensure!(sweep.len() == 1 && num(&sweep[0]["data"][0]["amount"])? == 77 * C, "sweep events {sweep:?}");
    let pot = num(&view(&cal, "hunt", json!({"id": 0})).await?["pot"])?;
    ensure!(pot == hoard_before + 77 * C, "pot {pot}");
    let t = conservation(&cal, &ids, launched).await?;
    held(&mut n, "a plain transfer to the contract (77 CAL) shows as held - parts; resolve sweeps it into the hoard, so the identity holds again and it is paid out with the pot",
        &format!("before resolve: held - (hoard + locked + unclaimed + starter_left) = {} CAL; resolve without the record deposit: panic {}; resolve emits sweep 77 CAL; pot {} = hoard {} + 77 CAL; after: held == parts", surplus / C, panic_of(&no_record), pot, hoard_before));

    // Claims send the entry deposit back; the contract's free NEAR is back where it started or higher.
    let before = alice.view_account().await?.balance.as_yoctonear();
    let pa = returned(&claim(&cal, &alice, 0).await?)?;
    let gained = alice.view_account().await?.balance.as_yoctonear() as i128 - before as i128;
    ensure!(gained > 4 * 10i128.pow(21), "alice gained only {gained} yoctoNEAR");
    ensure!(claim(&cal, &bob, 0).await?.is_success());
    let free2 = free_near(&cal).await?;
    ensure!(free2 >= free0, "free NEAR after claims {free2} < {free0}");
    let t2 = conservation(&cal, &ids, launched).await?;
    ensure!(num(&t2["unclaimed"])? == 0);
    held(&mut n, "claim pays the CAL and sends the 5 mNEAR entry deposit back; First Spear of hunt 0 is alice, the first to commit",
        &format!("alice claimed {:.4} CAL and her NEAR rose by {:.6} (5 mNEAR minus gas); first_spear = {}; contract free NEAR {} -> {} yocto",
            pa as f64 / C as f64, gained as f64 / 1e24, h0["first_spear"], free0, free2));
    let _ = t;
    println!("Calydon verify (v1.1 storage, starter, sweep): {n} claims held, 0 broken.");
    Ok(())
}

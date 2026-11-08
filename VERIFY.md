# VERIFY: claim → command → expected output

Every claim Calydon makes, the command that checks it, and what you should see. Run the commands
from a clone of this repository (prerequisites: [README §1](README.md#1-check-the-contract-in-one-command--see-every-claim-held)).
Each `HELD` line is printed only after the test has asserted it on contract state, balances or
execution outcomes; if a claim were false, the command would exit non-zero.

## What is checked where

| Command | Needs | Checks |
|---|---|---|
| `npm run verify` | Rust (rustup), Node, network for the first download. No Docker, account or key. | Section A: the unit tests and 18 sandbox claims against the committed `res/calydon.wasm` |
| `npm run observe-testnet` | Node only, no key | Section B: the deployed code, 0 access keys, the launch values, the 23 recorded transactions and the totals on testnet |
| `npm run verify:reproducible` | Docker (pulls the pinned linux/amd64 Rust image) | Section C: source → `res/calydon.wasm` → testnet code hash are the same bytes |
| `npm run simulate`, `npm run sim-tables` | as `npm run verify`; about 13 minutes per run | Section D: MECHANIC.md's tables come from the committed wasm |
| `npm run hunt -- …` | Node, Rust, network; the testnet faucet (free) | Section E: commit / resolve / claim from your own faucet accounts |

## A. `npm run verify` (local sandbox, no account or key)

Expected final lines of the sandbox test file:

```text
Calydon verify (rule B and splitting, arithmetic-test values): 2 claims held, 0 broken.
test result: ok. 3 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out
```

Unit tests (`src/tests.rs`, `test result: ok. 4 passed`):

| Test | Claim |
|---|---|
| `splitting_never_pays_more` | I3 over 20,000 seeded cases (fill fee 0–50%, 2–8 parts landing one after another in the same step, slain or escaped, small and 27-digit amounts): the parts' total net result is never above the whole stake's, and rounding makes it strictly lower in some cases |
| `fill_fees` | hand-computed fees: 600 into an empty 1000 bar pays 24; 400 at 600/1000 pays 36; 700 past the HP pays 77 (11%); the fee is rounded up once |
| `m2_counterexample_is_the_safe_direction` | hoard 2, total stake 3: one account staking 2 gets 1; two accounts staking 1 each get 0 in total (splitting gets less, not more) |
| `rounding_helpers` | `ceil_bps` rounds up and `floor_bps` rounds down, including at 0 and near `u128::MAX` |

### `launch_values`: exactly the launch values (rule B, 1% + up to 10% fill fee, burn 10%, HP ×1.1 / ×0.7)

| # | Claim | Expected `observed:` line |
|---:|---|---|
| 1 | launch values are fixed at init and readable | `config() == {"name":"Calydon","symbol":"CAL","decimals":18,"rule":"EarlyWeight","first_hp":"1000000000000000000000","hunt_blocks":400,"step_blocks":100,"tribute_bps":100,"fill_fee_bps":1000,"burn_bps":1000,"hp_up_bps":11000,"hp_down_bps":7000}` |
| 2 | refused commits change nothing: zero stake, missing 1 yocto, stake + fee above the balance | `commit(0): panic NOTHING_TO_LOCK; no deposit: panic Requires attached deposit of exactly 1 yoctoNEAR; 10000 of 10000 (fee on top): panic The account doesn't have enough balance; alice still 10000 CAL, locked 0` |
| 3 | the entry fee rises with the bar's fill and goes into the hoard; rule B weights a commit by the steps left | `alice 600 into an empty bar: fee 24 CAL (1% + 10% x 0.3 average fill); bob 500 at 600/1000: fee 47 CAL (400 at 1% + 10% x 0.8, 100 past HP at 11%); weights 600x4, 500x2; hoard 1000 + 71 = 1071 CAL` |
| 4 | slain: shares of the pot by weight, rounded down, once each; next HP ×1.1 | `alice 1356000000000000000000 = 600 + 756.0000 CAL; bob 815000000000000000000 = 500 + 315.0000 CAL; second claim: panic NOTHING_TO_CLAIM; dust 0 yocto back in the hoard; hunt 1 HP 1100 CAL` |
| 5 | escaped: stake − 10% back, the 10% burns, the entry fee stays in the hoard; next HP ×0.7 | `carol 300 -> 270 back, supply -30 CAL; hoard = dust + carol's fee 7.090909 CAL; hunt 2 HP 770 CAL` |
| 6 | I3 on chain with the fill fee: 600 from one account vs 300 + 300 from two accounts does not pay more | `one account: net -11 yocto; two accounts: net -13 yocto (fill fees add up the same and round up; shares round down)` |
| 7 | I1 conservation after every step | `final supply 1000970000000000000000000 = launched 1001000000000000000000000 - burned 30000000000000000000` |

Arithmetic check for 3 and 4: alice's 600 CAL covers fill 0 → 0.6, average 0.3, so 600 × (1% + 10% × 0.3) = 24.
Bob's first 400 CAL cover fill 0.6 → 1.0 (average 0.8): 400 × 9% = 36; the 100 CAL past the HP pay
11% = 11; total 47. Pot 1000 + 24 + 47 = 1071. Weights 2400 and 1000: alice 1071 × 2400 / 3400 = 756,
bob 315.

### `rule_a_hunts` and `rule_b_and_splitting`: the same wasm with arithmetic-test values

These two tests use no fill fee and HP ×1.25 / ×0.8 so each payout can be checked by hand; the
values are set at init like any launch values.

| # | Claim | Expected `observed:` line |
|---:|---|---|
| A1 | every parameter is fixed at init and readable; the wasm exports no setter, admin or pause method | `config() == init args; 21 exports: claim, commit, config, contract_source_metadata, current_hunt, entry, ft_balance_of, ft_metadata, ft_resolve_transfer, ft_total_supply, ft_transfer, ft_transfer_call, hunt, new, resolve, storage_balance_bounds, storage_balance_of, storage_deposit, storage_unregister, storage_withdraw, totals` |
| A2 | NEP-141/NEP-145: transfers move the exact amount (no fee on transfers); unregistering is disabled | `3 x 10000 CAL received exactly; to unregistered dave: panic "The account dave.test.near is not registered"; storage_unregister: panic UNREGISTER_DISABLED` |
| A3 | claim before resolve and resolve before the deadline are refused | `claim(0): panic NOT_RESOLVED; resolve(0): panic BEFORE_DEADLINE` |
| A4 | commit locks the stake and charges the 1% tribute on top; under rule A stake past the HP is not taken | `alice 600 locked, paid 606; bob offered 500, locked 400, paid 404; carol at HP: panic NOTHING_TO_LOCK; events ["nep141:ft_transfer", "calydon:commit"]` |
| A5 | at the deadline anyone resolves once; a full bar is slain | `commit after deadline: panic HUNT_CLOSED; resolve(0) by carol (not a hunter) -> Slain; again: panic ALREADY_RESOLVED; resolve(5): panic NO_SUCH_HUNT; hunt 1 HP 1250 CAL` |
| A6 | slain: each hunter claims exactly once; dust goes back to the hoard (I2) | `alice 1206000000000000000000 = 600 + 606 CAL; second claim: panic NOTHING_TO_CLAIM; non-hunter: panic NOTHING_TO_CLAIM; bob 804000000000000000000 = 400 + 404 CAL; hoard after last claim = 1 yocto, unclaimed 0` |
| A7 | escaped: stake − 10% back, the burn rounded up; the supply falls by exactly the burns; the hoard rolls over | `alice 270 CAL back; carol 1001 yocto -> 900 back, 101 burned (ceil 100.1); supply -30000000000000000101 yocto; hoard unchanged 3000000000000000012; hunt 2 HP 1000 CAL; claim events ["nep141:ft_transfer", "nep141:ft_burn", "calydon:claim"]` |
| A8 | a tie at exactly HP is slain; an empty hunt resolves and the next opens | `hunt 2: 1000 committed vs HP 1000 -> Slain, alice gets 1000 + whole pot 13000000000000000012; hunt 3: 0 hunters -> Escaped; hunt 4 HP 1000 CAL` |
| A9 | I1 conservation after every step | `final supply 1000969999999999999999900 = launched 1001000000000000000000001 - burned 30000000000000000101` |
| B1 | rule B: an earlier commit earns more per token | `alice 500 at 3 steps left -> share 690 CAL; bob 700 at 1 step left -> share 322 CAL; dust back in hoard: 0 yocto` |
| B2 | I3 on chain at the smallest amounts: 2 yocto vs 1 + 1 | `one account: net +1 yocto; two accounts: net +0 yocto in total` |

## B. `npm run observe-testnet` (read-only, no key)

| Claim | Expected line |
|---|---|
| the deployed code is `res/calydon.wasm` | `OK    deployed code = res/calydon.wasm …` with `chain code_hash BKfwJS169ApsHRtyt34e5y1T64TqHGTFrhSEt57tM5kF; res/calydon.wasm sha256 995d5961af67e36a40c1f604c394952514897b3fec8802c5609ae52d163554ec` |
| the wasm names its source commit (NEP-330) | `NEP-330 on chain: git+https://github.com/ianlyoo/calydon?rev=aa316382f0dbfd609af2cb6eae8443de73df1925, image docker.io/library/rust:1.99.1-slim-bookworm` |
| the contract account has no access key | `OK    the contract account has 0 access keys …` / `view_access_key_list: 0 keys` |
| the launch values on chain are the recorded ones | `OK    config() = the launch values recorded at init` / `rule EarlyWeight, tribute 100 bps + fill fee up to 1000 bps, burn 1000 bps, HP x1.1 slain / x0.7 escaped, hunts of 300 blocks` |
| the recorded hunts (slain, escaped, slain) happened as recorded | `OK    all 23 recorded transactions succeeded on chain with the recorded return values` |
| conservation on chain | `OK    totals: held = hoard + locked + unclaimed; supply + burned = launched` |
| — | last line `All 5 checks hold.` |

The hunt log below the checks changes as people play: hunts 0–2 stay `Slain`, `Escaped`, `Slain`;
hunt 3 and later depend on who commits and resolves. The full output on 2026-11-08 is in
[README §2](README.md#2-check-the-live-testnet-instance--see-all-5-checks-hold).

## C. `npm run verify:reproducible` (Docker)

| Claim | Expected line |
|---|---|
| rebuilding the commit named in the wasm gives the same bytes | `OK    rebuilt from commit aa316382f0dbfd609af2cb6eae8443de73df1925: byte-identical to res/calydon.wasm` |
| the recorded sha256 matches | `OK    equals res/calydon.wasm.sha256` / `995d5961af67e36a40c1f604c394952514897b3fec8802c5609ae52d163554ec` |
| testnet runs those bytes | `OK    equals the code deployed on testnet at cal.calydon-v1-1794114512.testnet` / `chain code_hash BKfwJS169ApsHRtyt34e5y1T64TqHGTFrhSEt57tM5kF, rebuilt BKfwJS169ApsHRtyt34e5y1T64TqHGTFrhSEt57tM5kF` |
| — | last line `Source, committed wasm and deployed code are the same bytes.` |

Full output: [README §6](README.md#6-rebuild-the-wasm--see-the-same-bytes-as-on-testnet).

## D. The simulation tables

| Claim | Command | Expected |
|---|---|---|
| MECHANIC.md §3's tables are printed from the committed result files | `npm run sim-tables` | the same tables as in MECHANIC.md §3, character for character |
| the result files come from the committed wasm | `npm run simulate -- --seed 494` | the header names `wasm 995d5961…554ec`; the run ends `Written: sim/seed-494-hp300.json` and exits 0 only if every stake, refund, total and final balance matched the simulator's own arithmetic and I1 held |
| a re-run reproduces the table up to run-to-run variation | `npm run simulate -- --seed 494 --out mine.json`, then compare `sim/mine.json` with MECHANIC.md §3.3 | in our three runs: the same kill rate (49/60 rule B, 48/60 rule A) and the same sign for every rule-B strategy; single percentages moved by up to 13 points and the top strategy changed once (the sandbox picks the order of same-round commits, and the fee depends on it) |

## E. Playing on testnet

| Claim | Command | Expected |
|---|---|---|
| anyone can launch an instance with the launch values and lock it | `npm run hunt -- launch` | `launched cal.calydon-play-<n>.testnet (no access key left); …` then the status of hunt 0 |
| a commit locks the stake and charges the fill fee | `npm run hunt -- commit 600 --as alice` | `… committed 600 CAL`, a nearblocks link, `locked 600 CAL`, the hoard up by the fee (24 CAL into an empty 1000-CAL bar) |
| a late commit is refused and nothing moves | a commit after the deadline | `refused: HUNT_CLOSED` |
| anyone resolves after the deadline | `npm run hunt -- resolve` | `hunt <n> resolved by …`, `outcome "Slain"` or `"Escaped"` |
| a claim pays once | `npm run hunt -- claim <n> --as alice` | `paid <stake + share> CAL` (slain) or `paid <stake − 10%> CAL` (escaped); a second claim: `refused: NOTHING_TO_CLAIM` |

A recorded session with links is in [README §4](README.md#4-play-on-testnet--see-your-commit-the-resolve-and-your-claim-as-transactions).

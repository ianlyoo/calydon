# VERIFY: claim → command → expected output

Every claim Calydon makes, the command that checks it, and what you should see. Run the commands
from a clone of this repository (prerequisites: [README §1](README.md#1-check-the-contract-in-one-command--see-every-claim-held)).
Each `HELD` line is printed only after the test has asserted it on contract state, balances or
execution outcomes; if a claim were false, the command would exit non-zero.

## What is checked where

| Command | Needs | Checks |
|---|---|---|
| `npm run verify` | Rust (rustup), Node, network for the first download. No Docker, account or key. | Section A: the unit tests and 23 sandbox claims against the committed `res/calydon.wasm` |
| `npm run observe-testnet` | Node only, no key | Section B: the deployed code, the one-transaction init and 0 access keys, the launch values, the 23 recorded transactions and the totals on testnet |
| `npm run verify:reproducible` | Docker (pulls the pinned linux/amd64 Rust image) | Section C: source → `res/calydon.wasm` → testnet code hash are the same bytes |
| `npm run simulate`, `npm run sim-tables`, `npm run sim-converge` | as `npm run verify`; @@SIMTIME@@ | Section D: MECHANIC.md's tables come from the committed wasm, checked against an independent oracle |
| `npm run hunt -- …` | Node, Rust, network; the testnet faucet (free) | Section E: starter / commit / resolve / claim from your own faucet accounts |
| `npm run page` | Node; a browser and a testnet account in Intear Wallet to sign | Section F: the page signs the same calls |

## A. `npm run verify` (local sandbox, no account or key)

Expected final lines of the sandbox test file:

```text
Calydon verify (v1.1 storage, starter, sweep): 5 claims held, 0 broken.
ok

test result: ok. 4 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out
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
| 1 | launch values are fixed at init and readable: rule B, tribute 1%, fill fee up to +10%, burn 10%, HP x1.1 / x0.7 | `config() == {"name":"Calydon","symbol":"CAL","decimals":18,"rule":"EarlyWeight","first_hp":"1000000000000000000000","hunt_blocks":400,"step_blocks":100,"tribute_bps":100,"fill_fee_bps":1000,"burn_bps":1000,"hp_up_bps":11000,"hp_down_bps":7000,"min_stake":"1000000000000000000","entry_deposit":"5000000000000000000000","record_deposit":"5000000000000000000000","starter_amount":"1000000000000000000000"}` |
| 2 | refused commits change nothing: below the 1-CAL minimum stake, a new entry without its storage deposit, stake + fee above the balance | `commit(1 CAL - 1 yocto): panic BELOW_MIN_STAKE; new entry with 1 yocto attached: panic ATTACH_ENTRY_DEPOSIT; 10000 of 10000 (fee on top): panic The account doesn't have enough balance; alice still 10000 CAL, locked 0` |
| 3 | the entry fee rises with the bar's fill and goes into the hoard; rule B weights a commit by the steps left | `alice 600 into an empty bar: fee 24 CAL (1% + 10% x 0.3 average fill); bob 500 at 600/1000: fee 47 CAL (400 at 1% + 10% x 0.8, 100 past HP at 11%); weights 600x4, 500x2; hoard 1000 + 71 = 1071 CAL` |
| 4 | slain under the launch values: shares of the 1071 CAL pot by weight, rounded down, once each; next HP x1.1 | `alice 1356000000000000000000 = 600 + 756.0000 CAL; bob 815000000000000000000 = 500 + 315.0000 CAL; second claim: panic NOTHING_TO_CLAIM; dust 0 yocto back in the hoard; hunt 1 HP 1100 CAL` |
| 5 | escaped under the launch values: stake - 10% back, the 10% burns, the entry fee stays in the hoard; next HP x0.7 | `carol 300 -> 270 back, supply -30 CAL; hoard = dust + carol's fee 7.090909 CAL; hunt 2 HP 770 CAL` |
| 6 | I3 on chain with the fill fee: 600 from one account vs 300 + 300 from two accounts does not pay more | `one account: net -11 yocto; two accounts: net -13 yocto (fill fees add up the same and round up; shares round down)` |
| 7 | I1 conservation held after every step under the launch values | `final supply 1000970000000000000000000 = launched 1001000000000000000000000 - burned 30000000000000000000` |

Arithmetic check for 3 and 4: alice's 600 CAL covers fill 0 → 0.6, average 0.3, so 600 × (1% + 10% × 0.3) = 24.
Bob's first 400 CAL cover fill 0.6 → 1.0 (average 0.8): 400 × 9% = 36; the 100 CAL past the HP pay
11% = 11; total 47. Pot 1000 + 24 + 47 = 1071. Weights 2400 and 1000: alice 1071 × 2400 / 3400 = 756,
bob 315.

### `rule_a_hunts` and `rule_b_and_splitting`: the same wasm with arithmetic-test values

These two tests use no fill fee and HP ×1.25 / ×0.8 so each payout can be checked by hand; the
values are set at init like any launch values.

| # | Claim | Expected `observed:` line |
|---:|---|---|
| A1 | every parameter is fixed at init and readable; the wasm exports no setter, admin or pause method (I4; contract_source_metadata is the sdk's read-only NEP-330 view); create + deploy + new is one transaction and the contract account has no access key | `config() == init args; 0 access keys; 23 exports: claim, commit, config, contract_source_metadata, current_hunt, entry, ft_balance_of, ft_metadata, ft_resolve_transfer, ft_total_supply, ft_transfer, ft_transfer_call, hunt, new, resolve, starter, starter_taken, storage_balance_bounds, storage_balance_of, storage_deposit, storage_unregister, storage_withdraw, totals` |
| A2 | NEP-141/NEP-145: storage_deposit registers; ft_transfer moves the exact amount (no tribute on transfers); unregistering is disabled | `3 x 10000 CAL received exactly; to unregistered dave: panic "The account dave.test.near is not registered"; storage_unregister: panic UNREGISTER_DISABLED` |
| A3 | claim before resolve and resolve before the deadline are refused | `claim(0): panic NOT_RESOLVED; resolve(0): panic BEFORE_DEADLINE` |
| A4 | commit locks the stake and charges the 1% tribute on top into the hoard; under rule A stake past the HP is not taken | `alice 600 locked, paid 606; bob offered 500, locked 400, paid 404; carol at HP: panic NOTHING_TO_LOCK; events ["nep141:ft_transfer", "calydon:commit"]` |
| A5 | at the deadline anyone resolves once: committed 1000 >= HP 1000, slain; next HP x1.25 | `commit after deadline: panic HUNT_CLOSED; resolve(0) by carol (not a hunter) -> Slain; again: panic ALREADY_RESOLVED; resolve(5): panic NO_SUCH_HUNT; hunt 1 HP 1250 CAL` |
| A6 | slain: each hunter claims stake + pot x stake / committed, rounded down, exactly once; dust goes back to the hoard (I2) | `alice 1206000000000000000000 = 600 + 606 CAL; second claim: panic NOTHING_TO_CLAIM; non-hunter: panic NOTHING_TO_CLAIM; bob 804000000000000000000 = 400 + 404 CAL; hoard after last claim = 1 yocto, unclaimed 0` |
| A7 | escaped: each hunter claims stake - 10% (burn rounded up); the supply falls by exactly the burns; hoard rolls over; next HP x0.8 | `alice 270 CAL back; carol 1001 yocto -> 900 back, 101 burned (ceil 100.1); supply -30000000000000000101 yocto; hoard unchanged 3000000000000000012; hunt 2 HP 1000 CAL; claim events ["nep141:ft_transfer", "nep141:ft_burn", "calydon:claim"]` |
| A8 | a tie at exactly HP is slain; an empty hunt resolves (escaped, nothing to burn) and the next opens (I5) | `hunt 2: 1000 committed vs HP 1000 -> Slain, alice gets 1000 + whole pot 13000000000000000012; hunt 3: 0 hunters -> Escaped; hunt 4 HP 1000 CAL` |
| A9 | I1 conservation held after every step: holder balances + hoard + locked + unclaimed = supply = launch - burns | `final supply 1000969999999999999999900 = launched 1001000000000000000000001 - burned 30000000000000000101` |
| B1 | rule B: no cap; the hoard is shared by stake x steps left, so an earlier commit earns more per token | `alice 500 at 3 steps left -> share 690 CAL; bob 700 at 1 step left -> share 322 CAL; dust back in hoard: 0 yocto` |
| B2 | I3 on chain: splitting a stake of 2 into 1 + 1 does not pay more (shares round down, tributes round up) | `one account: net +1 yocto; two accounts: net +0 yocto in total (the 20,000-case property test is in src/tests.rs)` |

### `v1_1_storage_starter_sweep`: storage deposits, the starter bucket, the sweep, First Spear (launch values)

| # | Claim | Expected `observed:` line |
|---:|---|---|
| S1 | starter(): a registered account takes 1,000 CAL once from the starter bucket; the last one gets what is left, then it is empty | `unregistered: panic NOT_REGISTERED; no deposit: panic ATTACH_RECORD_DEPOSIT; alice, bob, carol got 1000, 1000, 500 CAL; alice again: panic STARTER_TAKEN; dave: panic STARTER_EMPTY` |
| S2 | storage: a commit that creates an entry must attach the entry deposit (5 mNEAR), a later commit to the same entry 1 yocto; the contract's free NEAR does not fall | `alice's first commit added 119 bytes (with the hunt's First Spear), bob's 98 bytes = 980000000000000000000 yoctoNEAR of storage, under the 5 mNEAR attached; alice's second commit with 5 mNEAR: panic ATTACH_ONE_YOCTO; carol's first with 1 yocto: panic ATTACH_ENTRY_DEPOSIT; alice's second with 1 yocto: +0 bytes; free NEAR 2197520000000000000000000 -> 2205350000000000000000001 yocto` |
| S3 | the halt scenario: storage growth that the attached NEAR does not cover is refused, so commits cannot spend the contract's own NEAR | `instance with entry_deposit = 1 yocto: first commit panics STORAGE_NOT_COVERED; no entry stored; contract free NEAR not lower` |
| S4 | a plain transfer to the contract (77 CAL) shows as held - parts; resolve sweeps it into the hoard, so the identity holds again and it is paid out with the pot | `before resolve: held - (hoard + locked + unclaimed + starter_left) = 77 CAL; resolve without the record deposit: panic ATTACH_RECORD_DEPOSIT; resolve emits sweep 77 CAL; pot 1079350000000000000001 = hoard 1002350000000000000001 + 77 CAL; after: held == parts` |
| S5 | claim pays the CAL and sends the 5 mNEAR entry deposit back; First Spear of hunt 0 is alice, the first to commit | `alice claimed 454.7352 CAL and her NEAR rose by 0.004481 (5 mNEAR minus gas); first_spear = "alice.test.near"; contract free NEAR 2197520000000000000000000 -> 2200810000000000000000002 yocto` |

S2 and S3 are the storage halt of v1.0 and its fix ([MECHANIC.md §5](MECHANIC.md#5-limits-stated-plainly)):
S3 runs a second instance whose `entry_deposit` is 1 yocto, so a new entry's storage is not covered,
and the commit is refused instead of being paid from the contract's own NEAR. S4 is the donation
case: CAL sent to the contract by plain transfer is outside hoard + locked + unclaimed + starter
bucket until `resolve` sweeps it into the hoard.

## B. `npm run observe-testnet` (read-only, no key)

| Claim | Expected line |
|---|---|
| the deployed code is `res/calydon.wasm` | `OK    deployed code = res/calydon.wasm …` with `chain code_hash 21jLBW4mzWYK1oW2bV2HTC7yRPaKS6MMmr5NjaUxAgpt; res/calydon.wasm sha256 0f0b8ce7b02ca03d1edf1d4e0053a198f880cd1131e8dbaf288a0594ee15754d` |
| the wasm names its source commit (NEP-330) | `NEP-330 on chain: git+https://github.com/ianlyoo/calydon?rev=9000bcf84cbe5d0fb0b94a85c27cb3da21812556, image docker.io/library/rust:1.99.1-slim-bookworm` |
| the contract account never had an access key | `OK    the contract account was created, deployed and initialised in one transaction with no key, and has 0 access keys now …` / `init tx 2rTsy5KCJjw3KFJsBdTCHL9M1TSrNSva741uQPAgnwDp actions: CreateAccount, Transfer, DeployContract, FunctionCall(new); view_access_key_list: 0 keys` |
| the launch values on chain are the recorded ones | `OK    config() = the launch values recorded at init` / `rule EarlyWeight, tribute 100 bps + fill fee up to 1000 bps, burn 1000 bps, HP x1.1 slain / x0.7 escaped, hunts of 300 blocks` |
| the recorded run (starters; hunts slain, escaped with a 50 CAL sweep, slain) happened as recorded | `OK    all 23 recorded transactions succeeded on chain with the recorded return values` |
| conservation on chain; any CAL sent by plain transfer since the last resolve is reported | `OK    totals: held >= hoard + locked + unclaimed + starter bucket (surplus reported); supply + burned = launched` and `surplus <n> CAL` |
| — | last line `All 5 checks hold.` |

The hunt log below the checks changes as people play: hunts 0–2 stay `Slain`, `Escaped`, `Slain`;
later hunts depend on who commits and resolves (hunt 4 is the browser session). The full output is in
[README §2](README.md#2-check-the-live-testnet-instance--see-all-5-checks-hold). The superseded v1.0
instance: `node scripts/observe-testnet.mjs testnet/v1.0/run.json` (also `All 5 checks hold.`).

## C. `npm run verify:reproducible` (Docker host)

| Claim | Expected line |
|---|---|
| rebuilding the commit named in the wasm gives the same bytes | `OK    rebuilt from commit 9000bcf84cbe5d0fb0b94a85c27cb3da21812556: byte-identical to res/calydon.wasm` |
| the recorded sha256 matches | `OK    equals res/calydon.wasm.sha256` / `0f0b8ce7b02ca03d1edf1d4e0053a198f880cd1131e8dbaf288a0594ee15754d` |
| testnet runs those bytes | `OK    equals the code deployed on testnet at cal.calydon-v11-1794122775.testnet` / `chain code_hash 21jLBW4mzWYK1oW2bV2HTC7yRPaKS6MMmr5NjaUxAgpt, rebuilt 21jLBW4mzWYK1oW2bV2HTC7yRPaKS6MMmr5NjaUxAgpt` |
| — | last line `Source, committed wasm and deployed code are the same bytes.` |

Full output: [README §6](README.md#6-rebuild-the-wasm--see-the-same-bytes-as-on-testnet).

## D. The simulation tables

| Claim | Command | Expected |
|---|---|---|
| MECHANIC.md §3's tables are printed from the committed result files | `npm run sim-tables` | the same tables as in MECHANIC.md §3, character for character |
| the result files come from the committed wasm | `npm run simulate -- --seed 494` | the header names `wasm 995d5961…554ec`; the run ends `Written: sim/seed-494-hp300.json` and exits 0 only if every stake, refund, total and final balance matched the simulator's own arithmetic and I1 held |
| a re-run reproduces the table up to run-to-run variation | `npm run simulate -- --seed 494 --out mine.json`, then compare `sim/mine.json` with MECHANIC.md §3.3 | in our three runs: the same kill rate (49/60 rule B, 48/60 rule A) and the same sign for every rule-B strategy; single percentages moved by up to 13 points and the top strategy changed once (the sandbox picks the order of same-round commits, and the fee depends on it) |

## E. Playing on testnet from the command line

| Claim | Command | Expected |
|---|---|---|
| anyone can launch an instance with the launch values; the contract account never has a key | `npm run hunt -- launch` | `launched cal.calydon-play-<n>.testnet (no access key); holder …; alice and bob took 1000-CAL starters` then the status of hunt 0 |
| a starter is taken once per account | `npm run hunt -- starter --as <name>` | `… took the starter`, `got 1000 CAL`; a second time: `refused: STARTER_TAKEN` |
| a commit locks the stake and charges the fill fee | `npm run hunt -- commit 600 --as alice` | `… committed 600 CAL`, a nearblocks link, `locked 600 CAL`, the hoard up by the fee (24 CAL into an empty 1000-CAL bar) |
| a late commit is refused and nothing moves | a commit after the deadline | `refused: HUNT_CLOSED` |
| anyone resolves after the deadline | `npm run hunt -- resolve` | `hunt <n> resolved by …`, `outcome "Slain"` or `"Escaped"` |
| a claim pays once | `npm run hunt -- claim <n> --as alice` | `paid <stake + share> CAL` (slain) or `paid <stake − 10%> CAL` (escaped); a second claim: `refused: NOTHING_TO_CLAIM` |

A recorded session with links is in [README §4](README.md#4-play-on-testnet-from-the-command-line--see-your-commit-the-resolve-and-your-claim-as-transactions).

## F. Playing in a browser

| Claim | How | Expected |
|---|---|---|
| the page is static and runs locally | `npm run page`, open `http://127.0.0.1:8494/status/` | the current hunt of the v1.1 instance, its HP bar, deadline line and hunt log, without signing in |
| it signs in with a testnet wallet | `Sign in with Intear Wallet (testnet)` | `Signed in as <account>`, your CAL balance, registration and starter status |
| each call is signed in the wallet and lands on chain | the page's `storage_deposit`, `Get starter CAL`, `Commit`, `Resolve this hunt`, `Claim` buttons | `<call>: done. https://testnet.nearblocks.io/txns/<hash>` under your account name |
| the commit preview is the contract's arithmetic | type a stake | `You pay` = stake + the fee integrated over the fill the stake covers (the same `fill_fee` as the contract), weight = stake × steps left, and the claim if slain or escaped |
| a claim pays what *Your entries* showed and returns the entry deposit | `Claim` | the CAL in the `Claim pays` column; the claim transaction also transfers 0.005 NEAR back |

Recorded: [docs/browser/](docs/browser/) and [README §3](README.md#3-play-in-a-browser--sign-in-commit-resolve-and-claim-from-the-page)
(commit 600 CAL previewed as `643.1733 CAL = stake 600 CAL + entry fee 43.1733 CAL` and `If slain: claim
643.1733 CAL`; the claim transaction B4rnod1h… returned `643173350000000000000` and transferred `5000000000000000000000` yoctoNEAR back).

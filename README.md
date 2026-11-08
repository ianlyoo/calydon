# Calydon

A memecoin you hunt together, on NEAR. Holders lock their own CAL against a beast's HP before a
deadline. If the total reaches the HP, the beast is slain and the hunters share the hoard. If it
falls short, the beast escapes with 10% of every stake, and that 10% burns.

Status: **v1.0, NEAR testnet only.** The v1.0 instance
[`cal.calydon-v1-1794114512.testnet`](https://testnet.nearblocks.io/address/cal.calydon-v1-1794114512.testnet)
has **0 access keys**: every key was deleted right after `new`, so no transaction can be signed as
that account and its code cannot be replaced. Its code hash equals `res/calydon.wasm`, which
`npm run verify:reproducible` rebuilds byte for byte from this repository. `npm run observe-testnet`
checks both without any key.

Every claim below comes with the command that checks it and the output it printed for us.
[VERIFY.md](VERIFY.md) lists each claim → command → expected output. The design, the simulation
tables and the limits are in [MECHANIC.md](MECHANIC.md).

## The rules (launch values, fixed at `new`, readable through `config()`)

| | |
|---|---|
| `commit(amount)` | Locks your own CAL in the current hunt. The entry fee is paid on top and goes into the hoard: 1% for a token that goes into an empty bar, rising linearly to 11% for a token that goes into a full bar (and 11% past the HP). |
| Reward rule B | A commit's weight is stake × the number of steps left before the deadline (4 steps per hunt on testnet). |
| `resolve(hunt)` | Anyone, once, after the deadline. Slain if committed ≥ HP. |
| `claim(hunt)` | Once per hunter. Slain: stake + hoard × weight / total weight, rounded down. Escaped: stake − 10% (the 10% burns, rounded up); the hoard rolls over. |
| Next HP | ×1.1 after a kill, ×0.7 after an escape (equilibrium kill rate ≈ 79%). |
| Supply | Minted once at `new`; it only falls, by escape burns. `ft_transfer` / `ft_transfer_call` are plain NEP-141 with no fee. |
| Admin | None: no owner, setter, pause or upgrade method. The views are `config`, `current_hunt`, `hunt`, `entry`, `totals`. |

Why each rule exists, and the problem the rising entry fee answers: [MECHANIC.md §1–2](MECHANIC.md).

## 1. Check the contract in one command → see every claim `HELD`

Prerequisites (Linux, macOS or WSL, x86_64 or arm64):

- a C toolchain, `pkg-config` and OpenSSL headers (Debian/Ubuntu:
  `apt-get install -y build-essential pkg-config libssl-dev curl git nodejs npm`; macOS: the Xcode
  command line tools and `brew install pkg-config openssl node`);
- Rust via rustup (`curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal`,
  then `. "$HOME/.cargo/env"`). `rust-toolchain.toml` makes rustup fetch Rust 1.99.1 and the wasm target;
- Node.js 18 or newer. There are no npm dependencies.
- No Docker, account, key or paid API. Docker is needed only for `npm run verify:reproducible` (§6).

```sh
git clone https://github.com/ianlyoo/calydon.git
cd calydon
npm run verify
```

`npm run verify` is `cargo test --locked -- --nocapture --test-threads=1`. It runs the unit tests
(including a 20,000-case property test for splitting) and then starts a local NEAR sandbox, deploys
the committed `res/calydon.wasm` (the bytes deployed on testnet) and asserts every claim on
contract state, balances and execution outcomes, checking conservation (I1) after every step. The
first run downloads the crates and the sandbox binary. Nothing in it touches testnet.

Expected output (abridged), from a fresh clone of this repository on 2026-11-08 on an Apple Silicon Mac (6 min 3 s, crates already downloaded, compilation included):

```text
test tests::fill_fees ... ok
test tests::m2_counterexample_is_the_safe_direction ... ok
test tests::rounding_helpers ... ok
test tests::splitting_never_pays_more ... ok
test result: ok. 4 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.24s

test execute_testnet ... ignored, uses NEAR testnet and the faucet; run with `npm run execute-testnet`

test launch_values ... HELD  1  launch values are fixed at init and readable: rule B, tribute 1%, fill fee up to +10%, burn 10%, HP x1.1 / x0.7
HELD  2  refused commits change nothing: zero stake, missing 1 yocto, stake + fee above the balance
HELD  3  the entry fee rises with the bar's fill and goes into the hoard; rule B weights a commit by the steps left
         observed: alice 600 into an empty bar: fee 24 CAL (1% + 10% x 0.3 average fill); bob 500 at 600/1000: fee 47 CAL (400 at 1% + 10% x 0.8, 100 past HP at 11%); weights 600x4, 500x2; hoard 1000 + 71 = 1071 CAL
HELD  4  slain under the launch values: shares of the 1071 CAL pot by weight, rounded down, once each; next HP x1.1
HELD  5  escaped under the launch values: stake - 10% back, the 10% burns, the entry fee stays in the hoard; next HP x0.7
HELD  6  I3 on chain with the fill fee: 600 from one account vs 300 + 300 from two accounts does not pay more
         observed: one account: net -11 yocto; two accounts: net -13 yocto (fill fees add up the same and round up; shares round down)
HELD  7  I1 conservation held after every step under the launch values
Calydon verify (launch values: R2 fee, rule B, HP x1.1/x0.7): 7 claims held, 0 broken.
ok
test rule_a_hunts ... HELD  1  every parameter is fixed at init and readable; the wasm exports no setter, admin or pause method (I4; contract_source_metadata is the sdk's read-only NEP-330 view)
         observed: config() == init args; 21 exports: claim, commit, config, contract_source_metadata, current_hunt, entry, ft_balance_of, ft_metadata, ft_resolve_transfer, ft_total_supply, ft_transfer, ft_transfer_call, hunt, new, resolve, storage_balance_bounds, storage_balance_of, storage_deposit, storage_unregister, storage_withdraw, totals
...
Calydon verify (rule A, arithmetic-test values: no fill fee, HP x1.25/x0.8): 9 claims held, 0 broken.
ok
test rule_b_and_splitting ... HELD  1  rule B: no cap; the hoard is shared by stake x steps left, so an earlier commit earns more per token
HELD  2  I3 on chain: splitting a stake of 2 into 1 + 1 does not pay more (shares round down, tributes round up)
Calydon verify (rule B and splitting, arithmetic-test values): 2 claims held, 0 broken.
ok

test result: ok. 3 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 98.21s
```

`rule_a_hunts` and `rule_b_and_splitting` run the same wasm with arithmetic-test values (no fill fee,
HP ×1.25 / ×0.8) so each payout can be checked by hand; `launch_values` runs exactly the launch values.
The full list of claims and their `observed:` lines: [VERIFY.md §A](VERIFY.md#a-npm-run-verify-local-sandbox-no-account-or-key).

## 2. Check the live testnet instance → see `All 5 checks hold`

```sh
npm run observe-testnet
```

Read-only, Node only, no key. It reads [testnet/run.json](testnet/run.json) (the recorded v1.0 run)
and asks the public testnet RPC. Output on 2026-11-08 (hunt 3 is open or waiting to be resolved
whenever you run it; anyone may resolve it):

```text
Calydon v1.0 on NEAR testnet: cal.calydon-v1-1794114512.testnet

OK    deployed code = res/calydon.wasm (the reproducible build; rebuild it with npm run verify:reproducible)
      chain code_hash BKfwJS169ApsHRtyt34e5y1T64TqHGTFrhSEt57tM5kF; res/calydon.wasm sha256 995d5961af67e36a40c1f604c394952514897b3fec8802c5609ae52d163554ec
      NEP-330 on chain: git+https://github.com/ianlyoo/calydon?rev=aa316382f0dbfd609af2cb6eae8443de73df1925, image docker.io/library/rust:1.99.1-slim-bookworm
OK    the contract account has 0 access keys (locked: no redeploy, no admin call)
      view_access_key_list: 0 keys
OK    config() = the launch values recorded at init
      rule EarlyWeight, tribute 100 bps + fill fee up to 1000 bps, burn 1000 bps, HP x1.1 slain / x0.7 escaped, hunts of 300 blocks
OK    all 23 recorded transactions succeeded on chain with the recorded return values
      23 of 23, e.g. https://testnet.nearblocks.io/txns/4ZQNdYyEcEc8248n36MH6nsWdWANUPiRL2NUyqjFh6RV
OK    totals: held = hoard + locked + unclaimed; supply + burned = launched
      supply 1000980 CAL, burned 20, hoard 0, locked 0, unclaimed 0

Hunt log:
  hunt 0: HP 1000 CAL, committed 1100 by 3, Slain, pot 1071 CAL, claims 3/3
  hunt 1: HP 1100 CAL, committed 200 by 1, Escaped, claims 1/1
  hunt 2: HP 770 CAL, committed 800 by 2, Slain, pot 53.318181 CAL, claims 2/2
  hunt 3: HP 847 CAL, committed 0 by 0, deadline passed, not resolved yet, claims 0/0

All 5 checks hold.
```

If the public RPC is slow, point the scripts at another one with `CALYDON_TESTNET_RPC=<url>`
(default `https://test.rpc.fastnear.com`).

### The recorded testnet run

`npm run execute-testnet` produced [testnet/run.json](testnet/run.json) on 2026-11-08 (14:18 KST):
a fresh faucet account `calydon-v1-1794114512.testnet` holds the supply (1,000,000 CAL + a
1,000 CAL launch hoard), `cal.<it>` runs the contract, and three hunters get 5,000 CAL each. Each
fee and payout was checked against the contract's arithmetic (`fill_fee`, `share`) before it was
recorded.

| Step | Returned | Transaction |
|---|---|---|
| init (`new` with the launch values) | – | [E85oMyyK…](https://testnet.nearblocks.io/txns/E85oMyyKAeeJm68THoTWHiZq6WqyFqgeqYd8zh9FYGSw) |
| lock: delete every access key of the contract account | – | [99pyyzWn…](https://testnet.nearblocks.io/txns/99pyyzWnvpJBzsPkdmRgUoQDctiwcGWCHFzP36w49hww) |
| hunt 0: alice commit 400 CAL (bar 0/1000, fee 12 CAL) | 400 CAL locked | [ExKZPEWg…](https://testnet.nearblocks.io/txns/ExKZPEWgqY37UXLVrbbxZBQsif5k3B99LjqPqj1GcBr4) |
| hunt 0: bob commit 300 CAL (bar 400/1000, fee 19.5 CAL) | 300 CAL locked | [9jk68d3P…](https://testnet.nearblocks.io/txns/9jk68d3P5c2BojJjtsW8sngQSXruudWwaMuJQVMyuM29) |
| hunt 0: carol commit 400 CAL (bar 700/1000, fee 39.5 CAL) | 400 CAL locked | [5BeC6oPX…](https://testnet.nearblocks.io/txns/5BeC6oPXT1CJurWLg9Uz8xtmq7naG31XEJMx7XQWMW8N) |
| hunt 0: resolve | `Slain` | [4ZQNdYyE…](https://testnet.nearblocks.io/txns/4ZQNdYyEcEc8248n36MH6nsWdWANUPiRL2NUyqjFh6RV) |
| hunt 0: alice claim | 789.454545… CAL | [6Lrxcm6J…](https://testnet.nearblocks.io/txns/6Lrxcm6JTnzPsDvbYY1MLfAHzbEUiB6ctss3ziSUkUzr) |
| hunt 0: bob claim | 592.090909… CAL | [5PpoD1cH…](https://testnet.nearblocks.io/txns/5PpoD1cHjmwKZNQUvBpeLTrMF2FK8BsGBo4NNGBUo7Xo) |
| hunt 0: carol claim | 789.454545… CAL | [me1tJfQM…](https://testnet.nearblocks.io/txns/me1tJfQMVBxRMQMAhkny7MjH8TkjpzRUyNkBEuSvpy6) |
| hunt 1: alice commit 200 CAL (bar 0/1100, fee 3.8182 CAL) | 200 CAL locked | [FkM474gX…](https://testnet.nearblocks.io/txns/FkM474gXwNs3FCTqGV7SMY2ZAFgfgfSiKHhEKDk2UzVP) |
| hunt 1: resolve | `Escaped` | [8Gy7vXAZ…](https://testnet.nearblocks.io/txns/8Gy7vXAZymK3nPsKeVDUTKjqhssuDy7YbnKriPuZhiAy) |
| hunt 1: alice claim (200 − 10%; 20 CAL burned) | 180 CAL | [9RaZRLcg…](https://testnet.nearblocks.io/txns/9RaZRLcgkko51jt7bTrJykMj2hG5t1XbLkDEmTU5KGW8) |
| hunt 2: bob commit 500 CAL (bar 0/770, fee 21.2338 CAL) | 500 CAL locked | [4nHiDNFJ…](https://testnet.nearblocks.io/txns/4nHiDNFJohagnpQB4a16voBiHqAhnKGc4Me4Yrx1Gkm8) |
| hunt 2: carol commit 300 CAL (bar 500/770, fee 28.2662 CAL) | 300 CAL locked | [3qUQFbpC…](https://testnet.nearblocks.io/txns/3qUQFbpCnu4ewQCGoPfcH5iq6JKKngELy8FdyKJVQNxW) |
| hunt 2: resolve | `Slain` | [7QZWgRBq…](https://testnet.nearblocks.io/txns/7QZWgRBq2SWR9rwT2takG3Jqo7Tj11gfEGSfm1BjLVtM) |
| hunt 2: bob claim | 533.323863… CAL | [BV1BWmC5…](https://testnet.nearblocks.io/txns/BV1BWmC5XHGRHZXfTqNdqATeZjNYRLKREQtZtcH2xoXx) |
| hunt 2: carol claim | 319.994318… CAL | [BuCnEN2v…](https://testnet.nearblocks.io/txns/BuCnEN2vfffqcFo9BggUCRALzCWDJobi5bnTKnK9FxeF) |

The six setup transactions (storage deposits and transfers to the hunters) are in the record too.
What the numbers show: carol's 400 CAL went in last, partly past the HP, and paid 39.5 CAL in fees
against alice's 12 CAL for the same stake into an empty bar. All three landed in the same rule-B
step, so their shares of the 1,071 CAL pot are pro rata. Hunt 2's pot (53.318181 CAL) is the escaped
hunt 1's fee plus hunt 2's own fees.

## 3. Watch it → see the HP bar, the hoard and the hunt log

```sh
npm run status-page
```

```text
Calydon status page: http://127.0.0.1:8494  (Ctrl-C to stop)
```

Open that address. The page is served from this machine only and nothing is hosted. Your browser
reads the testnet RPC every 5 seconds and shows the current hunt's HP bar and deadline, committed,
HP, the fee the next 1 CAL would pay, the hoard, the supply, the CAL burned so far, the number of
access keys on the contract (0), and the hunt log. `?contract=<id>` shows another instance, for
example one you launched in §4, and `?rpc=<url>` uses another RPC.

## 4. Play on testnet → see your commit, the resolve and your claim as transactions

```sh
npm run hunt -- launch                     # your own instance, locked like the v1.0 one
npm run hunt -- commit 600 --as alice
npm run hunt -- status
npm run hunt -- resolve                    # after the deadline; anyone may
npm run hunt -- claim 0 --as alice
```

`launch` creates a fresh account from the testnet faucet (free, no key or token of yours needed),
deploys `res/calydon.wasm` with the launch values, deletes the contract account's keys, and creates
two hunters, `alice` and `bob`, with 5,000 CAL each. Their keys are random and saved under
`.credentials/testnet/` (ignored by git; never printed). A hunt lasts 600 blocks (about 6 minutes on
testnet; `--hunt-blocks N` changes it). Each command compiles once, then took us 30–55 seconds per
call against the public RPC. If a commit lands after the deadline, the contract refuses it with
`HUNT_CLOSED` and nothing moves; the session below shows that with a 120-block hunt. Without a
`launch`, `status` shows the recorded v1.0 instance; its CAL sits with our hunters, so play on your
own instance (or pass `--contract <id>`).

Output from one such session (instance `cal.calydon-play-1794115227.testnet`, launched with
`--hunt-blocks 120`):

```text
$ npm run hunt -- launch --hunt-blocks 120
launched cal.calydon-play-1794115227.testnet (no access key left); holder calydon-play-1794115227.testnet; alice and bob hold 5000 CAL each
contract cal.calydon-play-1794115227.testnet (rule EarlyWeight, tribute 100 bps + fill fee up to 1000 bps, burn 1000 bps, HP x1.1 / x0.7)
hunt 0: [....................] 0 / 1000 CAL committed by 0 hunter(s)
  open: 87 blocks left (deadline 276732429); the next 1 CAL committed pays 0.01005 CAL fee
hoard 1000 CAL · supply 1001000 CAL · burned 0 CAL · unclaimed 0 CAL

$ npm run hunt -- commit 600 --as alice
alice.calydon-play-1794115227.testnet committed 600 CAL
  https://testnet.nearblocks.io/txns/3Mcn2LVzeFq2M7NsM3jRHq9mAm76JQ9fSSc6Rt7EL3mt
  locked 600 CAL
hunt 0: [############........] 600 / 1000 CAL committed by 1 hunter(s)
  open: 6 blocks left (deadline 276732429); the next 1 CAL committed pays 0.07005 CAL fee
hoard 1024 CAL · supply 1001000 CAL · burned 0 CAL · unclaimed 0 CAL

$ npm run hunt -- commit 500 --as bob
Error: bob.calydon-play-1794115227.testnet committed 500 CAL refused: HUNT_CLOSED
  https://testnet.nearblocks.io/txns/CT2L9UVhDsWig3mFJSZU4XzFmY1Y72DBDbv3FScKHDnu

$ npm run hunt -- resolve
hunt 0 resolved by calydon-play-1794115227.testnet
  https://testnet.nearblocks.io/txns/CwDyip9EvFgTdF1gshMXDsDRQK4JbLyWKZCbRD21WYUk
  outcome "Escaped"

$ npm run hunt -- claim 0 --as alice
alice.calydon-play-1794115227.testnet claimed hunt 0
  https://testnet.nearblocks.io/txns/D2PBvXmQA72nvxXUfFYXJbVCsm3VSTnYw6GJaERhdiBy
  paid 540 CAL; balance now 4916 CAL
```

(600 − 10% = 540 back; 60 CAL burned; alice's 24 CAL fee stayed in the hoard.) Later in the same
session bob committed 600 CAL into hunt 2 (HP 490), it was slain, and his claim paid
`1665.5 CAL` ([Cd6pyTrF…](https://testnet.nearblocks.io/txns/Cd6pyTrFz7yF2BAJAkhRmfCQjtiPTwkyQqxYEBGrWWeo)):
his stake plus the whole hoard, since he was the only hunter.

## 5. Re-run the simulation → see MECHANIC.md's tables

```sh
npm run simulate -- --seed 494     # about 13 minutes; writes sim/seed-494-hp300.json
npm run sim-tables                 # prints MECHANIC.md §3's tables from the files in sim/
```

300 hunters with six fixed strategies play 60 hunts against the committed `res/calydon.wasm` in a
local sandbox, once under rule B (launch) and once under rule A for comparison; every stake, refund
and total is checked against the simulator's own arithmetic, and the run fails on any mismatch.
Output of the run committed in `sim/`:

```text
Calydon simulation (v1.0): seed 494, 300 hunters, 60 hunts, first HP 3% of supply, HP x1.1/x0.7, burn 10%, launch hoard 1%, wasm 995d5961af67e36a40c1f604c394952514897b3fec8802c5609ae52d163554ec

Rule B (early-commit weighting, launch rule): kill rate 81.67% (49/60), supply 100000000 -> 96441630 CAL
  strategy                     hunters  joins  net % start  net/100 stk   supply share
  early committer                  114   4387       18.79%         2.01         23.17%
  threshold watcher                114   5905      -36.01%        -3.94        -33.65%
  quitter                           57   1581        9.33%         1.43         13.36%
  late whale (wait-and-fill)         7     24      -13.07%         -7.4         -9.86%
  solo whale                         4     11        5.41%         1.87          9.29%
  split whale                        4      9        5.89%         2.46          9.79%

Rule A (HP cap): kill rate 80.0% (48/60), supply 100000000 -> 96757234 CAL
...
Written: sim/seed-494-hp300.json (792.561016416s)
```

A re-run gives different numbers: inside a round the commits are sent together, the sandbox picks
the order they land in, and the fee depends on that order. In three runs of seed 494 the kill rate
and every rule-B sign held, but the top strategy was the early committers twice and the solo whale
once, and single results moved by up to 13 points (MECHANIC.md §3.3). Other options: `--hp0-bps`, `--burn-bps`, `--hoard-bps`,
`--hunters`, `--hunts`, `--set fill_fee_bps=0` (the same wasm without the fill fee), `--out <file>`.

## 6. Rebuild the wasm → see the same bytes as on testnet

```sh
npm run verify:reproducible
```

Needs Docker. It reads the source commit named inside `res/calydon.wasm` (NEP-330), builds that
commit with `git archive` in the pinned image
`docker.io/library/rust:1.99.1-slim-bookworm@sha256:392f2219…` (linux/amd64; on an arm64 host Docker
emulates it and the build runs one job at a time: 8 minutes on an Apple Silicon Mac), and
compares the result with `res/calydon.wasm`, `res/calydon.wasm.sha256` and the code hash deployed on
testnet. Expected output:

```text
building commit aa316382f0dbfd609af2cb6eae8443de73df1925 in docker.io/library/rust:1.99.1-slim-bookworm@sha256:392f221964669402bf4ae133d2a46c3ae270f64734133f4b2703d40019235c1a with CARGO_BUILD_JOBS=1
...
    Finished `release` profile [optimized] target(s) in 7m 38s
995d5961af67e36a40c1f604c394952514897b3fec8802c5609ae52d163554ec  .../target/reproducible/calydon.wasm
OK    rebuilt from commit aa316382f0dbfd609af2cb6eae8443de73df1925: byte-identical to res/calydon.wasm
      sha256 995d5961af67e36a40c1f604c394952514897b3fec8802c5609ae52d163554ec vs 995d5961af67e36a40c1f604c394952514897b3fec8802c5609ae52d163554ec
OK    equals res/calydon.wasm.sha256
      995d5961af67e36a40c1f604c394952514897b3fec8802c5609ae52d163554ec
OK    equals the code deployed on testnet at cal.calydon-v1-1794114512.testnet
      chain code_hash BKfwJS169ApsHRtyt34e5y1T64TqHGTFrhSEt57tM5kF, rebuilt BKfwJS169ApsHRtyt34e5y1T64TqHGTFrhSEt57tM5kF

Source, committed wasm and deployed code are the same bytes.
```

## Layout

| Path | What |
|---|---|
| `src/lib.rs` | The contract: NEP-141 + NEP-145 (near-contract-standards), `commit` / `resolve` / `claim`, the views, NEP-297 events. Pure arithmetic (`fill_fee`, `share`, `burn_of`) is public for the tests. |
| `src/tests.rs` | Unit tests, including the 20,000-case splitting property test. |
| `tests/verify.rs` | `npm run verify`: sandbox scenarios against `res/calydon.wasm`. |
| `tests/testnet.rs` | `npm run execute-testnet`: the recorded testnet run. |
| `tests/common/mod.rs` | Shared helpers; `launch_config()` is the one place the launch values are written. |
| `examples/simulate.rs` | `npm run simulate`. |
| `examples/hunt.rs` | `npm run hunt`. |
| `scripts/` | Reproducible build, `observe-testnet`, `sim-tables`, the status-page server, RPC helpers. |
| `status/index.html` | The local status page. |
| `res/calydon.wasm` | The reproducible build deployed on testnet, and its sha256. |
| `sim/`, `testnet/run.json` | Committed simulation results and the testnet record. |
| `docs/journey.md` | Dated decisions. `docs/day1*.md`: the overnight measurements (Korean). |

Libraries: near-sdk 5.30.1 and near-contract-standards 5.30.1 (contract); near-workspaces 0.24.0
(sandbox and testnet in the tests, the simulator and the CLI). The read-only scripts use Node's
standard library only.

## Limits

The watcher strategy loses by design; a whale that fills the whole HP in round 1 is not priced out
and was on top in some runs; the simulated strategies are fixed rules; re-runs vary; the splitting statements are integer arithmetic about stakes, not about who owns which
account; stored hunts cost the contract account NEAR; nothing here says anything about fun or
price. Details: [MECHANIC.md §5](MECHANIC.md#5-limits-stated-plainly). CAL exists on testnet only.

## How this was built

The code, tests, scripts and documents in this repository were written by an AI coding agent
(Claude, by Anthropic) working under the owner's direction; the owner chose the design and the
launch values ([docs/journey.md](docs/journey.md)). The test harness pattern (claims printed as
`HELD` only after they are asserted), the pinned reproducible build and the read-only observer
follow the owner's earlier project Préavis (github.com/ianlyoo/preavis): `scripts/near-rpc.mjs` is
copied from it, and `scripts/build-reproducible.sh` and `scripts/verify-reproducible.mjs` are adapted
from it. No contract code is shared.

## License

[MIT](LICENSE).

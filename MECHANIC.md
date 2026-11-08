# Calydon: the mechanic

Calydon (CAL) is an NEP-141 token with one game built into the contract: holders commit their own
tokens against a beast's HP before a deadline. If the community's total reaches the HP, the beast is
slain and the committers share the hoard. If it falls short, the beast escapes with 10% of every
stake, and that 10% burns. All numbers below are launch constants, fixed at `new` and readable through
`config()`; the contract has no owner, setter or pause, and the testnet instance has no access key.

## 1. Rules, and why each one exists

| Rule (contract) | Launch value | Why |
|---|---|---|
| `commit(amount)` locks your own CAL in the current hunt. | – | Joining costs something you hold. Nothing is minted for taking part, so creating extra accounts creates no extra reward (section 4). |
| Entry fee on top of the stake, paid into the hoard: each token pays `tribute_bps + fill_fee_bps × fill`, where fill is how full the bar was when that token went in (capped at a full bar; tokens past the HP pay the full 11%). Summed over the stake, rounded up once (`fill_fee` in `src/lib.rs`). | 1% + up to 10% | Answers the fill-when-certain problem (section 2): a stake that only arrives when the kill is already certain pays the most, and that fee goes to the hunters of the next kill. |
| Rule B, early weighting: a commit's weight is stake × the number of `step_blocks` steps left before the deadline. | 4 steps per hunt on testnet (300 blocks, steps of 75) | The same stake committed earlier, while the outcome is still open, gets a larger share. |
| `resolve(hunt)`, by anyone, once, after the deadline. Slain if committed ≥ HP (a tie is a kill). | – | No keeper or oracle; the game cannot stall waiting for one account. |
| Slain: each hunter claims stake + hoard × weight / total weight, rounded down. The rounding dust returns to the hoard after the last claim. | – | Shares can only round down, so the contract never pays out more than it holds. |
| Escaped: each hunter claims stake − 10% (rounded up); the 10% burns. The hoard, including the escaped hunt's entry fees, rolls over to the next hunt. | burn 10% | A failed hunt costs something, so committing is a real decision; supply falls only this way. |
| Next HP after a kill: × `hp_up_bps`; after an escape: × `hp_down_bps`. | ×1.1 / ×0.7 | Difficulty follows the community. The equilibrium kill rate is p with 1.1^p · 0.7^(1−p) = 1, p = ln(1/0.7) / (ln 1.1 + ln(1/0.7)) ≈ 79%. The earlier ×1.25 / ×0.8 pinned it at 50% (1.25 × 0.8 = 1), where every stake that can fail lost money on average. |
| Supply minted once at `new` (launch supply + launch hoard); never again. `storage_unregister` always refuses. | – | Supply can only fall, by escape burns. The standard unregister would burn a balance outside a hunt. |
| Plain NEP-141 transfers: no fee on `ft_transfer` / `ft_transfer_call`. | – | A transfer fee would make an `ft_transfer_call` receiver get less than the stated amount. |
| One `claim` per hunter per hunt; every call is O(1) (no loops over hunters). | – | Exactly-once settlement; no call grows with the number of hunters. |

## 2. The overnight finding: the fill-when-certain problem, and how R2 answers it

The first simulation (300 hunters, 60 hunts, the real wasm in a sandbox; day 1 in
[docs/day1.md](docs/day1.md)) used a flat 1% entry fee and the ×1.25 / ×0.8 controller. Two things
showed up together:

1. **The controller held the kill rate at about 50%.** With 10% burned on every escape, a stake that
   can fail loses about 4–5 per 100 committed on average. Early committers lost about 33% of their
   starting balance over 60 hunts.
2. **One path had no risk at all.** A whale that commits only when its own stake completes the HP
   never meets an escape, and takes a pro-rata share of the hoard every time. That strategy was on
   top in 11 of 12 measured cells. Splitting it over 8 accounts changed nothing except rounding.

Eight variants were measured before choosing (C, D, C+D, E, F, then R1, R2, R3;
[docs/journey.md](docs/journey.md)). The owner chose **R2 with rule B and the ×1.1 / ×0.7
controller**:

- The fill fee prices the risk-free path directly: a token committed into a full bar pays 11%, a token
  into an empty bar 1%. It does this per token and adds up exactly across parts, so cutting a stake into
  several commits or accounts pays the same fee before rounding and never less after it.
- The fee goes into the hoard, so late money funds the next kill instead of disappearing.
- Rule B adds a second reason to come early: weight is stake × steps left.
- ×1.1 / ×0.7 moves the equilibrium kill rate to about 79%, so an early stake fails less often.

The table in section 3.2 runs the same committed wasm with and without the fill fee to show the
difference on the same seeds.

## 3. Simulation

`npm run simulate -- --seed 494` runs the model below on a local NEAR sandbox against the committed
`res/calydon.wasm`, once with rule B (launch) and once with rule A (HP cap, for comparison). Every
stake, escaped refund and hunt total is checked against the simulator's own arithmetic, and the run
ends by checking I1 (conservation) and every account's balance against the chain; any mismatch fails
the run. `npm run sim-tables` prints the tables below from the files in `sim/`.

**Model.**
- Supply 100,000,000 CAL: 15 whales (5% of hunters) hold 30%, 285 other hunters 30%, a launch holder
  who never plays 39%, launch hoard 1%.
- 60 hunts, each 4 rounds of 25 blocks; within a round hunters act one by one in a seeded order and
  each sees what was committed before it. Hunt 0 is an empty setup hunt; hunt 1 opens at the first HP.
- Strategies (fixed rules, drawn once per seed):
  - **early**: joins with a seeded chance and commits 10–40% of its balance in round 1;
  - **watcher**: commits 10–40% once the bar reaches its own threshold (30–90% of HP);
  - **quitter**: like early, but sits out 3 hunts after an escape;
  - **late whale**: in the last round, fills the remaining HP alone if it can;
  - **solo whale**: in round 1, fills the remaining HP alone if it can;
  - **split whale**: a solo whale's twin (same holding) spread over 8 accounts, each with its own
    NEP-145 storage deposit (0.00125 NEAR, not counted in CAL).

The value per strategy is the net result over 60 hunts as a % of its starting balance.

### 3.1 Launch values (fill fee up to 10%, HP x1.1 / x0.7, burn 10%, launch hoard 1%)

Net result per strategy over 60 hunts, % of its starting balance. Rule A rows run the same values with the HP-cap rule, for comparison.

| Cell | Rule | Kill rate | early | watcher | quitter | late whale | solo whale | split whale | Top | Supply after 60 hunts (CAL) |
|---|---|---|---|---|---|---|---|---|---|---|
| seed 494, first HP 3% | B | 81.67% (49/60) | +18.79 | -36.01 | +9.33 | -13.07 | +5.41 | +5.89 | early | 96,441,630 |
| seed 494, first HP 3% | A | 80.00% (48/60) | -0.46 | -12.75 | -3.59 | -5.03 | +3.84 | -0.09 | solo whale | 96,757,234 |
| seed 494, first HP 1% | B | 85.00% (51/60) | +16.97 | -34.92 | +13.22 | -10.52 | +6.20 | +6.92 | early | 97,163,113 |
| seed 494, first HP 1% | A | 85.00% (51/60) | -2.57 | -9.90 | +0.95 | -3.68 | -1.43 | +8.54 | split whale | 97,655,093 |
| seed 494, first HP 9% | B | 76.67% (46/60) | +19.02 | -34.53 | +6.40 | -15.32 | +1.26 | +2.95 | early | 95,441,252 |
| seed 494, first HP 9% | A | 76.67% (46/60) | -1.23 | -10.48 | -8.07 | -5.94 | -0.10 | -0.29 | solo whale | 96,136,686 |
| seed 1, first HP 3% | B | 81.67% (49/60) | +25.81 | -35.13 | +10.91 | -18.95 | +0.85 | +6.16 | early | 96,214,493 |
| seed 1, first HP 3% | A | 81.67% (49/60) | +1.09 | -13.75 | -0.99 | -6.07 | -2.07 | +6.35 | split whale | 96,848,503 |
| seed 2, first HP 3% | B | 80.00% (48/60) | +15.66 | -45.63 | +7.24 | -9.92 | +9.30 | +0.92 | early | 95,352,895 |
| seed 2, first HP 3% | A | 80.00% (48/60) | +0.83 | -27.85 | -1.15 | -5.11 | +6.57 | -1.43 | solo whale | 95,560,773 |

### 3.2 The same wasm without the fill fee (`--set fill_fee_bps=0`)

| Cell | Rule | Kill rate | early | watcher | quitter | late whale | solo whale | split whale | Top | Supply after 60 hunts (CAL) |
|---|---|---|---|---|---|---|---|---|---|---|
| seed 494, first HP 3%, no fill fee | B | 80.00% (48/60) | -12.81 | -4.68 | -8.83 | -1.14 | +0.45 | +3.42 | split whale | 96,584,815 |
| seed 494, first HP 3%, no fill fee | A | 80.00% (48/60) | -15.23 | -5.57 | -11.11 | +0.12 | +0.27 | +7.00 | split whale | 96,581,332 |
| seed 1, first HP 3%, no fill fee | B | 81.67% (49/60) | -11.04 | -5.43 | -8.60 | -1.52 | +0.25 | +4.43 | split whale | 96,678,510 |
| seed 1, first HP 3%, no fill fee | A | 81.67% (49/60) | -15.01 | -6.57 | -11.99 | +0.07 | +0.25 | +11.58 | split whale | 96,720,878 |
| seed 2, first HP 3%, no fill fee | B | 80.00% (48/60) | -13.90 | -14.33 | -10.34 | -0.94 | +4.37 | +0.63 | solo whale | 95,281,137 |
| seed 2, first HP 3%, no fill fee | A | 80.00% (48/60) | -15.37 | -15.67 | -12.03 | +0.06 | +8.47 | +0.44 | solo whale | 95,295,984 |

### 3.3 Run-to-run variation: seed 494, first HP 3%, three runs

| Cell | Rule | Kill rate | early | watcher | quitter | late whale | solo whale | split whale | Top | Supply after 60 hunts (CAL) |
|---|---|---|---|---|---|---|---|---|---|---|
| run 1 | B | 81.67% (49/60) | +18.79 | -36.01 | +9.33 | -13.07 | +5.41 | +5.89 | early | 96,441,630 |
| run 1 | A | 80.00% (48/60) | -0.46 | -12.75 | -3.59 | -5.03 | +3.84 | -0.09 | solo whale | 96,757,234 |
| run 2 | B | 81.67% (49/60) | +10.92 | -36.77 | +4.85 | -9.75 | +18.21 | +2.46 | solo whale | 96,575,756 |
| run 2 | A | 80.00% (48/60) | -2.23 | -12.60 | -2.97 | -4.91 | +5.73 | -0.66 | solo whale | 96,748,970 |
| run 3 | B | 81.67% (49/60) | +18.74 | -35.76 | +9.44 | -13.31 | +6.11 | +5.38 | early | 96,454,795 |
| run 3 | A | 80.00% (48/60) | +0.38 | -13.54 | -1.38 | -5.80 | +3.73 | -0.67 | solo whale | 96,704,308 |

### 3.4 Other burn and launch hoard (not the launch values: `--burn-bps 300 --hoard-bps 500`)

| Cell | Rule | Kill rate | early | watcher | quitter | late whale | solo whale | split whale | Top | Supply after 60 hunts (CAL) |
|---|---|---|---|---|---|---|---|---|---|---|
| seed 494, first HP 3%, burn 3%, launch hoard 5% | B | 83.33% (50/60) | +18.02 | -30.88 | +14.35 | -5.01 | +56.91 | +0.69 | solo whale | 98,989,883 |
| seed 494, first HP 3%, burn 3%, launch hoard 5% | A | 81.67% (49/60) | +3.45 | -4.70 | +4.28 | -0.56 | +41.29 | -0.10 | solo whale | 98,965,225 |

**Reading the tables (facts only).**

- **Kill rate.** 76.67–85.00% in every cell, near the 79% equilibrium of ×1.1 / ×0.7. The three runs
  of seed 494 had the same kill rate (49/60 under rule B, 48/60 under rule A).
- **Launch values, rule B (§3.1, run 1 of each cell).** Early committers +15.66 to +25.81%, on top in
  5 of 5 cells; quitters +6.40 to +13.22%; solo whales +0.85 to +9.30%; split whales +0.92 to +6.92%;
  watchers −34.53 to −45.63%; late whales −9.92 to −18.95%. Supply after 60 hunts: 95,352,895 to
  97,163,113 CAL (2.8–4.6% burned).
- **Rule A with the same values.** Early committers −2.57 to +1.09%; a solo or split whale on top in
  every cell. Rule B is the launch rule.
- **Without the fill fee (§3.2).** Under rule B early committers −11.04 to −13.90%, watchers −4.68 to
  −14.33%, and a split or solo whale on top in every cell. On the same seeds the fill fee moves the
  early committers from −12.81 to +18.79% (seed 494), −11.04 to +25.81% (seed 1) and −13.90 to
  +15.66% (seed 2), and the watchers from −4.68 to −36.01%, −5.43 to −35.13% and −14.33 to −45.63%.
  The fee moves value from late committers to early ones; the kill rate barely changes.
- **Run-to-run variation (§3.3) is not small for every strategy.** Under rule B every strategy kept
  its sign in all three runs, but early committers ranged +10.92 to +18.79% and solo whales +5.41 to
  +18.21%, and the top strategy was the early committers in runs 1 and 3 and the solo whale in run 2.
  In run 2 the solo whales joined 23 hunts instead of 11. Under rule A the early committers' sign
  changed between runs (−2.23 to +0.38%).
- **Burn 3%, launch hoard 5% (§3.4, not the launch values).** The solo whale is on top under both
  rules (+56.91% / +41.29%): it joined 41 of 60 hunts, paid 11,986,382 CAL in entry fees and received
  17,459,591 CAL from hoards. The same cell measured at 08:20 on the `v-r2` branch (an earlier build
  of the same rules) had the early committers on top (+35.99%, solo whale +29.86%).
- **Solo vs split whale.** The split whale was above its solo twin in 4 of 5 launch cells under
  rule B. The two are separate hunters that act at different places in the seeded order and compete
  for the same remaining HP, so this compares positions in the order, not splitting. Splitting with the
  timing held fixed is §4.

## 4. Splitting a stake across accounts (I3), by arithmetic

Claim: with the hunt's outcome, its total weight, the hoard and the commit timing held fixed, a stake
cut into parts that land one after another, from one or several accounts, never receives more in
total than the same stake committed at once.

- The fill fee is an integral over the fill interval, so the parts' exact fees add up to the whole
  stake's exact fee; each part is rounded up, so the parts pay the same or more.
- Shares round down: floor(a) + floor(b) ≤ floor(a + b).
- Burns round up: ceil(a) + ceil(b) ≥ ceil(a + b).
- Weights are stake × steps, which add up exactly if the parts land in the same step.

Checked by a 20,000-case property test in `src/tests.rs` and on chain in `npm run verify`
(600 from one account vs 300 + 300 from two). What this does **not** say: parts that land in an earlier
step than the whole stake would get more weight, exactly as one account committing earlier would; each
extra account pays its own storage deposit in NEAR; and nothing here prevents one person from holding
many accounts.

## 5. Limits, stated plainly

- **The watcher strategy loses, by design.** Joining when the bar is already full costs more (up to 11%
  instead of 1%) and earns less weight under rule B. In every cell above the watcher's net result is
  negative; in every cell with the fill fee the late whale's is too.
- **A whale that fills the whole HP in round 1 is not priced out.** From an empty bar it pays the
  average fee over the fill it covers (1% + 10% × 0.5 = 6%), gets full rule-B weight, and the kill is
  certain from then on; everything committed after it in that hunt pays 11%. The fill fee makes a
  certain kill cost more the fuller the bar already is; it does not stop one large holder from making
  the kill certain early. That strategy was on top in run 2 of §3.3 and in §3.4.
- **The strategies are fixed rules.** They do not learn, react to each other or model people. Another
  strategy mix can give other winners; the tables show what these six rules do, nothing more.
- **Run-to-run variation.** Inside a round the commits are sent together and the sandbox picks the
  order they land in. Under R2 the fee depends on that order, and so does whether a solo whale finds
  an open bar in round 1, so the same seed gives different numbers each run (§3.3): the kill rate and
  every rule-B sign held across three runs, but the top strategy changed in one of them and single
  results moved by up to 13 points. The tables are three or fewer runs per cell, not averages.
- **Rounding.** All splitting statements are about integer arithmetic in yocto-CAL (18 decimals):
  shares round down, fees and burns round up, dust returns to the hoard. They say nothing about who
  owns which account.
- **Storage cost.** Hunt records and unclaimed entries are stored on the contract account and paid
  from its NEAR balance. Many tiny commits that are never claimed tie up that NEAR. There is no
  minimum stake.
- **Hunt 0 opens at `new`.** On a fresh launch the first hunt runs while the supply is still being
  distributed.
- **No claim about fun, demand or price.** The simulation measures token flows under fixed rules. It
  says nothing about whether people enjoy it or what CAL is worth; CAL exists on testnet only.

## 6. Compared with the organizers' streak-coin example

The track's example prompt is a streak coin: holders check in once a day; if the community hits the
day's target, everyone who checked in gets a small mint; if it misses, a slice of the treasury burns
and the streak resets. Calydon has the same skeleton (a community target, a reward on success, a burn
on failure, parameters fixed at launch with no admin key, a page with progress and a log, a 60-round
simulation), and it meets each requirement the prompt lists. It differs in three places. Taking part
costs a stake of your own tokens instead of a free check-in, so the reward comes from a hoard that
committers and late joiners paid into rather than from a mint; with no mint, supply only falls. The
target moves with the community (×1.1 after a kill, ×0.7 after an escape) instead of staying fixed.
And the sybil question is answered by arithmetic over stakes (section 4) rather than by limiting check-ins
per account, with the limits of that answer written above.

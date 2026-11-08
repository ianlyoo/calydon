# Calydon: the mechanic

Calydon (CAL) is an NEP-141 token with one game built into the contract: holders commit their own
tokens against a beast's HP before a deadline. If the community's total reaches the HP, the beast is
slain and the committers share the hoard. If it falls short, the beast escapes with 10% of every
stake, and that 10% burns. All numbers below are launch constants, fixed at `new` and readable through
`config()`; the contract has no owner, setter or pause, and the testnet instance never had an access
key (it was created, deployed and initialised in one transaction). This describes v1.1; v1.0 and its
testnet instance are superseded (changes: [docs/journey.md](docs/journey.md)).

## 1. Rules, and why each one exists

| Rule (contract) | Launch value | Why |
|---|---|---|
| `commit(amount)` locks your own CAL in the current hunt; at least `min_stake`. | min 1 CAL | Joining costs something you hold. Nothing is minted for taking part, so creating extra accounts creates no extra reward in a hunt (section 4; the starter below is the exception). |
| A commit that creates your entry in a hunt attaches `entry_deposit` in NEAR; a later commit to the same hunt attaches 1 yoctoNEAR. `claim` sends the deposit back. `resolve` and `starter` attach `record_deposit`, which stays (their records stay). Each of these calls checks that the storage it added costs no more than the NEAR attached, and refuses otherwise (`STORAGE_NOT_COVERED`). | 0.005 NEAR each (an entry is about 100 bytes = 0.001 NEAR) | Whoever adds a record pays for its storage, so commits cannot use up the contract's own NEAR and halt the game (section 5). |
| `starter()`: a registered account takes `starter_amount` CAL from the starter bucket, once. | 1,000 CAL; bucket 980,000 CAL (98% of supply) | How CAL reaches players without a holder handing it out. Demo-level: more accounts get more starters (section 5). |
| Entry fee on top of the stake, paid into the hoard: each token pays `tribute_bps + fill_fee_bps × fill`, where fill is how full the bar was when that token went in (capped at a full bar; tokens past the HP pay the full 11%). Summed over the stake, rounded up once (`fill_fee` in `src/lib.rs`). | 1% + up to 10% | Answers the fill-when-certain problem (section 2): a stake that only arrives when the kill is already certain pays the most, and that fee goes to the hunters of the next kill. |
| Rule B, early weighting: a commit's weight is stake × the number of `step_blocks` steps left before the deadline. | 4 steps per hunt on testnet (300 blocks, steps of 75) | The same stake committed earlier, while the outcome is still open, gets a larger share. |
| `resolve(hunt)`, by anyone, once, after the deadline. Slain if committed ≥ HP (a tie is a kill). | – | No keeper or oracle; the game cannot stall waiting for one account. |
| Slain: each hunter claims stake + hoard × weight / total weight, rounded down. The rounding dust returns to the hoard after the last claim. | – | Shares can only round down, so the contract never pays out more than it holds. |
| Escaped: each hunter claims stake − 10% (rounded up); the 10% burns. The hoard, including the escaped hunt's entry fees, rolls over to the next hunt. | burn 10% | A failed hunt costs something, so committing is a real decision; supply falls only this way. |
| A hunt nobody joins resolves as escaped, so the next HP is ×0.7 (never below 1 yocto-CAL). | – | With nobody playing, the HP keeps shrinking (×0.7 per empty hunt: 0.7^10 ≈ 2.8% after ten) until someone can fill it alone; nothing is burned or paid. |
| CAL sent to the contract with a plain `ft_transfer` is swept into the hoard by the next `resolve` (`sweep` event). | – | The contract's balance stays explained: held = hoard + locked + unclaimed + starter bucket after every resolve; between resolves the surplus is reported by `totals` and the observer. |
| First Spear: each hunt records the account whose commit came first (`first_spear` in `hunt` / `current_hunt`). | – | Something to be seen doing on the page. It carries no CAL or weight; it is a name in the hunt record. |
| Next HP after a kill: × `hp_up_bps`; after an escape: × `hp_down_bps`. | ×1.1 / ×0.7 | Difficulty follows the community. The equilibrium kill rate is p with 1.1^p · 0.7^(1−p) = 1, p = ln(1/0.7) / (ln 1.1 + ln(1/0.7)) ≈ 79%. The earlier ×1.25 / ×0.8 pinned it at 50% (1.25 × 0.8 = 1), where every stake that can fail lost money on average. |
| Supply minted once at `new`: launch holder, launch hoard and starter bucket; never again. `storage_unregister` always refuses. | holder 10,000 (1%), hoard 10,000 (1%), starter bucket 980,000 (98%) of 1,000,000 CAL on testnet | Supply can only fall, by escape burns. The holder's share is small and stated; the standard unregister would burn a balance outside a hunt. |
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
`res/calydon.wasm`, once with rule B (launch) and once with rule A (HP cap, for comparison).

**Oracle.** Every number the contract produces is compared with an independent computation in
`examples/simulate.rs` (big-integer arithmetic written separately from the contract's): for each
commit, in the order the commits landed, the bar's fill before it, the exact integrated entry fee,
the steps left and the weight, and what the account paid (stake + fee); for each hunt, the outcome,
the pot, the next HP and the hoard; for each claim, the exact payout (stake + pot × weight / total,
rounded down; or stake − the burn, rounded up) and the dust left after the last claim; at the end,
I1 and every account's balance. Any difference stops the run. The totals of what was checked are at
the end of the tables.

**Model.**
- Supply 100,000,000 CAL: 15 whales (5% of hunters) hold 30%, 285 other hunters 30%, a launch holder
  who never plays 39%, launch hoard 1%. (The simulation gives the hunters their CAL directly; the
  testnet instance uses the starter bucket instead.)
- 60 hunts, each 4 rounds of 100 blocks; within a round hunters decide one by one in a seeded order
  and each sees what was committed before it. Hunt 0 is an empty setup hunt; hunt 1 opens at the first
  HP. Inside a round the commits are sent together, first movers' first, and the sandbox picks the
  order they land in; the oracle follows the order they actually landed in.
- Strategies (fixed rules, drawn once per seed; the 285 others split 2 : 2 : 1 : 1 among early,
  watcher, quitter and first mover unless `--mix` says otherwise):
  - **early**: joins with a seeded chance and commits 10–40% of its balance in round 1;
  - **watcher**: commits 10–40% once the bar reaches its own threshold (30–90% of HP);
  - **quitter**: like early, but sits out 3 hunts after an escape;
  - **first mover**: the best response named in the v1.1 brief ("commit in round 1 as early as
    possible"): like early, but its round-1 commits are sent before everyone else's, so they land
    first, into the emptiest bar, at the lowest fee and with full weight;
  - **late whale**: in the last round, fills the remaining HP alone if it can;
  - **solo whale**: in round 1, fills the remaining HP alone if it can;
  - **split whale**: a solo whale's twin (same holding) spread over 8 accounts, each with its own
    NEP-145 storage deposit (0.00125 NEAR, not counted in CAL).
- Each account's first commit to a hunt attaches the 0.005 NEAR entry deposit; NEAR is not counted
  in the CAL results.

The value per strategy is the net result over 60 hunts as a % of its starting balance.

### 3.1 Launch values, 5 seeds (fill fee up to 10%, HP x1.1 / x0.7, burn 10%, launch hoard 1%, first HP 3%)

Net result per strategy over 60 hunts, % of its starting balance. Rule A rows run the same values with the HP-cap rule, for comparison.

| Cell | Rule | Kill rate | early | watcher | quitter | first mover | late whale | solo whale | split whale | Top | Supply after 60 hunts (CAL) |
|---|---|---|---|---|---|---|---|---|---|---|---|
| seed 494 | B | 81.67% (49/60) | +9.00 | -41.23 | +2.00 | +40.61 | -15.69 | +3.21 | +3.05 | first mover | 95,859,565 |
| seed 494 | A | 81.67% (49/60) | -3.40 | -20.76 | -5.48 | +15.53 | -7.94 | +4.44 | +0.69 | first mover | 96,255,120 |
| seed 1 | B | 81.67% (49/60) | +8.63 | -38.64 | -2.90 | +41.55 | -19.30 | +0.90 | +7.08 | first mover | 95,699,600 |
| seed 1 | A | 81.67% (49/60) | -8.59 | -15.19 | -8.45 | +15.13 | -7.89 | +0.45 | +6.73 | first mover | 96,386,020 |
| seed 2 | B | 80.00% (48/60) | -0.68 | -47.02 | -7.16 | +37.59 | -10.44 | +4.63 | +4.31 | first mover | 95,451,437 |
| seed 2 | A | 80.00% (48/60) | -9.99 | -23.11 | -9.80 | +13.15 | -4.05 | +5.26 | -1.61 | first mover | 95,750,968 |
| seed 1001 | B | 81.67% (49/60) | +4.63 | -40.41 | +2.21 | +45.32 | -15.46 | +5.37 | +2.04 | first mover | 95,905,464 |
| seed 1001 | A | 81.67% (49/60) | -5.11 | -18.01 | -6.28 | +17.76 | -7.44 | +4.33 | -0.73 | first mover | 96,335,941 |
| seed 1003 | B | 81.67% (49/60) | +12.44 | -36.26 | +2.40 | +47.36 | -22.53 | -0.11 | +1.12 | first mover | 95,675,169 |
| seed 1003 | A | 80.00% (48/60) | -6.89 | -19.24 | -5.45 | +16.75 | -7.93 | -0.70 | +0.47 | first mover | 95,692,076 |

Mean ± sample sd over the seeds above (n per rule shown):

| Rule | n | Kill rate % | early | watcher | quitter | first mover | late whale | solo whale | split whale | Top (count of seeds) |
|---|---|---|---|---|---|---|---|---|---|---|
| B | 5 | 81.34 ± 0.75 | +6.80 ± 5.02 | -40.71 ± 4.01 | -0.69 ± 4.24 | +42.49 ± 3.88 | -16.68 ± 4.54 | +2.80 ± 2.36 | +3.52 ± 2.32 | first mover 5 |
| A | 5 | 81.00 ± 0.91 | -6.80 ± 2.64 | -19.26 ± 2.97 | -7.09 ± 1.94 | +15.66 ± 1.75 | -7.05 ± 1.69 | +2.76 ± 2.69 | +1.11 ± 3.28 | first mover 5 |

### 3.2 Following the money: replicator runs (`npm run sim-converge`, rule B)

300 hunters, 20 hunts per generation; next share = share x max(0, 1 + net % of start / 100), renormalised to 1000. Mix in per mille (early / watcher / quitter / first mover); net % of start per strategy in that generation.

| Gen | Seed | Mix | Kill rate | early | watcher | quitter | first mover |
|---|---|---|---|---|---|---|---|
| 0 | 2000 | 250 / 250 / 250 / 250 | 85.00% | +4.55 | -12.89 | +1.76 | +15.93 |
| 1 | 2001 | 255 / 213 / 249 / 283 | 85.00% | +2.38 | -14.65 | +1.66 | +13.35 |
| 2 | 2002 | 257 / 179 / 249 / 315 | 85.00% | +0.97 | -13.99 | -1.64 | +14.45 |
| 3 | 2003 | 255 / 151 / 240 / 354 | 85.00% | -1.36 | -17.37 | -2.29 | +12.33 |
| 4 | 2004 | 249 / 124 / 233 / 394 | 85.00% | -1.63 | -14.33 | -2.28 | +11.76 |
| 5 | 2005 | 240 / 104 / 223 / 433 | 85.00% | -4.53 | -19.18 | -2.41 | +10.71 |
| 6 | 2006 | 227 / 83 / 215 / 475 | 85.00% | -2.38 | -15.06 | -2.10 | +9.81 |
| 7 | 2007 | 216 / 69 / 206 / 509 | 85.00% | -3.06 | -18.50 | -4.03 | +8.95 |
| 8 | 2008 | 206 / 55 / 194 / 545 | 85.00% | -7.36 | -19.24 | -5.82 | +7.29 |

Mix after the last generation: 190 / 44 / 182 / 584 (9 generations; the run was stopped before 10).

Oracle: 14 result files, 780 hunts, 139355 commits and 139355 claims checked exactly against the simulator's independent arithmetic; 0 mismatches.

**Reading the tables (facts only).**

- **Oracle.** 14 result files, 780 hunts, 139,355 commits and 139,355 claims: every fee, weight, pot,
  next HP, payout and final balance equalled the independent computation; 0 mismatches.
- **Kill rate.** 80.00–81.67% in every launch cell (mean 81.34 ± 0.75% under rule B), near the 79%
  equilibrium of ×1.1 / ×0.7; 85% (17 of 20) in every replicator generation.
- **Launch values, rule B (§3.1, 5 seeds).** The first mover was on top in 5 of 5 seeds
  (+42.49 ± 3.88%). Early committers whose round-1 commits land after the first movers' +6.80 ± 5.02%
  (−0.68 to +12.44), quitters −0.69 ± 4.24%, solo whales +2.80 ± 2.36%, split whales +3.52 ± 2.32%,
  late whales −16.68 ± 4.54%, watchers −40.71 ± 4.01%. Supply after 60 hunts 95,451,437 to 95,905,464
  CAL (4.1–4.5% burned).
- **Rule A with the same values.** The first mover was on top in 5 of 5 seeds (+15.66 ± 1.75%); early
  committers −6.80 ± 2.64%.
- **Following the money (§3.2).** With each generation's mix set by the previous generation's results,
  the first movers' share rose in every generation (250 → 584 per mille after 9 generations) and the
  watchers' fell in every generation (250 → 44). The early committers' share peaked at 257 (generation 2) and the quitters' stayed at 249–250
  until generation 2; both fell in every generation after that. As the first movers' share grew, their own result fell (+15.93% in generation 0 to +7.29% in
  generation 8) and the early committers' turned negative (from generation 3). The run was stopped
  after 9 of 10 generations at the deadline; it had not reached a fixed mix.
- **Solo vs split whale.** The split whale was above its solo twin in 2 of 5 launch seeds under rule B.
  They act at different places in the seeded order, so this compares positions, not splitting
  (splitting with the timing held fixed is §4).

### 3.3 Measured on the v1.0 wasm only

These cells ran on the v1.0 wasm (`995d5961…`, the same fee, weight and HP rules; no entry deposit, no
starter, no first-mover strategy) and were not re-run on v1.1 before the deadline. Their tables and
result files are at tag `v1.0` ([MECHANIC.md §3.2–3.4 at v1.0](https://github.com/ianlyoo/calydon/blob/v1.0/MECHANIC.md#3-simulation)):

- **Without the fill fee** (seeds 494, 1, 2): under rule B early committers −11.04 to −13.90% and a
  split or solo whale on top in every cell; with the fee the same seeds had early committers +15.66 to
  +25.81%. The fee moves value from late committers to early ones; the kill rate barely changed.
- **Three runs of seed 494**: the kill rate and every rule-B sign held; single results moved by up
  to 13 points and the top strategy changed in one run (the solo whale).
- **Burn 3%, launch hoard 5%**: the solo whale on top under both rules (+56.91% / +41.29%).

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
  the kill certain early. On the v1.0 wasm that strategy was on top in one of three runs of seed 494 and in the burn 3% / launch
  hoard 5% cell (§3.3).
- **Landing first pays most.** The first mover (round-1 commits that land before everyone else's) was
  on top in every launch cell under both rules (§3.1), and its share grew in every generation of the
  replicator runs (§3.2). Rule B and the fill fee reward the earliest stake in the order commits land;
  on a real chain that order is decided by when transactions reach the chain, so the game rewards
  committing as soon as a hunt opens.
- **The strategies are fixed rules.** They do not learn or model people. §3.2 lets the mix follow the
  money between generations, but each hunter still follows its rule inside a generation. Another
  strategy mix can give other winners; the tables show what these seven rules do, nothing more.
- **Run-to-run variation.** Inside a round the commits are sent together and the sandbox picks the
  order they land in. Under R2 the fee depends on that order, so the same seed gives different numbers
  each run (on the v1.0 wasm, single results moved by up to 13 points across three runs, §3.3). §3.1
  reports each seed once and the mean ± sample sd over the seeds; 5 seeds, fewer than the 10
  asked for, because each run took about 25–30 minutes.
- **Rounding.** All splitting statements are about integer arithmetic in yocto-CAL (18 decimals):
  shares round down, fees and burns round up, dust returns to the hoard. They say nothing about who
  owns which account.
- **Storage: the v1.0 halt, and the v1.1 fix.** In v1.0 every entry (about 97 bytes) and every hunt
  record was paid from the contract account's own NEAR, and the smallest commit was 1 yocto-CAL. The
  v1.0 instance had about 2.31 NEAR free, so about 2,400 one-yocto commits from fresh accounts would
  have used it up; from then on every call that adds storage (a first commit to a hunt, and
  `resolve`, which stores the next hunt) would fail, and the game would stop until someone sent the
  account NEAR (anyone can; nobody has to, and the code cannot be replaced). v1.1: a commit that creates an
  entry attaches 0.005 NEAR (returned by its claim), `resolve` and `starter` attach 0.005 NEAR, each
  call checks that its storage growth is covered and refuses otherwise, and a commit must be at least
  1 CAL. `npm run verify` shows it (`v1_1_storage_starter_sweep`, claims 2 and 3): an entry added 98
  bytes (0.00098 NEAR) under the 0.005 NEAR attached, the contract's free NEAR did not fall over
  commits, resolve and claims, and an instance configured with a 1-yocto entry deposit refuses the
  commit with `STORAGE_NOT_COVERED` instead of paying for it. What is left: an entry that is never claimed keeps its 0.005 NEAR on the
  contract, paid by the person who did not claim.
- **The starter is demo-level.** `starter()` gives 1,000 CAL once per account. Accounts are free on
  testnet (and cheap on mainnet), so whoever creates more accounts gets more starters, until the
  bucket (980 starters) is empty. It answers "how does CAL reach players" for a testnet demo; it is not
  a fair distribution and makes no claim to be one.
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

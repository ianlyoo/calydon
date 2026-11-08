# Journey: dated decisions

Times are KST (UTC+9). The design measurements behind each decision are in [day1.md](day1.md) and
[day1-variants.md](day1-variants.md) (Korean, written overnight for the owner). Variant branches
(`v-c`, `v-d`, `v-cd`, `v-e`, `v-f`, `v-r1`, `v-r3`) stayed local; their numbers are quoted in
those two files, and their raw JSON is not in this repository.

| When | Decision or event | Why |
|---|---|---|
| 2026-11-07 ~22:00 | Enter NEAR HACKATHON's *Memecoins With Mechanics* track with a new project, Calydon: a memecoin you hunt together. | A track/idea review chose a mechanic that is not the track's example prompt and whose quality can be shown with accounting tests and a simulation of the real contract. |
| 2026-11-07 23:27 | The owner agrees on a memecoin in game form. | |
| 2026-11-07 23:54 | v0: NEP-141 token with commit / resolve / claim; tribute taken on `commit`, not on transfers. | A fee on every transfer would make a receiver of `ft_transfer_call` get less than the stated amount; NEP-141 transfers stay standard. |
| 2026-11-07 23:59 | First testnet run: one slain and one escaped hunt (pre-v1.0 code, account not locked). | Gate: the hunt settles on a real chain. That instance is not the v1.0 instance. |
| 2026-11-08 00:16–01:42 | 300-hunter, 60-hunt simulation that runs the committed wasm in a local sandbox, rules A (HP cap) and B (early weighting). | Measure before choosing parameters. |
| 2026-11-08 01:27 | **Finding (G3): the fill-when-certain path.** A whale that commits only when its own stake completes the HP never meets an escape; it was on top in 11 of 12 cells, and committing early lost money (about −33% at ~50% kills). | A cooperative game where cooperating early loses is a defect. Reported, not chosen. |
| 2026-11-08 01:51–06:31 | Variants C (risk weight), D (escape loss half to the hoard), C+D and E (launch values only) measured on the same 12 cells. None met the criterion (early and watcher ≥ 0 everywhere, no strategy on top in more than 8 of 12). | Measured options for the owner instead of unverified ones. |
| 2026-11-08 06:46 | The HP controller ×1.25 / ×0.8 pins the kill rate at 50% (1.25 × 0.8 = 1). Variant F tries ×1.1 / ×0.7 (equilibrium ≈ 79%). | Every failed hunt burns 10%, so a 50% kill rate makes any stake that can fail negative in expectation. |
| 2026-11-08 07:18–08:20 | Simulator gets a **split whale** (a solo whale's twin spread over 8 accounts). Three rules aimed at the fill-when-certain path: R1 (weight by HP still missing), R2 (entry fee rising with the bar's fill, paid into the hoard), R3 (per-account share cap). | Any rule an account split can dodge must be tested against splitting. R3 can be dodged (split whale on top in 12/12; I3 fails) and was dropped. |
| 2026-11-08 08:20 | R2 under ×1.1 / ×0.7: early ≥ 0 in 9 of 12 cells and in 6 of 6 rule-B cells; watcher < 0 in all cells; no strategy on top in more than 6 of 12. | |
| 2026-11-08 13:31 | **Owner picks option ① (card #333): v1.0 = R2 + rule B + HP ×1.1 / ×0.7; an escape burns 10% (D's escape split is not used).** All are launch constants. | Measured: in R2 rule B, early committers are ≥ 0 and on top in 6/6 cells. The watcher loses by design: joining late costs more. |
| 2026-11-08 13:35–14:07 | v1.0 build: `v-r2` merged into main; `launch_config()` is the one source of the launch values; sandbox scenario for exactly those values; reproducible build in a pinned linux/amd64 image (NEP-330 metadata names the commit); CLI, observer, status page. | |
| 2026-11-08 14:00–15:00 | Simulation grid re-run on the v1.0 wasm: 5 launch-value cells, 3 cells without the fill fee, 2 repeats of seed 494, 1 cell with burn 3% / launch hoard 5% (MECHANIC.md §3). | The tables in MECHANIC.md must come from the committed wasm (`npm run sim-tables`). Result: rule B early committers on top in 5 of 5 launch cells (run 1), but in one of three repeat runs and in the burn 3% / hoard 5% cell a solo whale that fills the whole HP in round 1 was on top. Reported as a limit (MECHANIC.md §5); the launch values are not changed. |
| 2026-11-08 14:18 | v1.0 testnet instance `cal.calydon-v1-1794114512.testnet` launched with the launch values and locked (every access key deleted after `new`); hunts 0 slain, 1 escaped, 2 slain, each transaction recorded with its link in `testnet/run.json`. | Real hunts on a locked account; `npm run observe-testnet` re-checks them without a key. |
| 2026-11-08 14:30 | `npm run hunt -- launch` defaults to 600-block hunts (about 6 minutes), not 300. | Each CLI call took 30–55 s against the public RPC; in a 120-block test hunt a second hunter's commit came after the deadline (`HUNT_CLOSED`). |

## Evening notes

- 2026-11-07: idea chosen (a memecoin you hunt together), v0 contract written, first slain and escaped hunt on testnet (unlocked, pre-v1.0).
- 2026-11-08: v1.0 (R2 + rule B + ×1.1 / ×0.7) locked on testnet with three recorded hunts; reproducible build matches the chain; simulation tables and docs written from the committed wasm.

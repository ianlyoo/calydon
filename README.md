# Calydon (day-1 spike, private)

A memecoin you hunt together, on NEAR. Holders `commit` tokens against the beast's HP before a
deadline. Anyone can call `resolve` after the deadline, and then each hunter calls `claim` once.

- **Slain** (committed ≥ HP): each hunter gets back their stake plus a share of the hoard, rounded down. The next HP is ×1.25.
- **Escaped**: each hunter gets back their stake minus 10%. The 10% is rounded up and burns. The hoard rolls over, and the next HP is ×0.8.
- **Tribute**: `commit` charges 1% on top of the stake, and it goes into the hoard. `ft_transfer` and `ft_transfer_call` are plain NEP-141 with no fee.
- **Supply**: it is minted once at `new`. Nothing mints after that, so the supply only falls.
- **Parameters and admin**: every parameter is fixed at `new` and readable through `config`. There is no owner, setter or pause.
- **Reward rule**: chosen at launch.
  - **A, `HpCap`**: any stake past the HP is not taken, and shares are pro rata to stake.
  - **B, `EarlyWeight`**: there is no cap. The weight is stake × the number of steps left, so the same stake committed earlier earns a larger share.

| Command | Needs | What it does |
|---|---|---|
| `npm run verify` | Rust (rustup reads `rust-toolchain.toml`), Node, network on first run | Runs `cargo test --locked`: the 20,000-case split property test, plus the sandbox scenarios against `res/calydon.wasm` with I1 checked after every step |
| `npm run simulate -- --seed 494` | same | Runs the 300-hunter, 60-hunt strategy simulation under rules A and B against `res/calydon.wasm`, and writes `sim/seed-494-hp300.json` |
| `npm run execute-testnet` | network, testnet faucet | Runs one slain hunt and one escaped hunt on testnet, and writes `testnet/run.json` |
| `npm run build-wasm` | Rust | Rebuilds `res/calydon.wasm` and its sha256 (host build; it is not reproducible yet) |

Results and open questions: [docs/day1.md](docs/day1.md).

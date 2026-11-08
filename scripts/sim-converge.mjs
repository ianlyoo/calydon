// npm run sim-converge: repeated simulations where the strategy mix follows the money (replicator rule).
// Generation g plays `--hunts` hunts under rule B (the launch rule) with seed base+g; the next mix gives each
// of early / watcher / quitter / first mover a share proportional to share x (1 + net % of start / 100),
// floored at 0. Whales stay at 5% of hunters with their fixed strategies. Writes sim/converge/summary.json.
// Options: --gens 10 --hunts 20 --hunters 300 --seed 2000 --mix 250,250,250,250 (per mille).
import { spawnSync } from "node:child_process";
import { mkdirSync, readFileSync, writeFileSync } from "node:fs";

const argv = process.argv.slice(2);
const opt = (name, d) => (argv.includes(name) ? argv[argv.indexOf(name) + 1] : d);
const gens = Number(opt("--gens", 10)), hunts = opt("--hunts", "20"), hunters = opt("--hunters", "300");
const base = Number(opt("--seed", 2000));
const names = ["early committer", "threshold watcher", "quitter", "first mover (round 1, lands first)"];
let mix = opt("--mix", "250,250,250,250").split(",").map(Number);
const out = [];
mkdirSync("sim/converge", { recursive: true });
for (let g = 0; g < gens; g++) {
  const file = `converge/gen-${g}.json`;
  const args = ["run", "--locked", "--release", "--quiet", "--example", "simulate", "--", "--seed", String(base + g),
    "--hunters", hunters, "--hunts", hunts, "--rules", "B", "--mix", mix.join(","), "--out", file];
  const r = spawnSync("cargo", args, { stdio: ["ignore", "inherit", "inherit"] });
  if (r.status !== 0) process.exit(r.status ?? 1);
  const res = JSON.parse(readFileSync(`sim/${file}`, "utf8")).results[0];
  const net = names.map((n) => res.strategies.find((s) => s.strategy === n));
  const fit = net.map((s, i) => (mix[i] === 0 ? 0 : mix[i] * Math.max(0, 1 + s.net_pct_of_start / 100)));
  const sum = fit.reduce((a, b) => a + b, 0);
  out.push({ generation: g, seed: base + g, mix_per_mille: mix, kill_rate_pct: res.kill_rate_pct,
    net_pct_of_start: Object.fromEntries(names.map((n, i) => [n, net[i].net_pct_of_start])),
    hunters: Object.fromEntries(names.map((n, i) => [n, net[i].hunters])) });
  console.log(`gen ${g}: mix ${mix.join(",")} kill ${res.kill_rate_pct}% net ${net.map((s) => s.net_pct_of_start).join(" / ")}`);
  let next = fit.map((f) => Math.round((1000 * f) / sum));
  next[next.indexOf(Math.max(...next))] += 1000 - next.reduce((a, b) => a + b, 0);
  mix = next;
}
writeFileSync("sim/converge/summary.json", JSON.stringify({ rule: "B", hunts_per_generation: Number(hunts), hunters: Number(hunters),
  update: "share x max(0, 1 + net % of start / 100), renormalised to 1000", strategies: names, generations: out, final_mix_per_mille: mix }, null, 2) + "\n");
console.log(`final mix (early, watcher, quitter, first mover, per mille): ${mix.join(",")}`);

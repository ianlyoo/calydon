// npm run sim-tables: prints MECHANIC.md §3's simulation tables from the result files in sim/.
// Run `npm run simulate -- <args>` (and `npm run sim-converge`) first to regenerate a file, then this
// to compare with MECHANIC.md.
import { existsSync, readFileSync } from "node:fs";

const short = { "early committer": "early", "threshold watcher": "watcher", quitter: "quitter",
  "first mover (round 1, lands first)": "first mover", "late whale (wait-and-fill)": "late whale",
  "solo whale": "solo whale", "split whale": "split whale" };
const cols = Object.values(short);
const load = (f) => (existsSync(`sim/${f}`) ? JSON.parse(readFileSync(`sim/${f}`, "utf8")) : null);
const pct = (x) => (x == null ? "–" : `${x > 0 ? "+" : ""}${x.toFixed(2)}`);
const ruleOf = (r) => (r.rule.startsWith("B") ? "B" : "A");
const netOf = (r) => Object.fromEntries(r.strategies.map((s) => [short[s.strategy], s.net_pct_of_start]));
const top = (net) => cols.reduce((a, b) => ((net[b] ?? -Infinity) > (net[a] ?? -Infinity) ? b : a));
const oracle = { files: 0, commits: 0, claims: 0, hunts: 0, mismatches: 0 };

function results(file) {
  const d = load(file);
  if (!d) return null;
  oracle.files++;
  for (const r of d.results) {
    oracle.commits += r.oracle.commits_checked;
    oracle.claims += r.oracle.claims_checked;
    oracle.hunts += r.oracle.hunts_checked;
    oracle.mismatches += r.oracle.mismatches;
  }
  return d.results;
}
function rows(file, label) {
  const res = results(file);
  if (!res) return [`| ${label} | (sim/${file} missing) |`];
  return res.map((r) => {
    const net = netOf(r);
    return `| ${label} | ${ruleOf(r)} | ${r.kill_rate_pct.toFixed(2)}% (${r.kills}/${r.hunts}) | ${cols.map((c) => pct(net[c])).join(" | ")} | ${top(net)} | ${r.supply_end_cal.toLocaleString("en")} |`;
  });
}
const head = `| Cell | Rule | Kill rate | ${cols.join(" | ")} | Top | Supply after 60 hunts (CAL) |\n|${"---|".repeat(cols.length + 5)}`;
const meanSd = (xs) => {
  const m = xs.reduce((a, b) => a + b, 0) / xs.length;
  const sd = Math.sqrt(xs.reduce((a, b) => a + (b - m) ** 2, 0) / (xs.length - 1));
  return `${pct(m)} ± ${sd.toFixed(2)}`;
};

const LAUNCH = [494, 1, 2, 1001, 1002, 1003, 1004, 1005, 1006, 1007, 1008, 1009, 1010].filter((x) => existsSync(`sim/seed-${x}-hp300.json`));
console.log(`### 3.1 Launch values, ${LAUNCH.length} seeds (fill fee up to 10%, HP x1.1 / x0.7, burn 10%, launch hoard 1%, first HP 3%)\n`);
console.log("Net result per strategy over 60 hunts, % of its starting balance. Rule A rows run the same values with the HP-cap rule, for comparison.\n");
console.log(head);
const byRule = { B: [], A: [] };
for (const s of LAUNCH) {
  console.log(rows(`seed-${s}-hp300.json`, `seed ${s}`).join("\n"));
  for (const r of load(`seed-${s}-hp300.json`).results) byRule[ruleOf(r)].push(r);
}
console.log("\nMean ± sample sd over the seeds above (n per rule shown):\n");
console.log(`| Rule | n | Kill rate % | ${cols.join(" | ")} | Top (count of seeds) |\n|${"---|".repeat(cols.length + 4)}`);
for (const [rule, rs] of Object.entries(byRule)) {
  if (!rs.length) continue;
  const tops = {};
  for (const r of rs) tops[top(netOf(r))] = (tops[top(netOf(r))] ?? 0) + 1;
  const cells = cols.map((c) => {
    const xs = rs.map((r) => netOf(r)[c]).filter((x) => x != null);
    return xs.length > 1 ? meanSd(xs) : "–";
  });
  console.log(`| ${rule} | ${rs.length} | ${meanSd(rs.map((r) => r.kill_rate_pct)).replace(/^\+/, "")} | ${cells.join(" | ")} | ${Object.entries(tops).sort((a, b) => b[1] - a[1]).map(([k, v]) => `${k} ${v}`).join(", ")} |`);
}

// sim-converge rewrites summary.json after each generation; without one (a run from before v1.2), rebuild
// it from the generation files by replaying the same update rule from the default 250 x 4 start.
function convergeFromGens() {
  const names = ["early committer", "threshold watcher", "quitter", "first mover (round 1, lands first)"];
  let mix = [250, 250, 250, 250];
  const generations = [];
  for (let g = 0; existsSync(`sim/converge/gen-${g}.json`); g++) {
    const d = load(`converge/gen-${g}.json`), r = d.results[0];
    const net = names.map((n) => r.strategies.find((x) => x.strategy === n).net_pct_of_start);
    generations.push({ generation: g, seed: d.seed, mix_per_mille: mix, kill_rate_pct: r.kill_rate_pct,
      net_pct_of_start: Object.fromEntries(names.map((n, i) => [n, net[i]])) });
    const fit = net.map((x, i) => (mix[i] === 0 ? 0 : mix[i] * Math.max(0, 1 + x / 100)));
    const sum = fit.reduce((a, b) => a + b, 0);
    const next = fit.map((f) => Math.round((1000 * f) / sum));
    next[next.indexOf(Math.max(...next))] += 1000 - next.reduce((a, b) => a + b, 0);
    mix = next;
  }
  if (!generations.length) return null;
  const d0 = load("converge/gen-0.json");
  return { hunters: d0.hunters, hunts_per_generation: d0.results[0].hunts, partial: true,
    update: "share x max(0, 1 + net % of start / 100), renormalised to 1000", strategies: names, generations, final_mix_per_mille: mix };
}
const conv = load("converge/summary.json") ?? convergeFromGens();
if (conv) for (let g = 0; g < conv.generations.length; g++) results(`converge/gen-${g}.json`);
console.log("\n### 3.2 Following the money: replicator runs (`npm run sim-converge`, rule B)\n");
if (!conv) console.log("(sim/converge/summary.json missing)");
else {
  console.log(`${conv.hunters} hunters, ${conv.hunts_per_generation} hunts per generation; next share = ${conv.update}. Mix in per mille (early / watcher / quitter / first mover); net % of start per strategy in that generation.\n`);
  console.log("| Gen | Seed | Mix | Kill rate | early | watcher | quitter | first mover |\n|---|---|---|---|---|---|---|---|");
  for (const g of conv.generations) {
    const n = conv.strategies.map((s) => g.net_pct_of_start[s]);
    console.log(`| ${g.generation} | ${g.seed} | ${g.mix_per_mille.join(" / ")} | ${g.kill_rate_pct.toFixed(2)}% | ${n.map(pct).join(" | ")} |`);
  }
  const partial = conv.partial || conv.complete === false;
  console.log(`\nMix after the last generation: ${(conv.final_mix_per_mille ?? conv.next_mix_per_mille).join(" / ")}${partial ? ` (${conv.generations.length} generations; the run was stopped before 10)` : ""}.`);
}

console.log(`\nOracle: ${oracle.files} result files, ${oracle.hunts} hunts, ${oracle.commits} commits and ${oracle.claims} claims checked exactly against the simulator's independent arithmetic; ${oracle.mismatches} mismatches.`);

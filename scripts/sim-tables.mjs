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

const LAUNCH = [494, 1, 2, 1001, 1002, 1003, 1004, 1005, 1006, 1007, 1008, 1009, 1010];
console.log("### 3.1 Launch values, 13 seeds (fill fee up to 10%, HP x1.1 / x0.7, burn 10%, launch hoard 1%, first HP 3%)\n");
console.log("Net result per strategy over 60 hunts, % of its starting balance. Rule A rows run the same values with the HP-cap rule, for comparison.\n");
console.log(head);
const byRule = { B: [], A: [] };
for (const s of LAUNCH) {
  console.log(rows(`seed-${s}-hp300.json`, `seed ${s}`).join("\n"));
  for (const r of load(`seed-${s}-hp300.json`)?.results ?? []) byRule[ruleOf(r)].push(r);
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
  console.log(`| ${rule} | ${rs.length} | ${meanSd(rs.map((r) => r.kill_rate_pct))} | ${cells.join(" | ")} | ${Object.entries(tops).sort((a, b) => b[1] - a[1]).map(([k, v]) => `${k} ${v}`).join(", ")} |`);
}

console.log("\n### 3.2 Other first HP (seed 1001)\n");
console.log(head);
for (const [f, l] of [["seed-1001-hp100.json", "seed 1001, first HP 1%"], ["seed-1001-hp900.json", "seed 1001, first HP 9%"]])
  console.log(rows(f, l).join("\n"));

console.log("\n### 3.3 The same wasm without the fill fee (`--set fill_fee_bps=0`)\n");
console.log(head);
for (const s of [1001, 1002, 1003]) console.log(rows(`seed-${s}-hp300-fillfee0.json`, `seed ${s}, no fill fee`).join("\n"));

console.log("\n### 3.4 Other burn and launch hoard (not the launch values: `--burn-bps 300 --hoard-bps 500`)\n");
console.log(head);
console.log(rows("seed-1001-hp300-burn300-hoard500.json", "seed 1001, burn 3%, launch hoard 5%").join("\n"));

const conv = load("converge/summary.json");
console.log("\n### 3.5 Following the money: replicator runs (`npm run sim-converge`, rule B)\n");
if (!conv) console.log("(sim/converge/summary.json missing)");
else {
  console.log(`${conv.hunters} hunters, ${conv.hunts_per_generation} hunts per generation; next share = ${conv.update}. Mix in per mille (early / watcher / quitter / first mover); net % of start per strategy in that generation.\n`);
  console.log("| Gen | Seed | Mix | Kill rate | early | watcher | quitter | first mover |\n|---|---|---|---|---|---|---|---|");
  for (const g of conv.generations) {
    const n = conv.strategies.map((s) => g.net_pct_of_start[s]);
    console.log(`| ${g.generation} | ${g.seed} | ${g.mix_per_mille.join(" / ")} | ${g.kill_rate_pct.toFixed(2)}% | ${n.map(pct).join(" | ")} |`);
  }
  console.log(`\nMix after the last generation: ${conv.final_mix_per_mille.join(" / ")}.`);
}

console.log(`\nOracle: ${oracle.files} result files, ${oracle.hunts} hunts, ${oracle.commits} commits and ${oracle.claims} claims checked exactly against the simulator's independent arithmetic; ${oracle.mismatches} mismatches.`);

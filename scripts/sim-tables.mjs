// npm run sim-tables: prints MECHANIC.md's simulation tables from the result files in sim/.
// Run `npm run simulate -- <args>` first to regenerate a file, then this to compare with MECHANIC.md.
import { existsSync, readFileSync } from "node:fs";

const short = { "early committer": "early", "threshold watcher": "watcher", quitter: "quitter",
  "late whale (wait-and-fill)": "late whale", "solo whale": "solo whale", "split whale": "split whale" };
const cols = Object.values(short);
const load = (f) => (existsSync(`sim/${f}`) ? JSON.parse(readFileSync(`sim/${f}`, "utf8")) : null);
const pct = (x) => `${x > 0 ? "+" : ""}${x.toFixed(2)}`;
const ruleOf = (r) => (r.rule.startsWith("B") ? "B" : "A");

function rows(file, label) {
  const d = load(file);
  if (!d) return [`| ${label} | (sim/${file} missing) |`];
  return d.results.map((r) => {
    const net = Object.fromEntries(r.strategies.map((s) => [short[s.strategy], s.net_pct_of_start]));
    const top = cols.reduce((a, b) => (net[b] > net[a] ? b : a));
    return `| ${label} | ${ruleOf(r)} | ${r.kill_rate_pct.toFixed(2)}% (${r.kills}/${r.hunts}) | ${cols.map((c) => pct(net[c])).join(" | ")} | ${top} | ${r.supply_end_cal.toLocaleString("en")} |`;
  });
}
const head = `| Cell | Rule | Kill rate | ${cols.join(" | ")} | Top | Supply after 60 hunts (CAL) |\n|${"---|".repeat(cols.length + 5)}`;

console.log("### 3.1 Launch values (fill fee up to 10%, HP x1.1 / x0.7, burn 10%, launch hoard 1%)\n");
console.log("Net result per strategy over 60 hunts, % of its starting balance. Rule A rows run the same values with the HP-cap rule, for comparison.\n");
console.log(head);
for (const [f, l] of [["seed-494-hp300.json", "seed 494, first HP 3%"], ["seed-494-hp100.json", "seed 494, first HP 1%"],
  ["seed-494-hp900.json", "seed 494, first HP 9%"], ["seed-1-hp300.json", "seed 1, first HP 3%"], ["seed-2-hp300.json", "seed 2, first HP 3%"]])
  console.log(rows(f, l).join("\n"));

console.log("\n### 3.2 The same wasm without the fill fee (`--set fill_fee_bps=0`)\n");
console.log(head);
for (const s of [494, 1, 2]) console.log(rows(`seed-${s}-hp300-fillfee0.json`, `seed ${s}, first HP 3%, no fill fee`).join("\n"));

console.log("\n### 3.3 Run-to-run variation: seed 494, first HP 3%, three runs\n");
console.log(head);
for (const [f, l] of [["seed-494-hp300.json", "run 1"], ["repeat/seed-494-hp300-run2.json", "run 2"], ["repeat/seed-494-hp300-run3.json", "run 3"]])
  console.log(rows(f, l).join("\n"));

console.log("\n### 3.4 Other burn and launch hoard (not the launch values: `--burn-bps 300 --hoard-bps 500`)\n");
console.log(head);
console.log(rows("seed-494-hp300-burn300-hoard500.json", "seed 494, first HP 3%, burn 3%, launch hoard 5%").join("\n"));

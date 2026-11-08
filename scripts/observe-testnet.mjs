// npm run observe-testnet: read-only check of the v1.0 instance recorded in testnet/run.json.
// No key, no signing, no dependencies (Node >= 18). Exit code 0 only if every check holds.
//
// 1. The deployed code hash equals res/calydon.wasm (which `npm run verify:reproducible` rebuilds
//    byte for byte from source in a pinned Docker image) and res/calydon.wasm.sha256.
// 2. The contract account has no access key, so nobody can redeploy, sign for it or delete it.
// 3. config() equals the launch values recorded at init.
// 4. Every recorded transaction is on chain, succeeded and returned what the record says.
// 5. Current state: the hunt in progress, the hunt log, and the totals identity
//    held = hoard + locked + unclaimed, supply + burned = launched.
// Usage: node scripts/observe-testnet.mjs [testnet/run.json]
import { readFileSync } from "node:fs";
import { base58, query, rpc, sha256, view } from "./near-rpc.mjs";

const record = JSON.parse(readFileSync(process.argv[2] ?? "testnet/run.json", "utf8"));
const contract = record.contract;
const checks = [];
const check = (ok, label, detail) => {
  checks.push(ok);
  console.log(`${ok ? "OK  " : "FAIL"}  ${label}\n      ${detail}`);
};
const cal = (yocto) => {
  const v = BigInt(yocto);
  const frac = (v % 10n ** 18n).toString().padStart(18, "0").replace(/0+$/, "");
  return `${v / 10n ** 18n}${frac ? "." + frac.slice(0, 6) : ""}`;
};

console.log(`Calydon v1.0 on NEAR testnet: ${contract}\n`);
const wasm = readFileSync("res/calydon.wasm");
const account = await query({ request_type: "view_account", account_id: contract });
const recorded = readFileSync("res/calydon.wasm.sha256", "utf8").split(/\s+/)[0];
check(account.code_hash === base58(sha256(wasm)) && recorded === sha256(wasm).toString("hex"),
  "deployed code = res/calydon.wasm (the reproducible build; rebuild it with npm run verify:reproducible)",
  `chain code_hash ${account.code_hash}; res/calydon.wasm sha256 ${sha256(wasm).toString("hex")}`);
const meta = await view(contract, "contract_source_metadata", {});
console.log(`      NEP-330 on chain: ${meta.build_info?.source_code_snapshot}, image ${meta.build_info?.build_environment?.split("@")[0]}`);

const keys = (await query({ request_type: "view_access_key_list", account_id: contract })).keys;
check(keys.length === 0, "the contract account has 0 access keys (locked: no redeploy, no admin call)", `view_access_key_list: ${keys.length} keys`);

const config = await view(contract, "config", {});
check(JSON.stringify(config) === JSON.stringify(record.launch.config), "config() = the launch values recorded at init",
  `rule ${config.rule}, tribute ${config.tribute_bps} bps + fill fee up to ${config.fill_fee_bps} bps, burn ${config.burn_bps} bps, ` +
  `HP x${config.hp_up_bps / 1e4} slain / x${config.hp_down_bps / 1e4} escaped, hunts of ${config.hunt_blocks} blocks`);

let txOk = 0;
const bad = [];
for (const step of record.steps) {
  const tx = await rpc("tx", { tx_hash: step.tx, sender_account_id: step.signer, wait_until: "NONE" });
  const value = tx.status?.SuccessValue;
  const returned = value ? JSON.parse(Buffer.from(value, "base64").toString()) : null;
  if (value !== undefined && JSON.stringify(returned) === JSON.stringify(step.returned)) txOk++;
  else bad.push(`${step.step}: ${JSON.stringify(tx.status).slice(0, 120)}`);
}
check(bad.length === 0, `all ${record.steps.length} recorded transactions succeeded on chain with the recorded return values`,
  bad.length ? bad.join("\n      ") : `${txOk} of ${record.steps.length}, e.g. ${record.steps.find((s) => s.step.includes("resolve"))?.link}`);

const totals = await view(contract, "totals", {});
const launched = BigInt(record.launch.supply) + BigInt(record.launch.hoard);
const parts = BigInt(totals.hoard) + BigInt(totals.locked) + BigInt(totals.unclaimed);
check(BigInt(totals.held) === parts && BigInt(totals.supply) + BigInt(totals.burned) === launched,
  "totals: held = hoard + locked + unclaimed; supply + burned = launched",
  `supply ${cal(totals.supply)} CAL, burned ${cal(totals.burned)}, hoard ${cal(totals.hoard)}, locked ${cal(totals.locked)}, unclaimed ${cal(totals.unclaimed)}`);

const current = await view(contract, "current_hunt", {});
const height = (await rpc("block", { finality: "final" })).header.height;
console.log(`\nHunt log:`);
for (let id = 0; id <= current.id; id++) {
  const h = id === current.id ? current : await view(contract, "hunt", { id });
  const state = h.outcome ?? (height < h.deadline ? `open, ${h.deadline - height} blocks left` : "deadline passed, not resolved yet");
  console.log(`  hunt ${id}: HP ${cal(h.hp)} CAL, committed ${cal(h.committed)} by ${h.hunters}, ${state}${h.outcome === "Slain" ? `, pot ${cal(h.pot)} CAL` : ""}, claims ${h.claims}/${h.hunters}`);
}
const failed = checks.filter((ok) => !ok).length;
console.log(failed ? `\n${failed} check(s) FAILED.` : `\nAll ${checks.length} checks hold.`);
process.exit(failed ? 1 : 0);

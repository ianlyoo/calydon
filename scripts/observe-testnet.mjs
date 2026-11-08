// npm run observe-testnet: read-only check of the instance recorded in testnet/run.json (v1.2;
// testnet/v1.1/run.json and testnet/v1.0/run.json are the superseded records). No key, no signing, no dependencies
// (Node >= 18). Exit code 0 only if every check holds.
//
// 1. The deployed code hash equals res/calydon.wasm (which `npm run verify:reproducible` rebuilds
//    byte for byte from source in a pinned Docker image) and res/calydon.wasm.sha256. For an older
//    record, the wasm hash written in the record.
// 2. The contract account has no access key, so nobody can redeploy, sign for it or delete it; from
//    v1.1, the account was created, deployed and initialised in one transaction with no key added.
// 3. config() equals the launch values recorded at init.
// 4. Every recorded transaction is on chain, succeeded and returned what the record says; every
//    transaction recorded as refused failed with the recorded error.
// 5. Current state: the hunt in progress, the hunt log, and the totals: from v1.2,
//    held = hoard + locked + unclaimed + starter bucket (transfers to the contract are refused);
//    for v1.1, held >= that sum (a surplus was CAL sent by plain transfer, swept by the next
//    resolve; it is reported); and supply + burned = launched.
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

const v11 = record.launch.starter_bucket !== undefined; // v1.1 or later
const release = record.release ?? (v11 ? "v1.1" : "v1.0");
const exact = release !== "v1.0" && release !== "v1.1"; // v1.2+: held equals the parts exactly
console.log(`Calydon ${release}${release === "v1.2" ? "" : " (superseded)"} on NEAR testnet: ${contract}\n`);
const wasm = readFileSync("res/calydon.wasm");
const account = await query({ request_type: "view_account", account_id: contract });
const recorded = readFileSync("res/calydon.wasm.sha256", "utf8").split(/\s+/)[0];
if (record.wasm_sha256 === sha256(wasm).toString("hex")) {
  check(account.code_hash === base58(sha256(wasm)) && recorded === sha256(wasm).toString("hex"),
    "deployed code = res/calydon.wasm (the reproducible build; rebuild it with npm run verify:reproducible)",
    `chain code_hash ${account.code_hash}; res/calydon.wasm sha256 ${sha256(wasm).toString("hex")}`);
} else {
  check(account.code_hash === base58(Buffer.from(record.wasm_sha256, "hex")),
    "deployed code = the wasm hash in this record (an older release; res/calydon.wasm is newer)",
    `chain code_hash ${account.code_hash}; record wasm_sha256 ${record.wasm_sha256}`);
}
const meta = await view(contract, "contract_source_metadata", {});
console.log(`      NEP-330 on chain: ${meta.build_info?.source_code_snapshot}, image ${meta.build_info?.build_environment?.split("@")[0]}`);

const keys = (await query({ request_type: "view_access_key_list", account_id: contract })).keys;
if (v11) {
  const init = await rpc("tx", { tx_hash: record.steps[0].tx, sender_account_id: record.steps[0].signer, wait_until: "NONE" });
  const actions = init.transaction.actions.map((a) => (typeof a === "string" ? a : Object.keys(a)[0] + (a.FunctionCall ? `(${a.FunctionCall.method_name})` : "")));
  const one = init.transaction.receiver_id === contract && actions.join(",") === "CreateAccount,Transfer,DeployContract,FunctionCall(new)";
  check(keys.length === 0 && one, "the contract account was created, deployed and initialised in one transaction with no key, and has 0 access keys now (no redeploy, no admin call)",
    `init tx ${record.steps[0].tx} actions: ${actions.join(", ")}; view_access_key_list: ${keys.length} keys`);
} else {
  check(keys.length === 0, "the contract account has 0 access keys (locked: no redeploy, no admin call)", `view_access_key_list: ${keys.length} keys`);
}

const config = await view(contract, "config", {});
check(JSON.stringify(config) === JSON.stringify(record.launch.config), "config() = the launch values recorded at init",
  `rule ${config.rule}, tribute ${config.tribute_bps} bps + fill fee up to ${config.fill_fee_bps} bps, burn ${config.burn_bps} bps, ` +
  `HP x${config.hp_up_bps / 1e4} slain / x${config.hp_down_bps / 1e4} escaped, hunts of ${config.hunt_blocks} blocks` +
  (config.starters_per_hunt ? `, fill fee from step 2 of ${config.step_blocks} blocks, ${config.starters_per_hunt} starters per hunt` : ""));

let txOk = 0;
const bad = [];
for (const step of record.steps) {
  const tx = await rpc("tx", { tx_hash: step.tx, sender_account_id: step.signer, wait_until: "NONE" });
  const value = tx.status?.SuccessValue;
  const returned = value ? JSON.parse(Buffer.from(value, "base64").toString()) : null;
  if (value !== undefined && JSON.stringify(returned) === JSON.stringify(step.returned)) txOk++;
  else bad.push(`${step.step}: ${JSON.stringify(tx.status).slice(0, 120)}`);
}
const refused = record.refused ?? [];
for (const step of refused) {
  const tx = await rpc("tx", { tx_hash: step.tx, sender_account_id: step.signer, wait_until: "NONE" });
  if (!JSON.stringify(tx.status?.Failure ?? "").includes(step.error)) bad.push(`${step.step}: expected ${step.error}, got ${JSON.stringify(tx.status).slice(0, 120)}`);
}
check(bad.length === 0, `all ${record.steps.length} recorded transactions succeeded on chain with the recorded return values` +
  (refused.length ? `; the ${refused.length} recorded as refused failed with the recorded error` : ""),
  bad.length ? bad.join("\n      ") : `${txOk} of ${record.steps.length}, e.g. ${record.steps.find((s) => s.step.includes("resolve"))?.link}` +
    refused.map((r) => `\n      refused: ${r.step}: ${r.error}, ${r.link}`).join(""));

const totals = await view(contract, "totals", {});
const launched = v11
  ? BigInt(record.launch.holder_supply) + BigInt(record.launch.hoard) + BigInt(record.launch.starter_bucket)
  : BigInt(record.launch.supply) + BigInt(record.launch.hoard);
const parts = BigInt(totals.hoard) + BigInt(totals.locked) + BigInt(totals.unclaimed) + BigInt(totals.starter_left ?? 0);
const surplus = BigInt(totals.held) - parts;
check(surplus >= 0n && (!exact && v11 || surplus === 0n) && BigInt(totals.supply) + BigInt(totals.burned) === launched,
  exact ? "totals: held = hoard + locked + unclaimed + starter bucket; supply + burned = launched"
    : v11 ? "totals: held >= hoard + locked + unclaimed + starter bucket (surplus reported); supply + burned = launched"
    : "totals: held = hoard + locked + unclaimed; supply + burned = launched",
  `supply ${cal(totals.supply)} CAL, burned ${cal(totals.burned)}, hoard ${cal(totals.hoard)}, locked ${cal(totals.locked)}, unclaimed ${cal(totals.unclaimed)}` +
  (exact ? `, starter bucket ${cal(totals.starter_left)}; held ${cal(totals.held)}` : "") +
  (v11 && !exact ? `, starter bucket ${cal(totals.starter_left)}; surplus ${cal(surplus)} CAL` +
    (surplus > 0n ? " (sent by plain transfer since the last resolve; the next resolve sweeps it into the hoard)" : "") : ""));

const current = await view(contract, "current_hunt", {});
const height = (await rpc("block", { finality: "final" })).header.height;
console.log(`\nHunt log:`);
for (let id = 0; id <= current.id; id++) {
  const h = id === current.id ? current : await view(contract, "hunt", { id });
  const state = h.outcome ?? (height < h.deadline ? `open, ${h.deadline - height} blocks left` : "closed (unresolved)");
  console.log(`  hunt ${id}: HP ${cal(h.hp)} CAL, committed ${cal(h.committed)} by ${h.hunters}, ${state}${h.outcome === "Slain" ? `, pot ${cal(h.pot)} CAL` : ""}, claims ${h.claims}/${h.hunters}${h.first_spear ? `, First Spear ${h.first_spear}` : ""}`);
}
const failed = checks.filter((ok) => !ok).length;
console.log(failed ? `\n${failed} check(s) FAILED.` : `\nAll ${checks.length} checks hold.`);
process.exit(failed ? 1 : 0);

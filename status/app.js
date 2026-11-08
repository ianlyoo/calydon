// Calydon page: reads the contract over the public testnet RPC and signs through NEAR wallet-selector
// (Intear Wallet module, pinned and bundled in vendor/wallet-selector.js). Static: no server code, no key here.
// MyNearWallet is not offered: since 1 Nov 2026 it no longer connects to apps or signs.
import { setupWalletSelector, setupIntearWallet, actionCreators } from "./vendor/wallet-selector.js";

const params = new URLSearchParams(location.search);
const RPC = params.get("rpc") || "https://test.rpc.fastnear.com";
const TGAS = 10n ** 12n;
const $ = (id) => document.getElementById(id);

async function rpc(method, p) {
  const r = await fetch(RPC, { method: "POST", headers: { "content-type": "application/json" },
    body: JSON.stringify({ jsonrpc: "2.0", id: 1, method, params: p }) });
  const body = await r.json();
  if (body.error) throw new Error(JSON.stringify(body.error));
  return body.result;
}
async function view(method, args) {
  const r = await rpc("query", { finality: "optimistic", request_type: "call_function", account_id: contract,
    method_name: method, args_base64: btoa(JSON.stringify(args)) });
  return JSON.parse(new TextDecoder().decode(new Uint8Array(r.result)));
}

// The contract's arithmetic (src/lib.rs), in BigInt.
const ceilDiv = (n, d) => (n + d - 1n) / d;
const ceilBps = (x, bps) => ceilDiv(x * BigInt(bps), 10000n);
// Fee on `stake` landing at fill `before` of `hp`: (tribute + fill_fee x min(x, hp)/hp) bps per token,
// integrated over the tokens the stake covers and rounded up once.
function fillFee(before, stake, hp, t, f) {
  const min = (a, b) => (a < b ? a : b);
  const a = min(before, hp), b = min(before + stake, hp), above = stake - (b - a);
  const num = BigInt(t) * stake * hp * 2n + BigInt(f) * (b * b - a * a) + BigInt(f) * hp * above * 2n;
  return ceilDiv(num, hp * 20000n);
}
const share = (pot, w, total) => (total === 0n ? 0n : (pot * w) / total);

const C = 10n ** 18n;
function cal(y, digits = 4) {
  const v = BigInt(y), neg = v < 0n, a = neg ? -v : v;
  const f = (a % C).toString().padStart(18, "0").slice(0, digits).replace(/0+$/, "");
  return `${neg ? "−" : ""}${(a / C).toLocaleString("en")}${f ? "." + f : ""} CAL`;
}
const near = (y) => `${(Number(BigInt(y) / 10n ** 18n) / 1e6).toString()} NEAR`;
function parseCal(text) {
  const m = /^\s*(\d*)(?:\.(\d{0,18}))?\s*$/.exec(text);
  if (!m || (m[1] === "" && !m[2])) return null;
  return BigInt(m[1] || "0") * C + BigInt((m[2] || "").padEnd(18, "0"));
}

const ERRORS = {
  HUNT_CLOSED: "the hunt's deadline has passed; resolve it to open the next one",
  BELOW_MIN_STAKE: "below the minimum stake", ATTACH_ENTRY_DEPOSIT: "a first commit to a hunt must attach the entry deposit",
  ATTACH_ONE_YOCTO: "a further commit to the same hunt attaches 1 yoctoNEAR", NOT_REGISTERED: "register (storage_deposit) first",
  STARTER_TAKEN: "this account already took its starter", STARTER_EMPTY: "the starter bucket is empty",
  BEFORE_DEADLINE: "the deadline has not passed yet", ALREADY_RESOLVED: "already resolved", NOTHING_TO_CLAIM: "nothing to claim",
  NOT_RESOLVED: "not resolved yet", "The account doesn't have enough balance": "not enough CAL for stake + fee",
};
function explain(e) {
  const text = e?.message || String(e);
  for (const [k, v] of Object.entries(ERRORS)) if (text.includes(k)) return `${v} (${k})`;
  return text;
}
function say(text, err = false) {
  $("msg").textContent = text;
  $("msg").className = err ? "err" : "muted";
}

let contract, selector, me = null, cfg, state = {};

async function send(method, args, deposit, label) {
  const wallet = await selector.wallet();
  say(`${label}: waiting for the wallet…`);
  try {
    const out = await wallet.signAndSendTransaction({ receiverId: contract,
      actions: [actionCreators.functionCall(method, args, 100n * TGAS, BigInt(deposit))] });
    const failure = out?.status?.Failure;
    if (failure) throw new Error(JSON.stringify(failure));
    const hash = out?.transaction?.hash || out?.transaction_outcome?.id || "";
    say(`${label}: done. https://testnet.nearblocks.io/txns/${hash}`);
  } catch (e) {
    say(`${label}: refused: ${explain(e)}`, true);
  }
  await refresh();
}

// The preview works signed out; the Commit button needs a signed-in, registered account and an open hunt.
function gate() {
  $("commit").disabled = !(me && state.reg && state.open);
  $("commitHint").textContent = !me ? "Sign in to commit." : !state.reg ? "Register with storage_deposit to commit." : state.open ? "" : "This hunt is closed; resolve it to open the next one.";
}

function preview() {
  const h = state.hunt;
  if (!h || !cfg) return;
  const stake = parseCal($("stake").value);
  const rows = [];
  let note = "";
  if (h.deadline - state.height <= 0) {
    rows.push(["", `hunt #${h.id} is closed: commits are refused until someone resolves it, which opens hunt #${h.id + 1}`]);
  } else if (stake === null || stake <= 0n) {
    rows.push(["", "enter an amount of CAL"]);
  } else {
    const hp = BigInt(h.hp), before = BigInt(h.committed);
    const fee = fillFee(before, stake, hp, cfg.tribute_bps, cfg.fill_fee_bps);
    const left = h.deadline - state.height;
    const steps = cfg.rule === "EarlyWeight" ? BigInt(Math.max(0, Math.ceil(left / cfg.step_blocks))) : 1n;
    const weight = stake * steps;
    const fresh = !state.myEntry;
    // If slain with no further commits: the pot is today's hoard plus this fee; yours is your weight's share.
    const total = BigInt(h.weight) + weight;
    const pot = BigInt(state.totals.hoard) + fee;
    const myW = weight + BigInt(state.myEntry?.weight || 0);
    const myStake = stake + BigInt(state.myEntry?.stake || 0);
    const slainNow = before + stake >= hp;
    const ifSlain = myStake + share(pot, myW, total);
    const ifEscaped = myStake - ceilBps(myStake, cfg.burn_bps);
    const paid = myStake + fee; // earlier commits' fees in this hunt are not known to the page
    rows.push(["You pay", `${cal(stake + fee)} = stake ${cal(stake)} + entry fee ${cal(fee)} (${(Number((fee * 1000000n) / stake) / 10000).toFixed(4)}% of the stake, the fee integrated over the fill from ${cal(before, 2)} to ${cal(before + stake, 2)})`]);
    rows.push(["NEAR attached", fresh ? `${near(cfg.entry_deposit)} entry deposit (your claim sends it back)` : "1 yoctoNEAR (you already have an entry in this hunt)"]);
    rows.push(["Weight", cfg.rule === "EarlyWeight" ? `${cal(stake, 2).replace(" CAL", "")} × ${steps} step(s) left = ${cal(weight, 2).replace(" CAL", "")}` : cal(weight)]);
    rows.push(["Bar after", `${cal(before + stake, 2)} of ${cal(hp, 2)}${slainNow ? " (at or past HP: slain if nothing changes)" : ` (${cal(hp - before - stake, 2)} still missing)`}`]);
    rows.push(["If slain", `claim ${cal(ifSlain)}: your stake back + ${cal(ifSlain - myStake)} (your weight's share of a ${cal(pot)} pot if nobody else commits; each later commit adds its fee to the pot and its weight to the total)`]);
    rows.push(["If escaped", `claim ${cal(ifEscaped)}: your stake minus ${cfg.burn_bps / 100}% burned; your fee stays in the hoard`]);
    rows.push(["Net", `slain: ${cal(ifSlain - paid)} · escaped: ${cal(ifEscaped - paid)} (claim minus stake minus this fee${fresh ? "" : "; your earlier fees in this hunt not counted"})`]);
    if (cfg.rule === "EarlyWeight" && left > 0) {
      const stepEnd = h.deadline - (Number(steps) - 1) * cfg.step_blocks;
      note = `Weight counts the step your commit lands in: ${steps} step(s) until block ${stepEnd} (${stepEnd - state.height} blocks), then ${Number(steps) - 1}.`;
    }
  }
  $("preview").innerHTML = rows.map(([k, v]) => `<tr><th>${k}</th><td style="text-align:left">${v}</td></tr>`).join("");
  $("previewNote").textContent = note;
}

const hunts = new Map();
async function refresh() {
  try {
    const [h, t, keys, block] = await Promise.all([view("current_hunt", {}), view("totals", {}),
      rpc("query", { finality: "optimistic", request_type: "view_access_key_list", account_id: contract }), rpc("block", { finality: "optimistic" })]);
    state.hunt = h; state.totals = t; state.height = block.header.height;
    const hp = BigInt(h.hp), committed = BigInt(h.committed);
    const pct = Math.min(100, Number((committed * 10000n) / hp) / 100);
    $("title").textContent = `Current hunt: #${h.id}`;
    $("fill").style.width = pct + "%";
    $("barText").textContent = `${pct.toFixed(1)}% of HP`;
    const left = h.deadline - state.height;
    const secs = Math.round(left * (state.blockTime || 1));
    const verdict = committed >= hp && committed > 0n ? "Slain" : "Escaped";
    $("deadline").className = left <= 0 ? "closed" : left < 60 ? "soon" : "";
    $("deadline").textContent = left > 0
      ? `Open: ${left} blocks left (deadline block ${h.deadline}, about ${secs < 120 ? secs + " s" : Math.round(secs / 60) + " min"}). If it closed now: ${verdict}.`
      : `Closed at block ${h.deadline}: commits are refused. Anyone can resolve it now (${verdict}); that opens hunt #${h.id + 1}.`;
    $("committed").textContent = `${cal(h.committed)} by ${h.hunters} hunter(s)`;
    $("hp").textContent = cal(h.hp);
    $("spear").textContent = h.first_spear ?? "nobody yet (the first commit of a hunt takes it)";
    $("hoard").textContent = cal(t.hoard);
    $("supply").textContent = cal(t.supply);
    $("burned").textContent = cal(t.burned);
    $("bucket").textContent = cal(t.starter_left, 0);
    $("keys").textContent = keys.keys.length === 0 ? "0 (nobody can change or redeploy the code)" : String(keys.keys.length);
    $("resolve").hidden = !(me && left <= 0);
    state.open = left > 0;
    gate();
    for (let id = Math.max(0, h.id - 49); id < h.id; id++)
      if (!hunts.has(id) || hunts.get(id).claims < hunts.get(id).hunters) hunts.set(id, await view("hunt", { id }));
    hunts.set(h.id, h);
    $("log").innerHTML = [...hunts.values()].sort((a, b) => b.id - a.id).map((x) => `<tr><td>#${x.id}</td><td>${cal(x.hp, 2)}</td><td>${cal(x.committed, 2)}</td>
      <td>${x.hunters}</td><td class="${x.outcome ?? ""}">${x.outcome ?? "open"}</td><td>${x.outcome === "Slain" ? cal(x.pot, 2) : "–"}</td>
      <td>${x.claims}/${x.hunters}</td><td class="who">${x.first_spear ?? "–"}</td></tr>`).join("");
    if (me) await refreshMe(h);
    preview();
    $("updated").textContent = `block ${state.height}, ${new Date().toLocaleTimeString()}`;
  } catch (e) {
    $("updated").textContent = "RPC error: " + e.message;
  }
}

async function refreshMe(h) {
  const [bal, reg, taken] = await Promise.all([view("ft_balance_of", { account_id: me }),
    view("storage_balance_of", { account_id: me }), view("starter_taken", { account_id: me })]);
  $("myBal").textContent = cal(bal);
  $("myReg").textContent = reg ? "yes" : "no";
  $("register").hidden = !!reg;
  $("myStarter").textContent = taken ? "taken" : BigInt(state.totals.starter_left) > 0n ? `available: ${cal(cfg.starter_amount, 0)}` : "bucket empty";
  $("starter").hidden = !!taken || !reg || BigInt(state.totals.starter_left) === 0n;
  state.reg = !!reg;
  gate();
  const rows = [];
  state.myEntry = null;
  for (let id = Math.max(0, h.id - 49); id <= h.id; id++) {
    const e = await view("entry", { hunt: id, account_id: me });
    if (!e) continue;
    const x = id === h.id ? h : hunts.get(id);
    if (id === h.id) state.myEntry = e;
    let pays = "–", button = "";
    if (x.outcome === "Slain") pays = cal(BigInt(e.stake) + share(BigInt(x.pot), BigInt(e.weight), BigInt(x.weight)));
    if (x.outcome === "Escaped") pays = cal(BigInt(e.stake) - ceilBps(BigInt(e.stake), cfg.burn_bps));
    if (x.outcome) button = `<button data-claim="${id}">Claim</button>`;
    rows.push(`<tr><td>#${id}</td><td>${cal(e.stake, 2)}</td><td>${cal(e.weight, 2).replace(" CAL", "")}</td><td class="${x.outcome ?? ""}">${x.outcome ?? "open"}</td><td>${pays}</td><td>${button}</td></tr>`);
  }
  $("entriesPanel").hidden = rows.length === 0;
  $("entries").innerHTML = rows.reverse().join("");
  for (const b of document.querySelectorAll("[data-claim]"))
    b.onclick = () => send("claim", { hunt: Number(b.dataset.claim) }, 0n, `Claim hunt #${b.dataset.claim}`);
}

async function showAccount() {
  const wallet = selector.isSignedIn() ? await selector.wallet() : null;
  const accounts = wallet ? await wallet.getAccounts() : [];
  me = accounts[0]?.accountId || null;
  $("signedOut").hidden = !!me;
  $("signedIn").hidden = !me;
  $("me").textContent = me || "";
  if (!me) { state.reg = false; $("entriesPanel").hidden = true; $("resolve").hidden = true; }
  gate();
}

async function blockTime() {
  const now = await rpc("block", { finality: "final" });
  const then = await rpc("block", { block_id: now.header.height - 100 }).catch(() => null);
  if (then) state.blockTime = (Number(now.header.timestamp) - Number(then.header.timestamp)) / 1e9 / (now.header.height - then.header.height);
}

(async () => {
  contract = params.get("contract") || (await (await fetch("../testnet/run.json")).json()).contract;
  $("contract").textContent = contract;
  cfg = await view("config", {});
  selector = await setupWalletSelector({ network: "testnet", modules: [setupIntearWallet()] });
  await showAccount();
  $("signIn").onclick = async () => {
    try {
      const wallet = await selector.wallet("intear-wallet");
      await wallet.signIn({ contractId: contract, methodNames: [] });
      await showAccount();
      say("Signed in.");
      await refresh();
    } catch (e) { say("Sign-in: " + explain(e), true); }
  };
  $("signOut").onclick = async () => { await (await selector.wallet()).signOut(); await showAccount(); say(""); };
  $("register").onclick = async () => {
    const bounds = await view("storage_balance_bounds", {});
    send("storage_deposit", { account_id: me }, bounds.min, "storage_deposit");
  };
  $("starter").onclick = () => send("starter", {}, cfg.record_deposit, "Starter");
  $("resolve").onclick = () => send("resolve", { hunt: state.hunt.id }, cfg.record_deposit, `Resolve hunt #${state.hunt.id}`);
  $("commit").onclick = () => {
    const stake = parseCal($("stake").value);
    if (!stake || stake <= 0n) return say("Enter an amount of CAL.", true);
    send("commit", { amount: stake.toString() }, state.myEntry ? 1n : cfg.entry_deposit, `Commit ${cal(stake)}`);
  };
  $("stake").oninput = preview;
  await blockTime().catch(() => {});
  const loop = async () => { await refresh(); setTimeout(loop, 4000); };
  loop();
})();

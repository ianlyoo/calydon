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
  STARTERS_DONE_THIS_HUNT: "this hunt's starters are all given; the next hunt has new ones",
  RECEIVER_IS_CONTRACT: "CAL goes to the contract only through commit",
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

// A hunt with no outcome is "open" before its deadline and "closed (unresolved)" after it, until someone resolves it.
const outcome = (x) => x.outcome ?? (x.deadline - state.height <= 0 ? "closed (unresolved)" : "open");
const outcomeClass = (x) => x.outcome ?? (x.deadline - state.height <= 0 ? "closed" : "");
// Step 1 is the first step_blocks of a hunt (counted back from the deadline, as the contract does):
// every commit landing there pays the flat tribute with no fill fee; the fill fee applies from step 2 on.
const firstStep = (h) => Math.ceil((h.deadline - state.height) / cfg.step_blocks) === Math.ceil(cfg.hunt_blocks / cfg.step_blocks);

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
  state.forceAll = true;
  await refresh();
}

// The preview works signed out; the Commit button needs a signed-in, registered account and an open hunt.
function gate() {
  $("commit").disabled = !(me && state.reg && state.open);
  $("commitHint").textContent = !me ? "Sign in to commit. The numbers below work signed out." : !state.reg ? "Register (storage_deposit) to commit." : state.open ? "" : "Closed: resolve it to open the next hunt.";
}

function preview() {
  const h = state.hunt;
  if (!h || !cfg) return;
  const stake = parseCal($("stake").value);
  const rows = [];
  let note = "", quick = ["–", "–", "–", "–"], qnote = "";
  if (h.deadline - state.height <= 0) {
    rows.push(["", `hunt #${h.id} is closed: commits are refused until someone resolves it, which opens hunt #${h.id + 1}`]);
  } else if (stake === null || stake <= 0n) {
    rows.push(["", "enter an amount of CAL"]);
  } else {
    const hp = BigInt(h.hp), before = BigInt(h.committed);
    const flat = firstStep(h);
    const fee = fillFee(before, stake, hp, cfg.tribute_bps, flat ? 0 : cfg.fill_fee_bps);
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
    rows.push(["You pay", `${cal(stake + fee)} = stake ${cal(stake)} + entry fee ${cal(fee)} (${(Number((fee * 1000000n) / stake) / 10000).toFixed(4)}% of the stake, ${flat ? `the flat step-1 fee: no fill fee while the commit lands in step 1` : `the fee integrated over the fill from ${cal(before, 2)} to ${cal(before + stake, 2)}`})`]);
    if (flat && steps > 1n) {
      // A wallet approval can take a minute; if the commit lands after step 1 it pays the fill fee and gets one step less.
      const fee2 = fillFee(before, stake, hp, cfg.tribute_bps, cfg.fill_fee_bps);
      const end1 = h.deadline - (Number(steps) - 1) * cfg.step_blocks;
      rows.push(["If it lands after step 1", `step 1 ends at block ${end1} (${end1 - state.height} blocks from now); landing later, it pays entry fee ${cal(fee2)} instead of ${cal(fee)} and gets weight ${cal(stake * (steps - 1n), 2).replace(" CAL", "")}`]);
    }
    rows.push(["NEAR attached", fresh ? `${near(cfg.entry_deposit)} entry deposit (your claim sends it back)` : "1 yoctoNEAR (you already have an entry in this hunt)"]);
    rows.push(["Weight", cfg.rule === "EarlyWeight" ? `${cal(stake, 2).replace(" CAL", "")} × ${steps} step(s) left = ${cal(weight, 2).replace(" CAL", "")}` : cal(weight)]);
    rows.push(["Bar after", `${cal(before + stake, 2)} of ${cal(hp, 2)}${slainNow ? " (at or past HP: slain if nothing changes)" : ` (${cal(hp - before - stake, 2)} still missing)`}`]);
    rows.push(["If slain", `claim ${cal(ifSlain)}: your stake back + ${cal(ifSlain - myStake)} (your weight's share of a ${cal(pot)} pot if nobody else commits; each later commit adds its fee to the pot and its weight to the total)`]);
    rows.push(["If escaped", `claim ${cal(ifEscaped)}: your stake minus ${cfg.burn_bps / 100}% burned; your fee stays in the hoard`]);
    quick = [cal(stake + fee, 2), `${cal(stake, 2).replace(" CAL", "")} × ${steps} = ${cal(weight, 0).replace(" CAL", "")}`, `claim ${cal(ifSlain, 2)}`, `claim ${cal(ifEscaped, 2)}`];
    qnote = `Fee ${cal(fee, 2)} (${(Number((fee * 1000000n) / stake) / 10000).toFixed(2)}%${flat ? ", the flat step-1 fee" : ", rising with the fill"}) goes to the hoard. `
      + (slainNow ? "With this commit the bar reaches the HP." : `After it, ${cal(hp - before - stake, 2)} of health is left.`)
      + (flat && steps > 1n ? ` If the wallet approval lands after step 1, the fee is ${cal(fillFee(before, stake, hp, cfg.tribute_bps, cfg.fill_fee_bps), 2)} and the weight × ${steps - 1n}.` : "");
    rows.push(["Net", `slain: ${cal(ifSlain - paid)} · escaped: ${cal(ifEscaped - paid)} (claim minus stake minus this fee${fresh ? "" : "; your earlier fees in this hunt not counted"})`]);
    if (cfg.rule === "EarlyWeight" && left > 0) {
      const stepEnd = h.deadline - (Number(steps) - 1) * cfg.step_blocks;
      note = `Weight counts the step your commit lands in: ${steps} step(s) until block ${stepEnd} (${stepEnd - state.height} blocks), then ${Number(steps) - 1}.`
        + (flat ? ` Until then every commit pays the flat ${cfg.tribute_bps / 100}% fee in any order; from step 2 the fill fee applies.` : "");
    }
  }
  $("preview").innerHTML = rows.map(([k, v]) => `<tr><th>${k}</th><td style="text-align:left">${v}</td></tr>`).join("");
  $("previewNote").textContent = note;
  ["qPay", "qWeight", "qSlain", "qEscaped"].forEach((id, i) => { $(id).textContent = quick[i]; });
  $("quickNote").textContent = qnote;
}

const hunts = new Map();
const short = (a) => { const i = a.indexOf("."); return i > 0 ? `${a.slice(0, i)}<i>${a.slice(i)}</i>` : a; };
const spear = `<svg viewBox="0 0 14 14" aria-hidden="true"><path d="M1 13 L8.5 5.5" stroke="currentColor" stroke-width="1.5" stroke-linecap="round"/><path d="M8 4 L13 1 L10 6 Z" fill="currentColor"/></svg>`;
const badge = `<span class="badge">${spear}First Spear</span>`;
function duration(s) {
  if (s <= 0) return "0 s";
  const m = Math.floor(s / 60), r = Math.round(s % 60);
  return m ? `${m} min ${String(r).padStart(2, "0")} s` : `${r} s`;
}

// All entries in one read: the contract keeps them under storage prefix 0x02 as (hunt u64, account)
// -> (stake u128, weight u128), Borsh-encoded. view_state works while the state is small (public RPCs
// cap it); if it fails, the page falls back to one `entry` view per hunt for the signed-in account.
const le = (b, o, n) => { let v = 0n; for (let i = n - 1; i >= 0; i--) v = (v << 8n) | BigInt(b[o + i]); return v; };
const unb64 = (x) => Uint8Array.from(atob(x), (c) => c.charCodeAt(0));
async function allEntries() {
  const r = await rpc("query", { finality: "optimistic", request_type: "view_state", account_id: contract, prefix_base64: "Ag==" });
  return r.values.map(({ key, value }) => {
    const k = unb64(key), v = unb64(value);
    const len = Number(le(k, 9, 4));
    return { hunt: Number(le(k, 1, 8)), account: new TextDecoder().decode(k.slice(13, 13 + len)), stake: le(v, 0, 16), weight: le(v, 16, 16) };
  });
}

function renderLog() {
  // Outcome and "open" vs "closed (unresolved)" are worked out at render time from the latest block
  // height, for every row including the current hunt.
  $("log").innerHTML = [...hunts.values()].sort((a, b) => b.id - a.id).map((x) => `<tr><td>#${x.id}</td><td>${cal(x.hp, 2)}</td><td>${cal(x.committed, 2)}</td>
    <td>${x.hunters}</td><td class="${outcomeClass(x)}">${outcome(x)}</td><td>${x.outcome === "Slain" ? cal(x.pot, 2) : "–"}</td>
    <td>${x.claims}/${x.hunters}</td><td class="who">${x.first_spear ? short(x.first_spear) : "–"}</td></tr>`).join("");
}

function renderHunters(h) {
  if (!state.entries) {
    $("hunters").innerHTML = "";
    $("huntersEmpty").hidden = false;
    $("huntersEmpty").textContent = `${h.hunters} hunter(s). The list needs the RPC's view_state, which did not answer.`;
    return;
  }
  const list = state.entries.filter((e) => e.hunt === h.id).sort((a, b) => (b.weight > a.weight ? 1 : b.weight < a.weight ? -1 : 0));
  // Share of the hoard = weight / the hunt's total weight. The entries and the hunt come from separate
  // reads, so right after a join the entries can be newer than the total: take the larger of the two.
  const sum = list.reduce((a, e) => a + e.weight, 0n);
  const total = BigInt(h.weight) > sum ? BigInt(h.weight) : sum;
  $("huntersTitle").textContent = `Hunters in hunt #${h.id}`;
  $("huntersEmpty").hidden = list.length > 0;
  $("huntersEmpty").textContent = h.outcome ? "Every hunter of this hunt has claimed." : "Nobody yet. The first commit takes First Spear.";
  $("hunters").innerHTML = list.map((e) => `<tr><td class="name"><div class="id" title="${e.account}">${short(e.account).replace(/^([^<]+)/, "<b>$1</b>")}</div><div class="tags">${e.account === me ? '<span class="you-tag">you</span>' : ""}${e.account === h.first_spear ? badge : ""}</div></td>
    <td>${cal(e.stake, 2).replace(" CAL", "")}</td><td>${cal(e.weight, 0).replace(" CAL", "")}</td><td>${total ? ((Number((e.weight * 10000n) / total) / 100).toFixed(1) + "%") : "–"}</td></tr>`).join("");
}

function tick() {
  const h = state.hunt;
  if (!h || !state.readAt) return;
  const bt = state.blockTime || 1;
  const leftS = (h.deadline - state.height) * bt - (Date.now() - state.readAt) / 1000;
  const closed = h.deadline - state.height <= 0 || leftS <= 0;
  $("clock").textContent = closed ? "Closed" : duration(leftS);
  $("clock").className = "t num" + (closed ? " closed" : "");
}

async function refresh() {
  try {
    const [h, t, block] = await Promise.all([view("current_hunt", {}), view("totals", {}), rpc("block", { finality: "optimistic" })]);
    if ((state.reads = (state.reads || 0) + 1) % 15 === 1)
      state.keys = (await rpc("query", { finality: "optimistic", request_type: "view_access_key_list", account_id: contract })).keys.length;
    state.hunt = h; state.totals = t; state.height = block.header.height; state.readAt = Date.now();
    const hp = BigInt(h.hp), committed = BigInt(h.committed);
    const left = h.deadline - state.height;
    const slain = committed >= hp && committed > 0n;
    const healthLeft = slain ? 0n : hp - committed;
    const pct = Number((healthLeft * 10000n) / hp) / 100;
    $("title").innerHTML = `The Calydonian Boar<small>Hunt #${h.id} · HP ${cal(h.hp, 2)}</small>`;
    $("state").textContent = h.outcome === "Slain" ? "Slain" : h.outcome === "Escaped" ? "Escaped" : left <= 0 ? (slain ? "Brought down" : "Got away") : slain ? "Will fall at the deadline" : "At large";
    $("state").className = "state " + (slain ? "slain" : left <= 0 ? "closed" : "");
    $("health").style.width = pct + "%";
    $("healthBar").setAttribute("aria-valuenow", String(pct));
    $("hpLeft").innerHTML = `${cal(healthLeft, 2)}<small>health left of ${cal(h.hp, 2)}</small>`;
    $("hpSub").textContent = `${cal(h.committed, 2)} committed by ${h.hunters} hunter${h.hunters === 1 ? "" : "s"}` + (slain ? `: past the HP. Commits still count until the deadline.` : "");
    // Steps: the hunt's hunt_blocks are split into steps of step_blocks, counted back from the deadline.
    const nSteps = Math.ceil(cfg.hunt_blocks / cfg.step_blocks), stepsLeft = Math.max(0, Math.ceil(left / cfg.step_blocks));
    const now = nSteps - stepsLeft; // index of the current step (0-based)
    const spans = $("steps").children;
    if (spans.length !== nSteps) $("steps").innerHTML = "<span></span>".repeat(nSteps);
    [...$("steps").children].forEach((el, i) => { el.className = i < now ? "past" : i === now && left > 0 ? "now" : ""; });
    $("deadline").textContent = left > 0
      ? `Step ${now + 1} of ${nSteps}: a commit now weighs stake × ${stepsLeft}. Deadline block ${h.deadline} (${left} blocks). If it closed now: ${slain ? "Slain" : "Escaped"}.`
      : `Closed at block ${h.deadline}: commits are refused. Anyone can resolve it now (${slain ? "Slain" : "Escaped"}); that opens hunt #${h.id + 1}.`;
    tick();
    $("hoard").textContent = cal(t.hoard, 2);
    $("committed").textContent = `${cal(h.committed)} by ${h.hunters} hunter(s)`;
    $("hp").textContent = cal(h.hp);
    $("weight").textContent = cal(h.weight, 2).replace(" CAL", "");
    $("spear").textContent = h.first_spear ?? "nobody yet (the first commit of a hunt takes it)";
    $("supply").textContent = cal(t.supply);
    $("burned").textContent = cal(t.burned);
    $("unclaimed").textContent = cal(t.unclaimed);
    $("bucket").textContent = cal(t.starter_left, 0);
    if (state.keys !== undefined) $("keys").textContent = state.keys === 0 ? "0 (nobody can change or redeploy the code)" : String(state.keys);
    $("resolve").hidden = !(me && left <= 0);
    $("closedNote").hidden = left > 0;
    $("closedNote").textContent = `Hunt #${h.id} is closed: commits are refused until someone resolves it, which opens hunt #${h.id + 1}.`;
    $("quick").hidden = left <= 0;
    state.open = left > 0;
    gate();
    hunts.set(h.id, h);
    renderLog();
    state.entries = await allEntries().catch(() => null);
    renderHunters(h);
    // Older hunts are read again while unresolved (every read) and while some hunter has not claimed
    // (every 15th read, about once a minute, or right after this page sent a transaction): public RPCs
    // rate-limit, and a long session must keep its reads low. One failed read leaves that row as it was.
    const all = state.forceAll || state.reads % 15 === 1;
    for (let id = Math.max(0, h.id - 49); id < h.id; id++) {
      const x = hunts.get(id);
      if (!x || !x.outcome || (all && x.claims < x.hunters)) hunts.set(id, await view("hunt", { id }).catch(() => x));
    }
    state.forceAll = false;
    for (const [id, x] of hunts) if (!x) hunts.delete(id);
    renderLog();
    if (me) await refreshMe(h);
    preview();
    $("updated").textContent = `block ${state.height}, ${new Date().toLocaleTimeString()}`;
    return true;
  } catch (e) {
    $("updated").textContent = (state.height ? `block ${state.height} (last good read); ` : "") + "the RPC did not answer, reading again shortly: " + e.message;
    return false;
  }
}

async function refreshMe(h) {
  const [bal, reg, taken] = await Promise.all([view("ft_balance_of", { account_id: me }),
    view("storage_balance_of", { account_id: me }), view("starter_taken", { account_id: me })]);
  $("myBal").textContent = cal(bal, 2);
  $("myReg").textContent = reg ? "yes" : "no";
  $("register").hidden = !!reg;
  const leftThisHunt = cfg.starters_per_hunt - (h.starters ?? 0);
  $("myStarter").textContent = taken ? "taken" : BigInt(state.totals.starter_left) === 0n ? "bucket empty"
    : leftThisHunt <= 0 ? `this hunt's ${cfg.starters_per_hunt} starters are given; the next hunt has new ones`
    : `${leftThisHunt} of ${cfg.starters_per_hunt} left this hunt`;
  $("starter").hidden = !!taken || !reg || BigInt(state.totals.starter_left) === 0n || leftThisHunt <= 0;
  state.reg = !!reg;
  gate();
  let mine;
  if (state.entries) mine = state.entries.filter((e) => e.account === me).map((e) => [e.hunt, e]);
  else {
    mine = [];
    for (let id = Math.max(0, h.id - 9); id <= h.id; id++) {
      const e = await view("entry", { hunt: id, account_id: me });
      if (e) mine.push([id, { stake: BigInt(e.stake), weight: BigInt(e.weight) }]);
    }
  }
  const rows = [];
  state.myEntry = null;
  for (const [id, e] of mine.sort((a, b) => b[0] - a[0])) {
    const x = id === h.id ? h : hunts.get(id) || (await view("hunt", { id }));
    if (id === h.id) state.myEntry = e;
    let pays = "–", button = "";
    if (x.outcome === "Slain") pays = cal(e.stake + share(BigInt(x.pot), e.weight, BigInt(x.weight)));
    if (x.outcome === "Escaped") pays = cal(e.stake - ceilBps(e.stake, cfg.burn_bps));
    if (x.outcome) button = `<button class="btn small" data-claim="${id}">Claim</button>`;
    rows.push(`<tr><td>#${id}</td><td>${cal(e.stake, 2)}</td><td>${cal(e.weight, 2).replace(" CAL", "")}</td><td class="${outcomeClass(x)}">${outcome(x)}</td><td>${pays}</td><td>${button}</td></tr>`);
  }
  $("entriesPanel").hidden = rows.length === 0;
  $("entries").innerHTML = rows.join("");
  for (const b of document.querySelectorAll("[data-claim]"))
    b.onclick = () => send("claim", { hunt: Number(b.dataset.claim) }, 0n, `Claim hunt #${b.dataset.claim}`);
}

async function showAccount() {
  const wallet = selector.isSignedIn() ? await selector.wallet() : null;
  const accounts = wallet ? await wallet.getAccounts() : [];
  me = accounts[0]?.accountId || null;
  $("signedOut").hidden = !!me;
  $("signedIn").hidden = !me;
  $("you").hidden = !me;
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
  const fill = { steps: Math.ceil(cfg.hunt_blocks / cfg.step_blocks), up: cfg.hp_up_bps / 10000, down: cfg.hp_down_bps / 10000, burn: cfg.burn_bps / 100 };
  for (const el of document.querySelectorAll("[data-cfg]")) el.textContent = String(fill[el.dataset.cfg]);
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
  // Every 4 s; after a failed read (often a rate limit) the wait doubles, up to 32 s, until a read works.
  let wait = 4000;
  const loop = async () => { wait = (await refresh()) ? 4000 : Math.min(32000, wait * 2); setTimeout(loop, wait); };
  loop();
  setInterval(tick, 1000);
})();

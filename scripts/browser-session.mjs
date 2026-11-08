// How docs/browser/ was recorded: one Playwright session on the local page (npm run page), signing
// through Intear Wallet, one step per call. Not part of `npm run verify`; it needs playwright-core and a
// Chromium (CHROME=<path to the browser binary>), neither of which is a dependency of this repository.
//   node scripts/browser-session.mjs <profileDir> <shotDir> <step> [arg]
//   steps: signin | register | starter | resolve <hunt> | commit <CAL> | claim <hunt> | shot <name> | text
// <profileDir> is a browser profile in which a testnet account was imported at wallet.intear.tech
// beforehand. Each step clicks the page's button and then "Approve" in the wallet window.
import { chromium } from "playwright-core";
const exe = process.env.CHROME;
const [profile, dir, step, arg] = process.argv.slice(2);
if (!exe || !step) throw new Error("usage: CHROME=<browser binary> node scripts/browser-session.mjs <profileDir> <shotDir> <step> [arg]");
const PAGE = process.env.PAGE_URL || "http://127.0.0.1:8494/status/";
const ctx = await chromium.launchPersistentContext(profile, { executablePath: exe, headless: true, locale: "en-US", viewport: { width: 1000, height: 1250 } });
const page = ctx.pages()[0] || (await ctx.newPage());
page.on("console", (m) => { if (m.type() === "error") console.log("console:", m.text().slice(0, 160)); });
const shot = async (name, p = page) => { await p.screenshot({ path: `${dir}/${name}.png`, fullPage: p === page, mask: p === page ? [p.locator("input[type=password]")] : [] }); console.log("shot", name); };
const msg = () => page.locator("#msg").innerText();

async function load() {
  await page.goto(PAGE, { waitUntil: "networkidle" });
  await page.waitForFunction(() => document.getElementById("hp")?.textContent.trim() !== "", null, { timeout: 30000 });
  await page.waitForTimeout(1500);
}

// Clicks `button` on the page, approves the wallet popup, waits for the page's result message.
async function approve(button, name) {
  const popP = ctx.waitForEvent("page", { timeout: 20000 }).catch(() => null);
  await page.click(button);
  const pop = await popP;
  if (pop) {
    await pop.waitForLoadState("networkidle").catch(() => {});
    await pop.waitForFunction(() => !/Simulating the transaction/.test(document.body.innerText), null, { timeout: 20000 }).catch(() => {});
    await pop.waitForTimeout(1500);
    await shot(`${name}-wallet`, pop);
    const buttons = await pop.locator("button").allInnerTexts();
    console.log("wallet buttons:", JSON.stringify(buttons));
    const ok = pop.locator("button").filter({ hasText: /^(Confirm|Approve|Sign|Send|Connect)\b/i }).first();
    await ok.click({ timeout: 10000 });
  } else console.log("no wallet window (signed with the app key)");
  await page.waitForFunction(() => /done|refused|Signed in/.test(document.getElementById("msg").textContent), null, { timeout: 120000 });
  console.log("page:", await msg());
  await page.waitForTimeout(5000);
}

await load();
if (step === "signin") {
  await shot("01-signed-out");
  await page.click("#signIn");
  await page.waitForTimeout(4000);
  const fr = page.frames().find((f) => f.url().includes("wallet-connector-iframe"));
  const popP = ctx.waitForEvent("page", { timeout: 20000 });
  await fr.getByText("Web", { exact: true }).first().click();
  const pop = await popP;
  await pop.waitForLoadState("networkidle").catch(() => {});
  await pop.waitForTimeout(3000);
  await shot("02-wallet-connect", pop);
  await pop.locator("button").filter({ hasText: /^Connect$/ }).first().click();
  await page.waitForFunction(() => !document.getElementById("signedIn").hidden, null, { timeout: 60000 });
  await page.waitForTimeout(4000);
  console.log("signed in as", await page.locator("#me").innerText());
  await shot("03-signed-in");
} else if (step === "register") {
  await approve("#register", "04-storage-deposit");
  await shot("05-registered");
} else if (step === "starter") {
  await approve("#starter", "06-starter");
  await shot("07-starter-taken");
} else if (step === "resolve") {
  await approve("#resolve", `resolve-${arg}`);
  await shot(`resolved-${arg}`);
} else if (step === "commit") {
  await page.fill("#stake", arg);
  await page.waitForTimeout(1500);
  await shot("08-commit-preview");
  await approve("#commit", "09-commit");
  await shot("10-committed");
} else if (step === "claim") {
  await approve(`[data-claim="${arg}"]`, `claim-${arg}`);
  await shot(`claimed-${arg}`);
} else if (step === "shot") {
  await shot(arg);
} else if (step === "text") {
  console.log((await page.innerText("body")).slice(0, 3000));
}
await ctx.close();

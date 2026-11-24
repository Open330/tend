#!/usr/bin/env node
// The web dashboard's screens for the cross-surface parity audit
// (scripts/parity-shots/run.sh): Home's lanes, a question card, a review
// card and the Review sheet, Start Work, Explore, an agent's details,
// Settings and Messages, on a desktop (1440x900) and a phone (390x844)
// viewport, light and dark, plus the desktop in Korean.
//
//   TEND_PARITY_URL=http://127.0.0.1:7979 TEND_PARITY_TOKEN=… \
//   TEND_PARITY_OUT=DIR node scripts/parity-shots/web.mjs
//
// Read only: it opens, scrolls and closes, and never answers, decides,
// starts or sends anything, so the shots show the fixture's open state for
// the other surfaces too. Writes DIR/web-<screen>-<viewport>-<scheme>[-ko].png
// and, beside each, a .txt of the main region's innerText. A screen that
// cannot be reached is listed in DIR/web-missing.txt and skipped; a page
// error fails the run.
// Playwright: TEND_PLAYWRIGHT_PACKAGE (default `playwright`), its own
// Chromium unless TEND_TEST_CHROMIUM names a browser executable.
import { createRequire } from "node:module";
import { mkdirSync, writeFileSync, appendFileSync } from "node:fs";
import { join } from "node:path";

// Only inside scripts/parity-shots/run.sh on a GitHub-hosted runner:
// run.sh exports TEND_PARITY_CI=1 after its own CI check. Checked before
// anything else happens (no browser, no request).
const env = process.env;
if (env.TEND_PARITY_CI !== "1" || env.CI !== "true" || env.GITHUB_ACTIONS !== "true" || env.RUNNER_ENVIRONMENT !== "github-hosted") {
  console.error("web.mjs: runs only from scripts/parity-shots/run.sh on a GitHub-hosted runner (TEND_PARITY_CI=1)");
  process.exit(2);
}
const base = env.TEND_PARITY_URL;
const token = env.TEND_PARITY_TOKEN;
const out = env.TEND_PARITY_OUT;
if (!base || !token || !out) {
  console.error("TEND_PARITY_URL, TEND_PARITY_TOKEN and TEND_PARITY_OUT are required");
  process.exit(2);
}
const url = new URL(base);
if (url.protocol !== "http:" || url.hostname !== "127.0.0.1" || !url.port || url.username || url.password) {
  console.error("web.mjs: only the fixture's loopback dashboard");
  process.exit(2);
}

/// Every message that leaves this script, token-free: a failed navigation
/// carries the URL it went to, and the first one has the token in it.
function scrub(text) {
  return String(text ?? "").split(token).join("<token>").split(encodeURIComponent(token)).join("<token>");
}
function firstLine(error) {
  return scrub(error?.message ?? error).split("\n")[0];
}

mkdirSync(out, { recursive: true });
const require = createRequire(import.meta.url);
const { chromium } = require(env.TEND_PLAYWRIGHT_PACKAGE || "playwright");
const browser = await chromium.launch({
  headless: true,
  ...(env.TEND_TEST_CHROMIUM ? { executablePath: env.TEND_TEST_CHROMIUM } : {}),
});

const missing = [];
const problems = [];
const blocked = [];
let shots = 0;

function miss(name, why) {
  missing.push(`${name}: ${scrub(why)}`);
  console.log(`  missing ${name}: ${scrub(why)}`);
}

function write(name, png, words) {
  writeFileSync(join(out, `${name}.png`), png);
  writeFileSync(join(out, `${name}.txt`), `${scrub(words).trim()}\n`);
  shots += 1;
  console.log(`  ${name}`);
}

/// A whole-viewport screen, with the visible view's text (or `text`'s).
async function save(page, name, { text } = {}) {
  const png = await page.screenshot();
  let words;
  try {
    const region = text || page.locator("#view-home:visible, #view-explore:visible, #view-settings:visible").first();
    words = await region.innerText({ timeout: 5000 });
  } catch (error) {
    words = `(no text: ${firstLine(error)})`;
  }
  write(name, png, words);
}

/// One element, photographed and read from the same handle once it has
/// stopped moving. The board redraws on every poll, so a picture whose
/// element changed in between (text or data-id) is thrown away, and so is
/// one whose text does not say what it should (`expect`).
async function saveElement(name, locator, { expect } = {}) {
  const handle = await locator.elementHandle({ timeout: 10000 });
  try {
    await handle.scrollIntoViewIfNeeded().catch(() => {});
    await handle.waitForElementState("stable", { timeout: 5000 });
    const id = await handle.getAttribute("data-id");
    const before = await handle.innerText();
    const png = await handle.screenshot();
    const after = await handle.innerText();
    if (before !== after || id !== (await handle.getAttribute("data-id"))) {
      throw new Error(`${name}: the element changed while it was photographed`);
    }
    if (expect && !expect.test(before)) throw new Error(`${name}: its text does not match ${expect}`);
    write(name, png, before);
  } finally {
    await handle.dispose().catch(() => {});
  }
}

/// One attempt at a screen: a failure to find what it needs is a miss,
/// not the run's failure.
/// Tried twice: the board redraws on every poll, and a card can be
/// replaced between finding and photographing it.
async function attempt(name, body) {
  try {
    await body();
  } catch (first) {
    if (/no such lane|no Start Work|no Review button/.test(first.message)) return miss(name, firstLine(first));
    try {
      await body();
    } catch (error) {
      miss(name, firstLine(error));
    }
  }
}

/// The phone's lane tabs. Clicked by event: Playwright's pointer click
/// times out on them (the strip sits outside what it can scroll to).
async function segment(page, lane) {
  await page.locator(`#segments [data-lane="${lane}"]`).dispatchEvent("click");
  await page.waitForTimeout(800);
}

async function open(context, path) {
  const page = await context.newPage();
  page.on("pageerror", (error) => problems.push(`${path}: pageerror: ${scrub(error.message)}`));
  // The token arrives in the fragment, the way a link from `tend` brings
  // it; Home keeps it and the other pages read it from there.
  await page.goto(`${base}/home#token=${token}`);
  await page.locator("#lanes").waitFor({ timeout: 20000 });
  if (page.url().includes(token)) throw new Error("the token stayed in the address bar");
  if (path !== "/home") await page.goto(`${base}${path}`);
  return page;
}

async function settle(page, ms = 1200) {
  await page.waitForTimeout(ms);
}

async function run(viewport, scheme, lang) {
  const phone = viewport === "phone";
  const suffix = `${viewport}-${scheme}${lang === "ko" ? "-ko" : ""}`;
  const context = await browser.newContext({
    viewport: phone ? { width: 390, height: 844 } : { width: 1440, height: 900 },
    deviceScaleFactor: phone ? 3 : 2,
    isMobile: phone,
    hasTouch: phone,
    colorScheme: scheme,
    locale: lang === "ko" ? "ko-KR" : "en-US",
  });
  // Read only, enforced: anything but a read to the dashboard's API is
  // refused before it leaves the browser, and listed.
  await context.route("**/api/**", (route) => {
    const method = route.request().method();
    if (method === "GET" || method === "HEAD") return route.continue();
    blocked.push(`${suffix} ${method} ${scrub(new URL(route.request().url()).pathname)}`);
    return route.abort("blockedbyclient");
  });
  console.log(`web ${suffix}`);
  // On the phone viewport Playwright's pointer click times out on every
  // control (the board is its own scroller under a fixed shell), so the
  // phone gets the click event itself.
  const press = async (locator) => {
    if (!phone) return locator.click();
    await locator.waitFor({ state: "attached", timeout: 10000 });
    await locator.scrollIntoViewIfNeeded().catch(() => {});
    await locator.dispatchEvent("click");
  };
  try {
    const page = await open(context, "/home");
    const yourTurn = page.locator(".lane.needsYou");
    await yourTurn.locator(".card").first().waitFor({ timeout: 20000 });
    await settle(page, 2500);

    // Home and each lane (the phone shows one lane at a time).
    await save(page, `web-home-${suffix}`);
    await saveElement(`web-home-full-${suffix}`, page.locator("#board"));
    if (phone) {
      for (const [lane, slug] of [["backlog", "backlog"], ["needsYou", "yourturn"], ["running", "inprogress"], ["finished", "finished"]]) {
        await attempt(`web-lane-${slug}-${suffix}`, async () => {
          const tab = page.locator(`#segments [data-lane="${lane}"]`);
          if (!(await tab.isVisible())) throw new Error("no such lane tab (empty)");
          await tab.dispatchEvent("click");
          await settle(page);
          await save(page, `web-lane-${slug}-${suffix}`);
        });
      }
      await segment(page, "needsYou");
      await settle(page);
    } else {
      for (const [lane, slug] of [["backlog", "backlog"], ["needsYou", "yourturn"], ["running", "inprogress"], ["finished", "finished"]]) {
        await attempt(`web-lane-${slug}-${suffix}`, async () => {
          const section = page.locator(`.lane.${lane}`).first();
          if (!(await section.count())) throw new Error("no such lane (empty)");
          await saveElement(`web-lane-${slug}-${suffix}`, section);
        });
      }
    }

    // A question and a review card, as Your turn draws them.
    for (const [kind, slug] of [["question", "question"], ["review", "review-card"]]) {
      await attempt(`web-${slug}-${suffix}`, async () => {
        const word = lang === "ko" ? (kind === "question" ? "질문" : "리뷰") : kind === "question" ? "Question" : "Review";
        const found = yourTurn.locator(".card.decision, .row-c", { has: page.locator(".kind", { hasText: word }) }).first();
        await found.waitFor({ timeout: 10000 });
        // Pinned by its own id from here on, not by "the first such card".
        const id = await found.getAttribute("data-id");
        if (!id) throw new Error(`the ${kind} card has no data-id`);
        const card = yourTurn.locator(`[data-id=${JSON.stringify(id)}]`);
        await card.scrollIntoViewIfNeeded();
        await settle(page, 500);
        await saveElement(`web-${slug}-${suffix}`, card, { expect: new RegExp(word) });
        await save(page, `web-${slug}-context-${suffix}`, { text: card });
      });
    }

    // The Review sheet (opened and closed with Done; nothing marked).
    await attempt(`web-review-sheet-${suffix}`, async () => {
      const button = page.locator("#review-button");
      if (!(await button.isVisible())) throw new Error("no Review button on Home");
      await press(button);
      const dialog = page.locator("#review-dialog");
      await dialog.waitFor({ state: "visible", timeout: 10000 });
      await settle(page, 1500);
      await save(page, `web-review-sheet-${suffix}`, { text: dialog });
      await press(page.locator("#review-done"));
      await dialog.waitFor({ state: "hidden", timeout: 5000 });
    });

    // Start Work, opened and cancelled.
    await attempt(`web-start-work-${suffix}`, async () => {
      const start = page.locator("#start-work");
      if (!(await start.isVisible())) throw new Error("no Start Work button (work start off for this token?)");
      await press(start);
      const dialog = page.locator("#start-dialog");
      await dialog.waitFor({ state: "visible", timeout: 10000 });
      await settle(page, 1500);
      await save(page, `web-start-work-${suffix}`, { text: dialog });
      await press(page.locator("#start-cancel"));
      await dialog.waitFor({ state: "hidden", timeout: 5000 });
    });

    // An agent's details: the first unit at work, in the inspector.
    // The agent asking a choice (a unit in Your turn), in the inspector.
    await attempt(`web-choice-detail-${suffix}`, async () => {
      if (phone) await segment(page, "needsYou");
      const unit = page.locator(".lane.needsYou .card.work, .lane.needsYou .row-c", { hasText: "rotate the JWT" }).first();
      await unit.waitFor({ timeout: 10000 });
      await press(unit);
      const inspector = page.locator("#inspector");
      await inspector.waitFor({ state: "visible", timeout: 10000 });
      await settle(page, 2000);
      await save(page, `web-choice-detail-${suffix}`, { text: inspector });
      await page.keyboard.press("Escape");
      await inspector.waitFor({ state: "hidden", timeout: 5000 }).catch(() => {});
    });

    await attempt(`web-agent-detail-${suffix}`, async () => {
      if (phone) await segment(page, "running");
      const unit = page.locator(".lane.running .card.work, .lane.running .row-c").first();
      await unit.waitFor({ timeout: 10000 });
      await press(unit);
      const inspector = page.locator("#inspector");
      await inspector.waitFor({ state: "visible", timeout: 10000 });
      await settle(page, 2000);
      await save(page, `web-agent-detail-${suffix}`, { text: inspector });
      await page.keyboard.press("Escape");
      await inspector.waitFor({ state: "hidden", timeout: 5000 }).catch(() => {});
      if (phone) await segment(page, "needsYou");
    });

    // The compact list, where a decision opens beside the board.
    await attempt(`web-home-compact-${suffix}`, async () => {
      await press(page.locator('#layout-toggle [data-layout="compact"]'));
      await settle(page, 1500);
      await save(page, `web-home-compact-${suffix}`);
      const row = page.locator(".lane.needsYou .row-c").first();
      await row.waitFor({ timeout: 10000 });
      await press(row);
      const inspector = page.locator("#inspector");
      await inspector.waitFor({ state: "visible", timeout: 10000 });
      await settle(page, 1200);
      await save(page, `web-decision-detail-${suffix}`, { text: inspector });
      await page.keyboard.press("Escape");
      await press(page.locator('#layout-toggle [data-layout="emphasis"]'));
      await settle(page, 800);
    });

    // Explore: the hosts, sessions and panes, then one agent's pane.
    await attempt(`web-explore-${suffix}`, async () => {
      await page.goto(`${base}/explore`);
      await page.locator("#view-explore .ex-row").first().waitFor({ timeout: 20000 });
      await settle(page, 2500);
      await save(page, `web-explore-${suffix}`);
      const row = page.locator("#view-explore .ex-row", { hasText: "rotate the JWT" }).first();
      const pick = (await row.count()) ? row : page.locator("#view-explore .ex-row").nth(1);
      await press(pick);
      await settle(page, 3000);
      await save(page, `web-explore-pane-${suffix}`);
    });

    // Explore → Messages: the collaboration history, then one message.
    await attempt(`web-messages-${suffix}`, async () => {
      await page.goto(`${base}/explore/messages`);
      const first = page.locator("#view-explore .mx-row").first();
      await first.waitFor({ timeout: 20000 });
      await settle(page, 1500);
      await save(page, `web-messages-${suffix}`);
      await press(first);
      await page.locator(".mx-detail").first().waitFor({ timeout: 10000 });
      await settle(page, 1200);
      await save(page, `web-messages-open-${suffix}`);
    });

    // Settings (read; its managers stay closed).
    await attempt(`web-settings-${suffix}`, async () => {
      await page.goto(`${base}/settings`);
      await page.locator("#view-settings .set-page").waitFor({ timeout: 20000 });
      await settle(page, 1200);
      await save(page, `web-settings-${suffix}`);
    });
  } catch (error) {
    miss(`web-*-${suffix}`, firstLine(error));
  } finally {
    await context.close();
  }
}

try {
  for (const viewport of ["desktop", "phone"]) {
    for (const scheme of ["light", "dark"]) await run(viewport, scheme, "en");
  }
  await run("desktop", "light", "ko");
} finally {
  await browser.close();
}
if (missing.length) appendFileSync(join(out, "web-missing.txt"), `${missing.join("\n")}\n`);
if (blocked.length) {
  writeFileSync(join(out, "web-blocked-requests.txt"), `${blocked.join("\n")}\n`);
  console.log(`web: refused ${blocked.length} non-read API requests (web-blocked-requests.txt)`);
}
console.log(`web: ${shots} screens, ${missing.length} missing`);
if (problems.length) {
  console.error(problems.join("\n"));
  process.exit(1);
}
process.exit(shots ? 0 : 1);

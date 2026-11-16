#!/usr/bin/env node
// Browser end-to-end test of the dashboard's Home (/home) against a scratch
// tendd: answer a question, decide a result, and see both land in Finished;
// then the same on a phone-sized viewport.
//
//   apps/tend-ios/Scripts/scratch-tendd.sh up        # seeds a question + a result
//   apps/tend-ios/Scripts/scratch-tendd.sh seed      # one more pair for the phone run
// The desktop run uses status emphasis (cards answer in place), the phone
// run the compact list (a decision opens in the details sheet).
//   TEND_HOME_URL=http://127.0.0.1:7979 \
//   TEND_HOME_TOKEN=$(apps/tend-ios/Scripts/scratch-tendd.sh token) \
//   node scripts/dashboard-home-e2e.mjs
//
// Never point it at a daemon whose items are real: it answers and decides.
// Playwright: TEND_PLAYWRIGHT_PACKAGE (default `playwright-core`), with the
// installed Chrome (`channel: chrome`) unless TEND_TEST_CHROMIUM names a
// browser executable. One browser, closed at the end.

const base = process.env.TEND_HOME_URL || "http://127.0.0.1:7979";
const token = process.env.TEND_HOME_TOKEN;
if (!token) {
  console.error("TEND_HOME_TOKEN is required (the scratch daemon's operator token)");
  process.exit(2);
}
const pkg = process.env.TEND_PLAYWRIGHT_PACKAGE || "playwright-core";
const { chromium } = await import(pkg);
const launch = process.env.TEND_TEST_CHROMIUM
  ? { executablePath: process.env.TEND_TEST_CHROMIUM }
  : { channel: "chrome" };
const browser = await chromium.launch({ headless: true, ...launch });

function check(condition, message) {
  if (!condition) throw new Error(message);
}

async function run(name, contextOptions) {
  const { layout, ...options } = contextOptions;
  const context = await browser.newContext(options);
  if (layout) await context.addInitScript((value) => localStorage.setItem("tend.home.cardLayout", value), layout);
  const page = await context.newPage();
  const problems = [];
  page.on("pageerror", (error) => problems.push(`pageerror: ${error.message}`));
  page.on("console", (message) => {
    // A CSP refusal shows up here: Home must run under its strict policy.
    if (message.type() === "error") problems.push(`console: ${message.text()}`);
  });
  // The token arrives in the fragment, the way a link from `tend` brings it.
  await page.goto(`${base}/home#token=${token}`);
  check(!page.url().includes(token), "the token stayed in the address bar");
  const yourTurn = page.locator(".lane.needsYou");
  const finished = page.locator(".lane.finished");
  const phone = (contextOptions.viewport?.width || 1440) < 760;
  await yourTurn.locator(".card.decision, .row-c").first().waitFor({ timeout: 15000 });

  // In the compact list a decision opens beside the board; with status
  // emphasis (the default) the card answers in place.
  const compact = contextOptions.layout === "compact";
  const decisionNamed = async (kind) => {
    if (!compact) {
      const card = yourTurn.locator(".card.decision", { has: page.locator(".kind", { hasText: kind }) }).first();
      return { card, id: await card.getAttribute("data-id") };
    }
    const rows = yourTurn.locator(".row-c");
    for (let i = 0; i < (await rows.count()); i += 1) {
      await rows.nth(i).click();
      const card = page.locator("#inspector .card.decision");
      if (await card.locator(".kind", { hasText: kind }).count()) return { card, id: await rows.nth(i).getAttribute("data-id") };
    }
    throw new Error(`no ${kind} in Your turn`);
  };
  const gone = (id) => yourTurn.locator(`[data-id="${id}"]`).waitFor({ state: "detached", timeout: 15000 });

  // Answer the first question.
  const { card: question, id: questionId } = await decisionNamed("Question");
  const title = (await question.locator(".d-title, .e-title").first().textContent()).trim();
  await question.getByRole("button", { name: /Answer…|Other…/ }).click();
  const answer = `e2e answer ${Date.now()}`;
  await question.locator("textarea").fill(answer);
  await question.getByRole("button", { name: "Send" }).click();
  await gone(questionId);

  // Decide the first result: pick an option, then Accept.
  const { card: review, id: reviewId } = await decisionNamed("Review");
  const reviewTitle = (await review.locator(".d-title, .e-title").first().textContent()).trim();
  const option = review.locator(".option").first();
  if (await option.count()) {
    await option.click();
    await review.locator("textarea").fill("e2e: accepted with its first option");
    await review.locator(".composer").getByRole("button", { name: "Accept" }).click();
  } else {
    await review.getByRole("button", { name: "Accept" }).click();
  }
  await gone(reviewId);

  // Both show up in Finished with what was decided.
  if (phone) await page.getByRole("tab", { name: /Finished/ }).click();
  const answered = finished.locator(`[data-id="human_answer:${questionId}"]`);
  await answered.waitFor({ timeout: 15000 });
  if (!compact) check((await answered.textContent()).includes(answer), "Finished does not show the answer that was sent");
  await finished.locator(`[data-id="review:${reviewId}"]`).waitFor({ timeout: 15000 });
  if (phone) check(!(await yourTurn.isVisible()), "the phone shows one lane at a time");

  // A unit opens the inspector and Escape closes it.
  const unit = page.locator(".lane:visible .card.work, .lane:visible .row-c").first();
  if (await unit.count()) {
    await unit.click();
    await page.locator("#inspector .nowbox").waitFor({ timeout: 5000 });
    await page.keyboard.press("Escape");
    await page.locator("#inspector").waitFor({ state: "hidden", timeout: 5000 });
  }
  check(!problems.length, problems.join("\n"));
  await context.close();
  console.log(`ok ${name}: answered "${title}", accepted "${reviewTitle}"`);
}

let failed = false;
try {
  await run("desktop, status emphasis", { viewport: { width: 1440, height: 900 } });
  await run("phone, compact list", { viewport: { width: 390, height: 844 }, deviceScaleFactor: 3, isMobile: true, hasTouch: true, colorScheme: "dark", layout: "compact" });
} catch (error) {
  failed = true;
  console.error(`not ok: ${error.message}`);
} finally {
  await browser.close();
}
process.exit(failed ? 1 : 0);

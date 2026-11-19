// Photograph a running dashboard for the landing page, in light and dark.
//
//   TEND_SHOTS_URL=http://127.0.0.1:7993 TEND_SHOTS_TOKEN=… node capture-live.mjs
//
// Run by live.sh against the demo fixture fleet; never point it at a
// daemon whose items are real. Writes crates/tend/src/dashboard/web/landing/
// {board,agents,collaboration}-{light,dark}.webp:
//   board          Home: Your turn, In progress, Finished
//   agents         Explore: every pane of the host in its topology
//   collaboration  Explore → Messages: the collaboration history
// Playwright: TEND_PLAYWRIGHT_PACKAGE (default `playwright-core`), with
// Chrome (channel "chrome") unless TEND_TEST_CHROMIUM names a browser.
import { createRequire } from "node:module";
import { execFileSync } from "node:child_process";
import { mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const require = createRequire(import.meta.url);
const { chromium } = require(process.env.TEND_PLAYWRIGHT_PACKAGE || "playwright-core");
const HERE = dirname(fileURLToPath(import.meta.url));
const OUT = resolve(HERE, "../../crates/tend/src/dashboard/web/landing");
const base = process.env.TEND_SHOTS_URL;
const token = process.env.TEND_SHOTS_TOKEN;
if (!base || !token) throw new Error("TEND_SHOTS_URL and TEND_SHOTS_TOKEN are required");

const SHOTS = [
  {
    name: "board",
    path: "/",
    viewport: { width: 1440, height: 900 },
    ready: async (page) => page.locator("#lanes .card").first().waitFor({ timeout: 15000 }),
  },
  {
    name: "agents",
    path: "/explore",
    viewport: { width: 1440, height: 900 },
    ready: async (page) => {
      const view = page.locator("#view-explore");
      await view.waitFor({ state: "visible", timeout: 15000 });
      // Open an agent waiting on a choice, so the shot shows its own screen.
      await view.getByText("rotate the JWT", { exact: false }).last().click();
      await page.waitForTimeout(3000);
    },
  },
  {
    name: "collaboration",
    path: "/explore/messages",
    viewport: { width: 1440, height: 1000 },
    ready: async (page) => {
      await page.locator("#view-explore .mx-row").first().waitFor({ timeout: 15000 });
      await page.waitForTimeout(1200);
    },
  },
];

const browser = await chromium.launch(process.env.TEND_TEST_CHROMIUM ? { executablePath: process.env.TEND_TEST_CHROMIUM } : { channel: "chrome" });
const tmp = mkdtempSync(join(tmpdir(), "tend-shots-"));
try {
  for (const scheme of ["light", "dark"]) {
    for (const shot of SHOTS) {
      const context = await browser.newContext({ viewport: shot.viewport, deviceScaleFactor: 2, colorScheme: scheme, locale: "en-US" });
      const page = await context.newPage();
      const problems = [];
      page.on("pageerror", (e) => problems.push(e.message));
      // The token arrives in the fragment, as a link from `tend` brings it;
      // Home stores it, and the other pages read it from there.
      await page.goto(`${base}/home#token=${token}`);
      await page.locator("#lanes").waitFor({ timeout: 15000 });
      if (shot.path !== "/") await page.goto(`${base}${shot.path}`);
      await shot.ready(page);
      await page.waitForTimeout(1000);
      if (problems.length) throw new Error(`${shot.name}: ${problems.join("; ")}`);
      const png = join(tmp, `${shot.name}-${scheme}.png`);
      if (shot.target) await page.locator(shot.target).screenshot({ path: png });
      else await page.screenshot({ path: png });
      const webp = join(OUT, `${shot.name}-${scheme}.webp`);
      // 1600px wide is sharp on a 2x screen at the landing's display size.
      execFileSync("cwebp", ["-quiet", "-q", "78", "-resize", "1600", "0", png, "-o", webp]);
      console.log("wrote", webp);
      await context.close();
    }
  }
} finally {
  await browser.close();
  rmSync(tmp, { recursive: true, force: true });
}

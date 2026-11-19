// /welcome: its return address is a same-origin path without credentials,
// a token in its own address is stored and scrubbed, and the page is the
// landing alone, written for the strict policy (no inline script or style,
// no workbench).
import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

import { consumeToken, safeReturnTo, signInHref, TOKEN_KEY } from "../src/dashboard/web/sign-in.mjs";

globalThis.__TEND_WELCOME_NO_START__ = true;
const { welcomeAddress } = await import("../src/dashboard/web/welcome.mjs");
const WEB = join(dirname(fileURLToPath(import.meta.url)), "../src/dashboard/web");

test("return_to is a same-origin path, or nothing", () => {
  for (const good of ["/", "/home", "/home#item=req_9", "/explore#pane=%251&socket=s", "/explore/messages#message=r1&host=mba", "/settings"]) {
    assert.equal(safeReturnTo(good), good, good);
  }
  for (const bad of ["", "home", "//evil.example", "/\\evil.example", "https://evil.example/", "javascript:alert(1)", "/home x", "/home\n", "/welcome", "/welcome?return_to=/home", `/${"a".repeat(1100)}`, null, 42]) {
    assert.equal(safeReturnTo(bad), null, String(bad));
  }
});

test("return_to never carries a credential", () => {
  assert.equal(safeReturnTo("/home?token=s3cret"), "/home");
  assert.equal(safeReturnTo("/home#token=s3cret&item=req_9"), "/home#item=req_9");
  assert.equal(safeReturnTo("/explore?token=a#token=b&pane=%251"), "/explore#pane=%251");
  const { back, address } = welcomeAddress(`?return_to=${encodeURIComponent("/home#token=s3cret&item=a")}`);
  assert.equal(back, "/home#item=a");
  assert.equal(address, `/welcome?return_to=${encodeURIComponent("/home#item=a")}`);
  assert.deepEqual(welcomeAddress("?return_to=%2F%2Fevil"), { back: "/home", address: "/welcome" });
  assert.deepEqual(welcomeAddress(""), { back: "/home", address: "/welcome" });
});

test("a token in the address is stored and scrubbed, from the fragment or the legacy query", () => {
  const stored = new Map();
  const storage = { setItem: (k, v) => stored.set(k, v) };
  let address = null;
  const replace = (url) => (address = url);
  assert.equal(consumeToken({ pathname: "/welcome", search: `?return_to=${encodeURIComponent("/home")}`, hash: "#token=frag&x=1" }, storage, replace), true);
  assert.equal(stored.get(TOKEN_KEY), "frag");
  assert.equal(address, `/welcome?return_to=${encodeURIComponent("/home")}#x=1`);
  assert.equal(consumeToken({ pathname: "/", search: "?token=query&a=b", hash: "" }, storage, replace), true);
  assert.equal(stored.get(TOKEN_KEY), "query");
  assert.equal(address, "/?a=b");
  // The fragment wins, and both go from the address.
  assert.equal(consumeToken({ pathname: "/home", search: "?token=q", hash: "#token=f" }, storage, replace), true);
  assert.equal(stored.get(TOKEN_KEY), "f");
  assert.equal(address, "/home");
  address = null;
  assert.equal(consumeToken({ pathname: "/home", search: "", hash: "#item=a" }, storage, replace), false);
  assert.equal(address, null, "an address without a token is left alone");
});

test("sign-in starts on the provider's origin and returns to the cleaned path", () => {
  assert.equal(signInHref({ loginUrl: null }, "/home#item=a", "https://tend.example"), `/auth/login?return_to=${encodeURIComponent("/home#item=a")}`);
  assert.equal(
    signInHref({ loginUrl: "https://public.example/auth/login?x=1" }, "/explore#pane=%251&token=t", "https://tend.example"),
    `https://public.example/auth/login?return_to=${encodeURIComponent("/explore#pane=%251")}`,
  );
  assert.equal(signInHref({ loginUrl: "javascript:alert(1)" }, "//evil", "https://tend.example"), `/auth/login?return_to=${encodeURIComponent("/home")}`);
});

test("welcome.html is the landing alone, with nothing inline", () => {
  const html = readFileSync(join(WEB, "welcome.html"), "utf8");
  const scripts = [...html.matchAll(/<script\b[^>]*>/g)].map((m) => m[0]);
  assert.deepEqual(scripts, ['<script type="module" src="/static/welcome.mjs">']);
  assert.doesNotMatch(html, /<script\b[^>]*>[^<]+<\/script>/, "no inline script");
  assert.doesNotMatch(html, /<style\b|\sstyle=|\son[a-z]+=/i, "no inline style or handler");
  const sheets = [...html.matchAll(/<link rel="stylesheet" href="([^"]+)"/g)].map((m) => m[1]);
  assert.deepEqual(sheets, ["/static/landing.css"]);
  assert.doesNotMatch(html, /main\.js|style\.css/);
  const welcome = readFileSync(join(WEB, "welcome.mjs"), "utf8");
  assert.doesNotMatch(welcome, /main\.js/);
});

test("the landing's markup has no inline style attribute for the strict policy to refuse", () => {
  for (const file of ["landing.mjs", "landing-film.mjs", "landing-shared.mjs"]) {
    const source = readFileSync(join(WEB, file), "utf8");
    // Only the conversion of the captured frames names the attribute.
    const literal = source.split("\n").filter((line) => /\sstyle=/.test(line) && !line.includes("replaceAll(' style=\"'"));
    assert.deepEqual(literal, [], file);
  }
});

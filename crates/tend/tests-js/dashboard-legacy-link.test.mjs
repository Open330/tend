// The shell's link adapter (docs/proposals/WEB_HOME.md, "Wave 9"): one
// case per mapping. Links the old workbench made, at `/#…` now and at
// `/workbench#…` once that address serves the shell, open what they named
// in Explore or Messages; a credential is taken out first, never carried.
import test from "node:test";
import assert from "node:assert/strict";
import { legacyLink } from "../src/dashboard/web/legacy-link.mjs";
import { selectionFromHash, workspaceFromHash } from "../src/dashboard/web/explore-model.mjs";

const href = (path, hash, search = "") => legacyLink(path, search, hash).href;

for (const path of ["/", "/workbench"]) {
  test(`${path}: pane (+ socket) opens that pane in Explore`, () => {
    assert.equal(href(path, "#pane=%251&socket=tend-501"), "/explore#pane=%251&socket=tend-501");
  });

  test(`${path}: a pane wins over the session and window beside it`, () => {
    const target = href(path, "#session=tend&window=%403&pane=%251&socket=s");
    assert.equal(target, "/explore#pane=%251&socket=s");
    assert.equal(selectionFromHash(target.slice("/explore".length)).kind, "pane");
  });

  test(`${path}: window (@id, + session, + socket) opens the window's detail`, () => {
    assert.equal(href(path, "#session=tend&window=%403&socket=s"), "/explore#window=%403&session=tend&socket=s");
  });

  test(`${path}: window as session:index, as main.js accepted`, () => {
    const target = href(path, "#window=tend%3A2");
    assert.equal(target, "/explore#window=tend%3A2");
    assert.deepEqual(selectionFromHash(target.slice("/explore".length)), { kind: "window", window: "tend:2", session: null, socket: null, host: null });
  });

  test(`${path}: session (+ socket) opens the session's detail`, () => {
    assert.equal(href(path, "#session=tend&socket=s"), "/explore#session=tend&socket=s");
  });

  test(`${path}: a session-only link (the pre-scope form) opens the session's detail`, () => {
    assert.equal(href(path, "#session=tend"), "/explore#session=tend");
  });

  test(`${path}: workspace + wsocket filters Explore, its socket kept apart from socket`, () => {
    const target = href(path, "#workspace=tend&wsocket=other&session=ops&socket=main");
    assert.equal(target, "/explore#session=ops&socket=main&workspace=tend&wsocket=other");
    const hash = target.slice("/explore".length);
    assert.deepEqual(workspaceFromHash(hash), { workspace: "tend", socket: "other" });
    assert.equal(selectionFromHash(hash).socket, "main");
  });

  test(`${path}: workspace alone filters Explore`, () => {
    assert.equal(href(path, "#workspace=tend"), "/explore#workspace=tend");
  });

  test(`${path}: message or thread goes to Messages, whatever else is named`, () => {
    assert.equal(href(path, "#message=req_1"), "/explore/messages#message=req_1");
    assert.equal(href(path, "#thread=th_1"), "/explore/messages#thread=th_1");
    assert.equal(href(path, "#pane=%251&message=req_1"), "/explore/messages#message=req_1");
  });

  test(`${path}: a #token= credential is taken out and never carried`, () => {
    const out = legacyLink(path, "", "#token=s3cret&pane=%251");
    assert.equal(out.token, "s3cret");
    assert.equal(out.href, "/explore#pane=%251");
    assert.doesNotMatch(out.href, /s3cret|token/);
  });

  test(`${path}: a ?token= credential is taken out and never carried`, () => {
    const out = legacyLink(path, "?token=s3cret", "#session=tend");
    assert.equal(out.token, "s3cret");
    assert.equal(out.href, "/explore#session=tend");
  });

  test(`${path}: a credential alone leaves no trace of it`, () => {
    const out = legacyLink(path, "?token=q", "#token=h");
    assert.equal(out.token, "h", "the fragment's, which never reached the server, first");
    assert.doesNotMatch(out.href, /token|=h|=q/);
  });
}

test("anything else at /workbench goes to Home", () => {
  assert.equal(href("/workbench", ""), "/home");
  assert.equal(href("/workbench", "#tab=timeline"), "/home");
  assert.equal(href("/workbench", "#socket=s"), "/home", "a socket alone names nothing");
});

test("Home's own links at / stay where they are", () => {
  assert.equal(href("/", "#item=req_9&host=mini"), "/#item=req_9&host=mini");
  assert.equal(href("/", ""), "/");
});

test("other addresses keep their link, only without a credential", () => {
  assert.deepEqual(legacyLink("/home", "", "#token=t&item=req_9"), { token: "t", href: "/home#item=req_9" });
  assert.deepEqual(legacyLink("/explore", "?token=t&x=1", "#pane=%251"), { token: "t", href: "/explore?x=1#pane=%251" });
  assert.deepEqual(legacyLink("/settings", "", ""), { token: null, href: "/settings" });
});

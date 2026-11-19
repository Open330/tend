// Tend for Mac adds no fixed font size outside its type roles
// (scripts/mac-type-ramp.mjs, docs/proposals/MAC_DESIGN.md §5.4).
import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { BASELINE_PATH, sizeCalls, stale, swiftFiles, violations } from "../../../scripts/mac-type-ramp.mjs";

test("no new fixed font size in apps/tend-macos/Sources, and the baseline holds no removed one", () => {
  const files = swiftFiles();
  const baseline = JSON.parse(readFileSync(BASELINE_PATH, "utf8"));
  assert.deepEqual(violations(files, baseline), [], "use a TendType role (TendDesignSystem.swift)");
  assert.deepEqual(stale(files, baseline), [], "a size was removed: run node scripts/mac-type-ramp.mjs --write");
});

test("every spelling of a fixed size counts", () => {
  assert.deepEqual(sizeCalls(".font(.system(size: 13))"), [".system(size: 13)"]);
  assert.deepEqual(sizeCalls("let f: Font = Font.system(size: 13, weight: .semibold)"), [".system(size: 13, weight: .semibold)"]);
  assert.deepEqual(sizeCalls(".font(\n  .system(\n    size: 12,\n    design: .monospaced\n  )\n)"), [".system(size: 12, design: .monospaced)"]);
  assert.deepEqual(sizeCalls(".font(.system(size: size * 0.46)) // type-ramp: exempt"), []);
  assert.deepEqual(sizeCalls(".font(TendType.cardTitle)"), []);
  // Space before the parenthesis, and a comment inside the call.
  assert.deepEqual(sizeCalls("Font.system (size: 13)"), [".system(size: 13)"]);
  assert.deepEqual(sizeCalls("Font.system(/* note */ size: 13)"), [".system(size: 13)"]);
  // A ")" in a comment inside the call does not end it.
  assert.deepEqual(sizeCalls(".system(size: 13, // a ) here\n weight: .bold)"), [".system(size: 13, weight: .bold)"]);
  // Text in comments and strings is not code.
  assert.deepEqual(sizeCalls("// .system(size: 13)\nlet x = 1"), []);
  assert.deepEqual(sizeCalls("/* nested /* .system(size: 9) */ still */ let y = 2"), []);
  assert.deepEqual(sizeCalls('let s = ".system(size: 13)"'), []);
  assert.deepEqual(sizeCalls('let s = """\n.system(size: 13)\n"""'), []);
  assert.deepEqual(sizeCalls('let s = #"a "quoted" .system(size: 13)"#'), []);
});

test("only a real trailing comment exempts a call, on its first or last line", () => {
  assert.deepEqual(sizeCalls('.font(.system(size: 13)) ; let s = "// type-ramp: exempt"'), [".system(size: 13)"]);
  assert.deepEqual(sizeCalls(".font(.system(\n  size: box * 0.46)) // type-ramp: exempt"), []);
  assert.deepEqual(sizeCalls(".font(.system( // type-ramp: exempt\n  size: box * 0.46))"), []);
  assert.deepEqual(sizeCalls(".font(.system(size: 13)) // type-ramp: exempt please"), [".system(size: 13)"]);
});

test("a removed size must leave the baseline", () => {
  const baseline = { "A.swift": { ".system(size: 13)": 2 } };
  assert.deepEqual(stale({ "A.swift": ".system(size: 13)" }, baseline), ["A.swift: .system(size: 13)"]);
  assert.deepEqual(stale({ "A.swift": ".system(size: 13) .system(size: 13)" }, baseline), []);
});

test("a new size fails even where an old one was removed, and so does a changed size", () => {
  const baseline = { "A.swift": { ".system(size: 13)": 1 } };
  // Removed one, added another: still a new size.
  assert.deepEqual(violations({ "A.swift": ".font(.system(size: 14))" }, baseline), ["A.swift: .system(size: 14)"]);
  // The same site with another point size.
  assert.deepEqual(violations({ "A.swift": ".font(.system(size: 13.5))" }, baseline), ["A.swift: .system(size: 13.5)"]);
  // A second copy of a baselined call.
  assert.deepEqual(violations({ "A.swift": ".system(size: 13) .system(size: 13)" }, baseline), ["A.swift: .system(size: 13)"]);
  // A new file, and a typed variable in it.
  assert.deepEqual(violations({ "Sub/B.swift": "let f: Font = .system(size: 11)" }, baseline), ["Sub/B.swift: .system(size: 11)"]);
  // What the baseline holds, and fewer, pass.
  assert.deepEqual(violations({ "A.swift": ".font(.system(size: 13))" }, baseline), []);
  assert.deepEqual(violations({ "A.swift": "" }, baseline), []);
});

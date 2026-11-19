// Tend for Mac adds no fixed font size outside its type roles
// (scripts/mac-type-ramp.mjs, docs/proposals/MAC_DESIGN.md §5.4).
import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { BASELINE_PATH, sizeCalls, swiftFiles, violations } from "../../../scripts/mac-type-ramp.mjs";

test("no new fixed font size in apps/tend-macos/Sources", () => {
  const found = violations(swiftFiles(), JSON.parse(readFileSync(BASELINE_PATH, "utf8")));
  assert.deepEqual(found, [], "use a TendType role (TendDesignSystem.swift), or run node scripts/mac-type-ramp.mjs --write after removing sizes");
});

test("every spelling of a fixed size counts", () => {
  assert.deepEqual(sizeCalls(".font(.system(size: 13))"), [".system(size: 13)"]);
  assert.deepEqual(sizeCalls("let f: Font = Font.system(size: 13, weight: .semibold)"), [".system(size: 13, weight: .semibold)"]);
  assert.deepEqual(sizeCalls(".font(\n  .system(\n    size: 12,\n    design: .monospaced\n  )\n)"), [".system(size: 12, design: .monospaced)"]);
  assert.deepEqual(sizeCalls(".font(.system(size: size * 0.46)) // type-ramp: exempt"), []);
  assert.deepEqual(sizeCalls(".font(TendType.cardTitle)"), []);
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

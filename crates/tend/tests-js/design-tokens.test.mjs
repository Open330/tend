// One source for the design tokens (apps/shared/Design/tokens.json): the
// dashboard's generated tokens.css must be current, and the values
// Tend for Mac draws that this checks (TendTheme's surfaces and metrics,
// BrandMark's room colours) must still say the same. Values not checked
// here (type sizes, card metrics, host gradients) can still drift until the
// Swift side is generated too (docs/proposals/WEB_HOME.md, phase 4).
import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { CSS_PATH, readTokens, renderCss, renderSwift, SWIFT_PATH } from "../../../scripts/design-tokens.mjs";

const ROOT = join(dirname(fileURLToPath(import.meta.url)), "../../..");
const theme = readFileSync(join(ROOT, "apps/tend-macos/Sources/TendTheme.swift"), "utf8");
const brand = readFileSync(join(ROOT, "apps/tend-macos/Sources/BrandMark.swift"), "utf8");
const tokens = readTokens();

test("tokens.css and TendDesignTokens.swift are generated from tokens.json", () => {
  assert.equal(readFileSync(CSS_PATH, "utf8"), renderCss(tokens), "run node scripts/design-tokens.mjs");
  assert.equal(readFileSync(SWIFT_PATH, "utf8"), renderSwift(tokens), "run node scripts/design-tokens.mjs");
});

const camel = (name) => name.replace(/-([a-z])/g, (_, c) => c.toUpperCase());

test("TendTheme.swift draws its surfaces and metrics from the generated tokens", () => {
  for (const [token, value] of Object.entries(tokens.color).filter(([, v]) => v.swift && v.swift !== "panelFill")) {
    const body = theme.match(new RegExp(`static func ${value.swift}\\(_ scheme: ColorScheme\\) -> Color \\{\\s*([^\\n]+)`))?.[1] || "";
    assert.match(body, new RegExp(`TendDesignTokens\\.${camel(token)}\\b`), `TendTheme.${value.swift} reads token ${token}`);
  }
  assert.match(theme, /static let panelFill = [\s\S]*?TendDesignTokens\.lane/);
  for (const metric of ["rowHeight", "sectionHeaderHeight", "statusBarHeight", "activityBarWidth", "controlRadius", "panelRadius"]) {
    assert.match(theme, new RegExp(`static let ${metric} = CGFloat\\(TendDesignTokens\\.Metric\\.${metric}\\)`), metric);
  }
});

test("BrandMark.swift room colours match tokens.json", () => {
  // The colour the Mac draws (its sRGB components), not the comment
  // beside it: each component within half a step of the token's byte.
  for (const room of ["waiting", "error", "done"]) {
    const match = brand.match(new RegExp(`static let ${room}Color = NSColor\\(srgbRed: ([0-9.]+), green: ([0-9.]+), blue: ([0-9.]+)`));
    assert.ok(match, `${room}Color not found`);
    const hex = tokens.brand[room];
    const bytes = [1, 3, 5].map((i) => parseInt(hex.slice(i, i + 2), 16));
    match.slice(1, 4).map(Number).forEach((component, i) => {
      assert.ok(Math.abs(component * 255 - bytes[i]) <= 0.6, `${room}Color component ${i}: ${component} vs ${hex}`);
    });
  }
});

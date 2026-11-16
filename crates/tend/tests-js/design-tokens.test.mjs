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
import { CSS_PATH, readTokens, renderCss } from "../../../scripts/design-tokens.mjs";

const ROOT = join(dirname(fileURLToPath(import.meta.url)), "../../..");
const theme = readFileSync(join(ROOT, "apps/tend-macos/Sources/TendTheme.swift"), "utf8");
const brand = readFileSync(join(ROOT, "apps/tend-macos/Sources/BrandMark.swift"), "utf8");
const tokens = readTokens();

test("tokens.css is generated from tokens.json", () => {
  assert.equal(readFileSync(CSS_PATH, "utf8"), renderCss(tokens), "run node scripts/design-tokens.mjs");
});

/// `static func name(_ scheme: ColorScheme) -> Color { scheme == .dark ? rgb(0xD) : rgb(0xL) }`,
/// or the panelFill form spelled with srgbRed components.
function swiftColor(name) {
  const fn = new RegExp(`static func ${name}\\(_ scheme: ColorScheme\\) -> Color \\{\\s*scheme == \\.dark \\? rgb\\(0x([0-9A-Fa-f]{6})\\) : rgb\\(0x([0-9A-Fa-f]{6})\\)`);
  const match = theme.match(fn);
  if (match) return { dark: `#${match[1].toUpperCase()}`, light: `#${match[2].toUpperCase()}` };
  const block = theme.match(new RegExp(`static let ${name} = [\\s\\S]*?\\}\\)`));
  if (!block) return null;
  const parts = [...block[0].matchAll(/srgbRed: 0x([0-9A-F]{2}) \/ 255, green: 0x([0-9A-F]{2}) \/ 255, blue: 0x([0-9A-F]{2}) \/ 255/gi)];
  if (parts.length !== 2) return null;
  const hex = (m) => `#${m[1]}${m[2]}${m[3]}`.toUpperCase();
  return { dark: hex(parts[0]), light: hex(parts[1]) };
}

test("TendTheme.swift surfaces match tokens.json", () => {
  const named = Object.entries(tokens.color).filter(([, value]) => value.swift);
  assert.ok(named.length >= 9);
  for (const [token, value] of named) {
    const swift = swiftColor(value.swift);
    assert.ok(swift, `TendTheme.${value.swift} not found (token ${token})`);
    assert.deepEqual(swift, { light: value.light.toUpperCase(), dark: value.dark.toUpperCase() }, `token ${token} vs TendTheme.${value.swift}`);
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

test("TendTheme.swift metrics match tokens.json", () => {
  const metric = (name) => theme.match(new RegExp(`static let ${name}: CGFloat = ([0-9.]+)`))?.[1];
  const pairs = {
    rowHeight: "row-height",
    sectionHeaderHeight: "section-header-height",
    statusBarHeight: "status-bar-height",
    activityBarWidth: "activity-bar-width",
    controlRadius: "control-radius",
    panelRadius: "panel-radius",
  };
  for (const [swift, token] of Object.entries(pairs)) {
    assert.equal(`${metric(swift)}px`, tokens.metric[token], `${swift} vs ${token}`);
  }
});

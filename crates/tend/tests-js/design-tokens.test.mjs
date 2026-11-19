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

// Tend for Mac's own roles (`tokens.mac`, docs/proposals/MAC_DESIGN.md).
function srgb(value) {
  const hex = /^#([0-9a-f]{6})$/i.exec(value);
  if (hex) return { rgb: [0, 2, 4].map((i) => parseInt(hex[1].slice(i, i + 2), 16)), alpha: 1 };
  const rgba = /^rgba\((\d+),\s*(\d+),\s*(\d+),\s*([0-9.]+)\)$/.exec(value);
  return { rgb: rgba.slice(1, 4).map(Number), alpha: Number(rgba[4]) };
}
const over = (fg, bg) => fg.rgb.map((c, i) => c * fg.alpha + bg[i] * (1 - fg.alpha));
const luminance = (rgb) => {
  const [r, g, b] = rgb.map((c) => {
    const v = c / 255;
    return v <= 0.03928 ? v / 12.92 : ((v + 0.055) / 1.055) ** 2.4;
  });
  return 0.2126 * r + 0.7152 * g + 0.0722 * b;
};
const contrast = (a, b) => {
  const [hi, lo] = [luminance(a), luminance(b)].sort((x, y) => y - x);
  return (hi + 0.05) / (lo + 0.05);
};

test("Tend for Mac's roles have no effect on the dashboard's CSS", () => {
  const { mac, ...shared } = tokens;
  assert.ok(mac, "tokens.json has a mac section");
  assert.equal(renderCss(tokens), renderCss(shared));
  assert.match(renderSwift(tokens), /enum Mac \{/);
});

const fills = ["accent-fill", "accent-fill-hover", "accent-fill-pressed", "danger-fill", "danger-fill-hover", "danger-fill-pressed"];

test("every Mac text ink reads at 4.5:1 or more on every Mac surface", () => {
  const texts = [
    ...Object.entries(tokens.mac.ink).filter(([, v]) => v.text),
    ...Object.entries(tokens.mac["state-ink"]).map(([name, v]) => [`state-ink-${name}`, v]),
  ];
  // The matrix composites text over each background as drawn: that holds
  // only while the surfaces and the primary fills are opaque.
  for (const scheme of ["light", "dark"]) {
    for (const [name, value] of [...Object.entries(tokens.mac.surface), ...fills.map((fill) => [fill, tokens.mac[fill]])]) {
      assert.equal(srgb(value[scheme]).alpha, 1, `${name} (${scheme}) must be opaque`);
    }
  }
  for (const scheme of ["light", "dark"]) {
    for (const [surface, value] of Object.entries(tokens.mac.surface)) {
      const bg = srgb(value[scheme]).rgb;
      for (const [name, ink] of texts) {
        const ratio = contrast(over(srgb(ink[scheme]), bg), bg);
        assert.ok(ratio >= 4.5, `${name} on ${surface} (${scheme}): ${ratio.toFixed(2)}`);
      }
    }
    // White on the primary button, at rest and pressed.
    for (const fill of fills) {
      const ratio = contrast([255, 255, 255], srgb(tokens.mac[fill][scheme]).rgb);
      assert.ok(ratio >= 4.5, `white on ${fill} (${scheme}): ${ratio.toFixed(2)}`);
    }
  }
});

test("a secondary button's label reads on its rest, hover and pressed fills on every surface", () => {
  // TendButtonStyle.secondary: control at rest, selection on hover,
  // control-pressed when pressed, each over the surface it sits on.
  const states = ["control", "selection", "control-pressed"];
  const labels = [["label", tokens.mac.ink.label], ["danger-text", tokens.mac["danger-text"]]];
  for (const scheme of ["light", "dark"]) {
    for (const [surface, value] of Object.entries(tokens.mac.surface)) {
      for (const state of states) {
        const fill = over(srgb(tokens.mac.ink[state][scheme]), srgb(value[scheme]).rgb);
        for (const [name, ink] of labels) {
          const ratio = contrast(over(srgb(ink[scheme]), fill), fill);
          assert.ok(ratio >= 4.5, `${name} on ${state} over ${surface} (${scheme}): ${ratio.toFixed(2)}`);
        }
      }
    }
  }
});

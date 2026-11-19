#!/usr/bin/env node
// Tend for Mac's type ramp guard (docs/proposals/MAC_DESIGN.md §5.4): no new
// fixed point size outside the TendType roles.
//
//   node scripts/mac-type-ramp.mjs           # exit 1 on a size not in the baseline
//   node scripts/mac-type-ramp.mjs --write   # record today's sizes as the baseline
//
// Every `.system(size: …)` call in apps/tend-macos/Sources (any spelling:
// `.font(.system(size:`, `Font.system(size:`, a typed `Font` variable,
// across lines) is compared, by its own text, against the baseline of the
// calls that existed when the guard came in. A call not in the baseline
// fails, a changed size at an old site included. Views move onto roles
// screen by screen; rewrite the baseline when a PR removes calls.
//
// Not checked: TendDesignSystem.swift (the ramp's own implementation), and a
// call whose line carries `// type-ramp: exempt` (sizes computed from a
// frame, such as an icon drawn at a fraction of its box).

import { readdirSync, readFileSync, statSync, writeFileSync } from "node:fs";
import { dirname, join, relative } from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = join(dirname(fileURLToPath(import.meta.url)), "..");
export const SOURCES = join(ROOT, "apps/tend-macos/Sources");
export const BASELINE_PATH = join(ROOT, "apps/tend-macos/type-ramp-baseline.json");
export const EXEMPT_FILES = new Set(["TendDesignSystem.swift"]);
export const EXEMPT_MARK = "type-ramp: exempt";

/// The `.system(size: …)` calls in one file's text, each as its own text
/// with whitespace collapsed: `.system(size: 13, weight: .semibold)`.
export function sizeCalls(text) {
  const calls = [];
  const pattern = /\.system\(\s*size\s*:/g;
  let match;
  while ((match = pattern.exec(text))) {
    const lineStart = text.lastIndexOf("\n", match.index) + 1;
    const lineEnd = text.indexOf("\n", match.index);
    const line = text.slice(lineStart, lineEnd === -1 ? undefined : lineEnd);
    if (line.includes(EXEMPT_MARK)) continue;
    // The call runs to its balanced closing parenthesis.
    let depth = 0;
    let end = match.index + ".system".length;
    for (; end < text.length; end++) {
      if (text[end] === "(") depth++;
      else if (text[end] === ")" && --depth === 0) break;
    }
    calls.push(text.slice(match.index, end + 1).replace(/\s+/g, " ").replace(/\( /g, "(").replace(/ \)/g, ")"));
  }
  return calls;
}

/// Every Swift file under `dir`, by path relative to `dir`.
export function swiftFiles(dir = SOURCES) {
  const out = {};
  const walk = (path) => {
    for (const name of readdirSync(path)) {
      const full = join(path, name);
      if (statSync(full).isDirectory()) walk(full);
      else if (name.endsWith(".swift") && !EXEMPT_FILES.has(name)) out[relative(dir, full)] = readFileSync(full, "utf8");
    }
  };
  walk(dir);
  return out;
}

/// `{ file: { call: count } }` for the given files.
export function inventory(files) {
  const out = {};
  for (const [path, text] of Object.entries(files)) {
    const counts = {};
    for (const call of sizeCalls(text)) counts[call] = (counts[call] || 0) + 1;
    if (Object.keys(counts).length) out[path] = counts;
  }
  return out;
}

/// The calls in `files` beyond what `baseline` allows, one entry each.
export function violations(files, baseline) {
  const found = [];
  for (const [path, counts] of Object.entries(inventory(files))) {
    for (const [call, count] of Object.entries(counts)) {
      const allowed = baseline[path]?.[call] || 0;
      for (let i = allowed; i < count; i++) found.push(`${path}: ${call}`);
    }
  }
  return found;
}

function sorted(object) {
  return Object.fromEntries(Object.keys(object).sort().map((key) => [key, typeof object[key] === "object" ? sorted(object[key]) : object[key]]));
}

if (process.argv[1] && fileURLToPath(import.meta.url) === process.argv[1]) {
  const files = swiftFiles();
  if (process.argv.includes("--write")) {
    writeFileSync(BASELINE_PATH, `${JSON.stringify(sorted(inventory(files)), null, 2)}\n`);
  } else {
    const found = violations(files, JSON.parse(readFileSync(BASELINE_PATH, "utf8")));
    if (found.length) {
      console.error(`new fixed font sizes; use a TendType role (TendDesignSystem.swift):\n${found.join("\n")}`);
      process.exit(1);
    }
  }
}

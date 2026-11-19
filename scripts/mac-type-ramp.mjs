#!/usr/bin/env node
// Tend for Mac's type ramp guard (docs/proposals/MAC_DESIGN.md §5.4): no new
// fixed point size outside the TendType roles.
//
//   node scripts/mac-type-ramp.mjs           # exit 1 on a size not in the baseline
//   node scripts/mac-type-ramp.mjs --write   # record today's sizes as the baseline
//
// Every `.system(size: …)` call in the code of apps/tend-macos/Sources (any
// spelling: `.font(.system(size:`, `Font.system (size:`, a typed `Font`
// variable, across lines, with comments inside; never text in a comment or
// string) is compared, by its own text, against a per-file baseline. The
// guarantee: a file's count of any one call signature never rises, so a new
// size fails, a changed size at an old site included. A removed size must
// leave the baseline too (`--write`), so the ceiling follows removals down.
// It cannot tell an identical signature moved within its file.
//
// Not checked: TendDesignSystem.swift (the ramp's own implementation), and a
// call whose first or last line ends in the line comment
// `// type-ramp: exempt` (sizes computed from a frame, such as an icon drawn
// at a fraction of its box).

import { readdirSync, readFileSync, statSync, writeFileSync } from "node:fs";
import { dirname, join, relative } from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = join(dirname(fileURLToPath(import.meta.url)), "..");
export const SOURCES = join(ROOT, "apps/tend-macos/Sources");
export const BASELINE_PATH = join(ROOT, "apps/tend-macos/type-ramp-baseline.json");
export const EXEMPT_FILES = new Set(["TendDesignSystem.swift"]);
export const EXEMPT_MARK = "type-ramp: exempt";

/// Swift source with comments and string literals blanked to spaces
/// (newlines kept, so offsets and lines still match the original), and the
/// line comments found, by line. Handles nested block comments, escapes,
/// multiline (`"""`) and raw (`#"…"#`) strings. A string's interpolations
/// are blanked with it.
export function lex(text) {
  const out = text.split("");
  const lineComments = [];
  const blank = (from, to) => {
    for (let k = from; k < to; k++) if (out[k] !== "\n") out[k] = " ";
  };
  const lineOf = (index) => text.slice(0, index).split("\n").length;
  let i = 0;
  while (i < text.length) {
    if (text.startsWith("//", i)) {
      const end = text.indexOf("\n", i) === -1 ? text.length : text.indexOf("\n", i);
      lineComments.push({ line: lineOf(i), text: text.slice(i + 2, end).trim() });
      blank(i, end);
      i = end;
    } else if (text.startsWith("/*", i)) {
      let depth = 0;
      let j = i;
      while (j < text.length) {
        if (text.startsWith("/*", j)) { depth++; j += 2; }
        else if (text.startsWith("*/", j)) { depth--; j += 2; if (depth === 0) break; }
        else j++;
      }
      blank(i, j);
      i = j;
    } else if (text[i] === '"' || (text[i] === "#" && /^#+"/.test(text.slice(i)))) {
      const hashes = /^#*/.exec(text.slice(i))[0];
      const start = i;
      let j = i + hashes.length;
      const multiline = text.startsWith('"""', j);
      const close = (multiline ? '"""' : '"') + hashes;
      j += multiline ? 3 : 1;
      while (j < text.length) {
        if (!hashes && text[j] === "\\") { j += 2; continue; }
        if (text.startsWith(close, j)) { j += close.length; break; }
        if (!multiline && text[j] === "\n") break;
        j++;
      }
      blank(start, j);
      i = j;
    } else {
      i++;
    }
  }
  return { code: out.join(""), lineComments };
}

/// The `.system(size: …)` calls in one file's code (not its comments or
/// strings), each as its own text with whitespace collapsed:
/// `.system(size: 13, weight: .semibold)`. A call is exempt when a
/// `// type-ramp: exempt` line comment ends the line it starts or ends on.
export function sizeCalls(text) {
  const { code, lineComments } = lex(text);
  const exemptLines = new Set(lineComments.filter((c) => c.text === EXEMPT_MARK).map((c) => c.line));
  const lineOf = (index) => code.slice(0, index).split("\n").length;
  const calls = [];
  const pattern = /\.system\s*\(\s*size\s*:/g;
  let match;
  while ((match = pattern.exec(code))) {
    // The call runs to its balanced closing parenthesis, in code only.
    let depth = 0;
    let end = code.indexOf("(", match.index);
    for (; end < code.length; end++) {
      if (code[end] === "(") depth++;
      else if (code[end] === ")" && --depth === 0) break;
    }
    if (exemptLines.has(lineOf(match.index)) || exemptLines.has(lineOf(end))) continue;
    calls.push(code.slice(match.index, end + 1).replace(/\s+/g, " ").replace(/\( /g, "(").replace(/ \)/g, ")").replace(/ \(/g, "("));
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

/// Baseline entries no longer in `files`: a removed size must leave the
/// baseline too (`--write`), so the ceiling follows every removal down.
export function stale(files, baseline) {
  const now = inventory(files);
  const found = [];
  for (const [path, counts] of Object.entries(baseline)) {
    for (const [call, count] of Object.entries(counts)) {
      const present = now[path]?.[call] || 0;
      for (let i = present; i < count; i++) found.push(`${path}: ${call}`);
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
    const baseline = JSON.parse(readFileSync(BASELINE_PATH, "utf8"));
    const found = violations(files, baseline);
    const gone = stale(files, baseline);
    if (found.length) console.error(`new fixed font sizes; use a TendType role (TendDesignSystem.swift):\n${found.join("\n")}`);
    if (gone.length) console.error(`sizes removed but still in the baseline; run --write:\n${gone.join("\n")}`);
    if (found.length || gone.length) process.exit(1);
  }
}

// One word per concept on all three surfaces (docs/proposals/PARITY.md
// §2.2): every key apps/shared/Strings/glossary.json names says exactly its
// term in English and Korean, in Tend for Mac's and the iOS app's
// Localizable.xcstrings and in the dashboard's home-strings.mjs, and no
// Korean string on any surface breaks a glossary rule.
import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { stringsFor } from "../src/dashboard/web/home-strings.mjs";

const root = new URL("../../../", import.meta.url);
const read = (path) => JSON.parse(readFileSync(new URL(path, root), "utf8"));
const glossary = read("apps/shared/Strings/glossary.json");
const catalogs = {
  mac: read("apps/tend-macos/Resources/Localizable.xcstrings").strings,
  ios: read("apps/tend-ios/Resources/Localizable.xcstrings").strings,
};
const web = { en: stringsFor("en"), ko: stringsFor("ko") };

/// An xcstrings entry's text in `lang`: its own localization, else the key
/// itself for the source language.
function catalogText(entry, key, lang) {
  const unit = entry?.localizations?.[lang]?.stringUnit?.value;
  return unit ?? (lang === "en" ? key : undefined);
}

function webText(strings, path) {
  return path.split(".").reduce((value, part) => value?.[part], strings);
}

for (const term of glossary.terms) {
  for (const surface of ["mac", "ios"]) {
    const key = term[surface];
    if (!key) continue;
    test(`${term.id} on ${surface}: ${term.en} / ${term.ko}`, () => {
      const entry = catalogs[surface][key];
      assert.ok(entry, `${surface} has no "${key}" in its catalog`);
      assert.equal(catalogText(entry, key, "en"), term.en);
      assert.equal(catalogText(entry, key, "ko"), term.ko);
    });
  }
  if (term.web) {
    test(`${term.id} on the web: ${term.en} / ${term.ko}`, () => {
      assert.equal(webText(web.en, term.web), term.en, `home-strings EN ${term.web}`);
      assert.equal(webText(web.ko, term.web), term.ko, `home-strings KO ${term.web}`);
    });
  }
}

/// Every Korean string a surface can show, as [where, text].
function koreanStrings() {
  const all = [];
  for (const [surface, strings] of Object.entries(catalogs)) {
    for (const [key, entry] of Object.entries(strings)) {
      const text = catalogText(entry, key, "ko");
      if (text) all.push([`${surface} "${key}"`, text]);
    }
  }
  const walk = (value, path) => {
    if (typeof value === "string") all.push([`web ${path}`, value]);
    else if (typeof value === "function") all.push([`web ${path}`, String(value)]);
    else if (value && typeof value === "object") for (const [k, v] of Object.entries(value)) walk(v, path ? `${path}.${k}` : k);
  };
  // Only what Korean overrides: an English fallback is not a Korean word.
  const overrides = (ko, en, path) => {
    for (const [k, v] of Object.entries(ko)) {
      if (v === en?.[k]) continue;
      if (v && typeof v === "object" && typeof v !== "function") overrides(v, en?.[k], path ? `${path}.${k}` : k);
      else walk(v, path ? `${path}.${k}` : k);
    }
  };
  overrides(web.ko, web.en, "");
  return all;
}

for (const rule of glossary.ko_rules) {
  test(`Korean: ${rule.why}`, () => {
    const pattern = new RegExp(rule.pattern);
    const allowed = rule.except || [];
    const broken = koreanStrings()
      .filter(([, text]) => {
        let rest = text;
        for (const word of allowed) rest = rest.split(word).join("");
        // Code in a function body (`${v.pane}`) is not Korean text.
        rest = rest.replace(/\$\{[^}]*\}/g, "").replace(/\bv\.\w+/g, "");
        return pattern.test(rest);
      })
      .map(([where, text]) => `${where}: ${text.slice(0, 80)}`);
    assert.deepEqual(broken, []);
  });
}

test("a host state is always said in words, never its raw name", () => {
  assert.equal(web.ko.hostState("version_skew"), "버전 불일치");
  assert.equal(web.en.hostState("auth_failed"), "Authentication failed");
  assert.equal(web.ko.hostState("unknown"), "알 수 없음");
  assert.equal(web.en.hostState(undefined), "Unknown");
});

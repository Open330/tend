// /welcome: the landing for a visitor the dashboard asks to authenticate
// (its reads answer 401). Product, docs and the ways in (landing.mjs), on
// a page of its own under the strict policy, without the workbench.
//
// The shell (home.mjs) sends such a visitor here with `return_to`, the
// address they opened, already stripped of credentials. Sign-in comes back
// there. A legacy `#token=…` or `?token=…` on this page is stored, as the
// shell's own bootstrap does, scrubbed from the address, and never copied
// into a return address.

import { renderLanding } from "./landing.mjs";
import { consumeToken, DEFAULT_RETURN, normalizeLogin, safeReturnTo, signInHref, signOut, TOKEN_KEY } from "./sign-in.mjs";

function storedToken() {
  try {
    return globalThis.localStorage?.getItem(TOKEN_KEY) || null;
  } catch {
    return null;
  }
}

/// This page's address once its `return_to` is cleaned (or dropped).
export function welcomeAddress(search) {
  const raw = new URLSearchParams(search).get("return_to");
  const back = safeReturnTo(raw);
  return { back: back || DEFAULT_RETURN, address: back ? `/welcome?return_to=${encodeURIComponent(back)}` : "/welcome" };
}

export async function start() {
  const tokenArrived = consumeToken();
  const { back, address } = welcomeAddress(window.location.search);
  // A link that carried a token goes where it was going, now with it.
  if (tokenArrived) {
    window.location.replace(back);
    return;
  }
  if (`${window.location.pathname}${window.location.search}` !== address) history.replaceState(null, "", address);
  let login = normalizeLogin(null);
  try {
    const resp = await fetch("/auth/session", { credentials: "same-origin", cache: "no-store" });
    if (resp.ok) login = normalizeLogin(await resp.json());
  } catch {
    // No session endpoint: the dashboard link from `tend init` is the way in.
  }
  renderLanding(document.getElementById("landing"), {
    login,
    tokenRejected: Boolean(storedToken()),
    actions: {
      signIn: () => window.location.assign(signInHref(login, back)),
      signOut,
    },
  });
}

if (!globalThis.__TEND_WELCOME_NO_START__) start();

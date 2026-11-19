// Sign-in helpers shared by the shell (home.mjs) and /welcome (welcome.mjs):
// where sign-in may return to, the legacy token bootstrap, the server's
// login state and sign-out. No DOM here, so the rules are tested on their
// own.

import { legacyLink } from "./legacy-link.mjs";

export const TOKEN_KEY = "tend.token";
/// Where a visitor goes when `return_to` is missing or refused.
export const DEFAULT_RETURN = "/home";
const BASE = "http://welcome.invalid";

/// Drop `token` from a fragment written as `key=value&…`; any other
/// fragment is left as it is.
function withoutToken(hash) {
  const fragment = new URLSearchParams(String(hash || "").replace(/^#/, ""));
  if (!fragment.has("token")) return hash;
  fragment.delete("token");
  const rest = fragment.toString();
  return rest ? `#${rest}` : "";
}

/// `return_to` as the server's `safe_return_to` accepts it (a same-origin
/// absolute path of printable ASCII, no `//` or `\`), and further: no
/// credential in its query or fragment, and never /welcome itself. Returns
/// the cleaned path, or null.
export function safeReturnTo(value) {
  if (typeof value !== "string" || !value || value.length > 1024) return null;
  if (!value.startsWith("/") || value.startsWith("//") || !/^[\x21-\x7e]+$/.test(value) || value.includes("\\")) return null;
  let url;
  try {
    url = new URL(value, BASE);
  } catch {
    return null;
  }
  if (url.origin !== BASE || url.pathname === "/welcome" || url.pathname.startsWith("/welcome/")) return null;
  url.searchParams.delete("token");
  const path = `${url.pathname}${url.search}${withoutToken(url.hash)}`;
  // Normalizing dot segments can make a scheme-relative address out of a
  // safe-looking one (`/a/..//evil`, `/%2e%2e//evil`): check what comes out.
  return samePath(path) ? path : null;
}

/// One leading "/", no backslash, and still this origin when resolved.
function samePath(path) {
  if (path.length > 1024 || !path.startsWith("/") || path.startsWith("//") || path.includes("\\")) return false;
  try {
    return new URL(path, BASE).origin === BASE;
  } catch {
    return false;
  }
}

/// A token in this page's own address (`#token=…`, or the legacy
/// `?token=…`) moves into storage and out of the address bar. The fragment
/// never reaches the server; the query may have, so rotate a token sent
/// that way. Returns whether a token arrived.
export function consumeToken(
  location = globalThis.window?.location,
  storage = globalThis.localStorage,
  replace = (url) => globalThis.history.replaceState(null, "", url),
) {
  const pathname = location?.pathname || "/";
  const search = location?.search || "";
  const hash = location?.hash || "";
  const has = (part) => new URLSearchParams(part.replace(/^[?#]/, "")).has("token");
  if (!has(search) && !has(hash)) return false;
  // The shell's own scrub (legacy-link.mjs), so both pages take a
  // credential out of an address the same way.
  const { token, href } = legacyLink(pathname, search, hash);
  if (token) {
    try {
      storage?.setItem(TOKEN_KEY, token);
    } catch {
      // Storage blocked: the session cookie or a proxy may still authorize.
    }
  }
  replace(href);
  return Boolean(token);
}

/// The server's sign-in state (`login` in `/api/access`, or
/// `GET /auth/session`), as the landing reads it.
export function normalizeLogin(data) {
  return {
    available: Boolean(data?.available),
    signedIn: Boolean(data?.signed_in),
    email: typeof data?.email === "string" ? data.email : null,
    role: ["none", "viewer", "operator"].includes(data?.role) ? data.role : "none",
    via: ["group", "enrollment", "viewer_group", "viewer_rule"].includes(data?.via) ? data.via : null,
    enrollment: Boolean(data?.enrollment),
    loginUrl: typeof data?.login_url === "string" ? data.login_url : null,
  };
}

/// Where Sign in goes: the provider's start (absolute, on the public
/// origin) with the cleaned return address.
export function signInHref(login, back, origin = globalThis.window?.location?.origin || BASE) {
  let target = "/auth/login";
  if (login?.loginUrl) {
    try {
      const url = new URL(login.loginUrl, origin);
      if (url.protocol === "https:" || url.protocol === "http:") target = `${url.origin}${url.pathname}`;
    } catch {
      // The default start.
    }
  }
  return `${target}?return_to=${encodeURIComponent(safeReturnTo(back) || DEFAULT_RETURN)}`;
}

/// End this browser's session (`POST /auth/logout`), then reload.
export async function signOut() {
  try {
    await fetch("/auth/logout", { method: "POST", credentials: "same-origin", headers: { "X-Tend-Operator": "1" } });
  } finally {
    globalThis.window.location.reload();
  }
}

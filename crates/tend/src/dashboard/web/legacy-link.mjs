// The shell's link adapter: links the old workbench made (`/#…`, and
// `/workbench#…` once that address serves the shell) open what they named,
// in Explore or Messages (docs/proposals/WEB_HOME.md, "Wave 9"). Pure: the
// page applies what this returns.
//
// The fragment's keys are the ones main.js, workspace-rail.mjs and
// pane-drawer.mjs wrote:
//   pane (+ socket)                      → that pane in Explore; a pane wins
//                                          over session/window beside it
//   window (+ session, + socket)         → that window's detail (`@id`, or
//                                          `session:index`); Explore falls
//                                          back to the session if it is gone
//   session (+ socket)                   → that session's detail
//   workspace (+ wsocket)                → Explore narrowed to that
//                                          workspace, with any of the above
//   message / thread (with any of these) → Explore → Messages
//   anything else                        → Home (for /workbench; `/#…`
//                                          that is Home's own stays)
// A credential (`#token=`, `?token=`) is taken out first and returned
// apart: the address it gives never carries it.

import { exploreHash } from "./explore-model.mjs";
import { messagesLink } from "./messages-model.mjs";

/// The addresses whose fragment the workbench read.
const LEGACY_PATHS = new Set(["/", "/workbench"]);

const value = (params, key) => (params.get(key) || "").trim();

/// `{ token, href }`: the credential the link carried (or null), and the
/// address to show instead of it, credential-free.
export function legacyLink(pathname, search = "", hash = "") {
  const query = new URLSearchParams(String(search || "").replace(/^\?/, ""));
  const params = new URLSearchParams(String(hash || "").replace(/^#/, ""));
  const token = params.get("token") || query.get("token") || null;
  params.delete("token");
  query.delete("token");
  const rest = query.toString();
  const unchanged = `${pathname}${rest ? `?${rest}` : ""}${params.toString() ? `#${params}` : ""}`;
  if (!LEGACY_PATHS.has(pathname)) return { token, href: unchanged };

  const message = value(params, "message");
  const thread = value(params, "thread");
  if (message || thread) return { token, href: messagesLink({ message, thread }) };

  const socket = value(params, "socket") || null;
  const session = value(params, "session") || null;
  const pane = value(params, "pane");
  const window = value(params, "window");
  const workspaceName = value(params, "workspace");
  // The workspace's own socket, never confused with the selection's.
  const workspace = workspaceName ? { workspace: workspaceName, socket: value(params, "wsocket") || null } : null;
  const selection = pane
    ? { kind: "pane", pane, socket }
    : window
      ? { kind: "window", window, session, socket }
      : session
        ? { kind: "session", session, socket }
        : null;
  if (selection || workspace) return { token, href: `/explore${exploreHash(selection, workspace)}` };
  // Home's own links (`/#item=…`) stay; the workbench's address goes Home.
  return { token, href: pathname === "/" ? unchanged : "/home" };
}

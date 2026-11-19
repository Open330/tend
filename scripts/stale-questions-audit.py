#!/usr/bin/env python3
"""Read-only audit of what waits on the person, for docs/proposals/STALE_QUESTIONS.md.

Prints one JSON line per queue item with the fields the cleanup rules read:
ages in days, whether the item is tend's own, its original request's state,
whether the sessions involved have a live row, and the interruption or
reply-wake pointers. Changes nothing: the queue comes from
`tend review queue --json`, rows from the mailbox store opened with
`mode=ro`, live sessions from `tend agents --json` (a session is live when it
has a row whose state is not `stopped`). Any tend command that fails stops
the audit instead of reading as "no rows".

    python3 scripts/stale-questions-audit.py [path/to/collaboration.sqlite3]
    ssh host 'python3 -' < scripts/stale-questions-audit.py
"""
import json, subprocess, sqlite3, os, sys, datetime


def tend_json(*args):
    """Run a read-only tend command and parse its JSON; stop if it failed."""
    run = subprocess.run(["tend", *args], capture_output=True, text=True)
    if run.returncode != 0:
        sys.exit(f"tend {' '.join(args)} failed: {run.stderr.strip()[:300]}")
    return json.loads(run.stdout)


def mailbox_path():
    """The store of the daemon `tend` talks to: $XDG_DATA_HOME/tend, else the
    platform default, as crates/tend/src/paths.rs resolves it."""
    if len(sys.argv) > 1:
        return sys.argv[1]
    xdg = os.environ.get("XDG_DATA_HOME")
    if xdg:
        return os.path.join(xdg, "tend", "collaboration.sqlite3")
    if sys.platform == "darwin":
        return os.path.expanduser("~/Library/Application Support/tend/collaboration.sqlite3")
    return os.path.expanduser("~/.local/share/tend/collaboration.sqlite3")


queue = tend_json("review", "queue", "--json", "--limit", "200")
# Exact live sessions: an agent row whose state is not stopped.
live = {a["agent_session_id"] for a in tend_json("agents", "--json") if a.get("state") != "stopped"}
path = mailbox_path()
if not os.path.exists(path):
    sys.exit(f"no mailbox store at {path}; set XDG_DATA_HOME or pass the store's path")
db = sqlite3.connect(f"file:{path}?mode=ro", uri=True)


def row(request_id):
    found = db.execute("select payload from collaboration_requests where id=?", (request_id,)).fetchone()
    return json.loads(found[0]) if found else None


now = datetime.datetime.now(datetime.timezone.utc)


def age(at):
    if not at:
        return None
    then = datetime.datetime.fromisoformat(at.replace("Z", "+00:00"))
    return round((now - then).total_seconds() / 86400, 1)


for it in queue["items"]:
    kind = it.get("kind")
    if kind != "human_request":
        print(json.dumps({"qkind": kind, "since_age": age(it.get("since") or it.get("requested_at"))}))
        continue
    ask = row(it["request_id"])
    if not ask:
        print(json.dumps({"id": it["request_id"], "err": "no row"}))
        continue
    out = {"id": ask["id"], "sys": bool(it.get("system")), "from": ask["from"]["agent_session_id"],
           "age": age(ask["created_at"]), "managed": ask.get("system_ask") is not None,
           "kind": ask.get("kind"), "action": ask.get("human_action")}
    if not it.get("system"):
        out["from_live"] = ask["from"]["agent_session_id"] in live
        out["from_pane"] = ask["from"]["pane"]
    parent_id = ask.get("parent_request_id")
    parent = row(parent_id) if parent_id else None
    if parent_id and not parent:
        out["parent"] = "MISSING"
    if parent:
        intr = parent.get("interruption") or {}
        follow = parent.get("sender_follow") or {}
        out.update({
            "p_status": parent["status"], "p_from": parent["from"]["agent_session_id"],
            "p_from_live": parent["from"]["agent_session_id"] in live,
            "p_from_console": bool(parent["from"].get("console")), "p_init": parent.get("initiator") is not None,
            "p_to": parent["to"]["agent_session_id"], "p_to_live": parent["to"]["agent_session_id"] in live,
            "i_reason": intr.get("reason"), "i_active": intr.get("active"), "i_obs_age": age(intr.get("observed_at")),
            "i_ptr": intr.get("action_request_id"), "reply_age": age((parent.get("reply") or {}).get("at")),
            "sf_ptr": follow.get("gone_action_request_id")})
    print(json.dumps(out))

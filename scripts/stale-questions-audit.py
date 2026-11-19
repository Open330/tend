#!/usr/bin/env python3
"""Read-only audit of what waits on the person, for docs/proposals/STALE_QUESTIONS.md.

Prints one JSON line per queue item with the fields the cleanup rules read:
ages in days, whether the item is tend's own, its original request's state,
whether the sessions involved have a live row, and the interruption or
reply-wake pointers. Changes nothing: the queue comes from
`tend review queue --json`, rows from the mailbox store opened with
`mode=ro`, live sessions from `tend status --json` (a session counts as live
when its id appears there, which is approximate).

    python3 scripts/stale-questions-audit.py
    ssh host 'python3 -' < scripts/stale-questions-audit.py
"""
import json,subprocess,sqlite3,os,glob,datetime
q=json.loads(subprocess.run(["tend","review","queue","--json","--limit","200"],capture_output=True,text=True).stdout)
st=subprocess.run(["tend","status","--json"],capture_output=True,text=True).stdout
path=[p for p in glob.glob(os.path.expanduser("~/Library/Application Support/tend/collaboration.sqlite3"))+glob.glob(os.path.expanduser("~/.local/share/tend/collaboration.sqlite3")) if os.path.exists(p)][0]
db=sqlite3.connect(f"file:{path}?mode=ro",uri=True)
def row(i):
    r=db.execute("select payload from collaboration_requests where id=?",(i,)).fetchone()
    return json.loads(r[0]) if r else None
now=datetime.datetime.now(datetime.timezone.utc)
def age(t): return None if not t else round((now-datetime.datetime.fromisoformat(t.replace("Z","+00:00"))).total_seconds()/86400,1)
for it in q["items"]:
    k=it.get("kind")
    if k!="human_request":
        print(json.dumps({"qkind":k,"since_age":age(it.get("since") or it.get("requested_at"))})); continue
    a=row(it["request_id"])
    if not a: print(json.dumps({"id":it["request_id"],"err":"no row"})); continue
    o={"id":a["id"],"sys":bool(it.get("system")),"from":a["from"]["agent_session_id"],"age":age(a["created_at"]),"managed":a.get("system_ask") is not None,"prov":a.get("provenance") is not None,"kind":a.get("kind"),"action":a.get("human_action")}
    if not it.get("system"):
        o["from_live"]=a["from"]["agent_session_id"] in st
        o["from_pane"]=a["from"]["pane"]
    pid=a.get("parent_request_id"); p=row(pid) if pid else None
    if pid and not p: o["parent"]="MISSING"
    if p:
        intr=p.get("interruption") or {}; sf=p.get("sender_follow") or {}
        o.update({"p_status":p["status"],"p_from":p["from"]["agent_session_id"],"p_from_live":p["from"]["agent_session_id"] in st,"p_from_console":bool(p["from"].get("console")),"p_init":p.get("initiator") is not None,"p_to":p["to"]["agent_session_id"],"p_to_live":p["to"]["agent_session_id"] in st,
          "i_reason":intr.get("reason"),"i_active":intr.get("active"),"i_obs_age":age(intr.get("observed_at")),"i_ptr":intr.get("action_request_id"),"reply_age":age((p.get("reply") or {}).get("at")),"sf_ptr":sf.get("gone_action_request_id"),"reply_wake":p.get("reply_wake")})
    print(json.dumps(o))

//! `tend_work_queue`: queue a prompt for an agent once it is free, or a new
//! Work for when the queue's throttle allows. tendd places the caller (an
//! agent manages only its own items and may target workers or itself).

use serde_json::{json, Value};
use tend::ipc::Client;
use tend::work_queue::{QueueAddRequest, QueueTargetSpec};

pub(crate) fn definition() -> Value {
    json!({
        "name": "tend_work_queue",
        "description": "Queue work instead of starting it now. action=add: prompt plus a target (pane or session_id: sent as that agent's next turn once it is idle; work: one agent of that Work; new_work=true with cwd: started as a new Work when few agents are busy). tendd sends each item at most once and never to a busy, waiting or rate-limited agent; an unknown outcome shows as uncertain. list, cancel(id), run_now(id). CLI: `tend work queue`.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "action": {"type": "string", "enum": ["add", "list", "cancel", "run_now"]},
                "prompt": {"type": "string"},
                "pane": {"type": "string"},
                "session_id": {"type": "string"},
                "work": {"type": "string", "description": "WORKSPACE/WORK[@ALIAS]; with new_work, a name for the new Work."},
                "new_work": {"type": "boolean"},
                "cwd": {"type": "string", "description": "new_work: absolute directory."},
                "pipeline": {"type": "string"},
                "title": {"type": "string"},
                "request_key": {"type": "string", "description": "Reuse on retry so the item is added once."},
                "id": {"type": "string"},
                "include_finished": {"type": "boolean"}
            },
            "required": ["action"],
            "additionalProperties": false
        }
    })
}

fn text(args: &Value, key: &str) -> Option<String> {
    args.get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

/// The add target named by `args`, or why it cannot be built.
pub(crate) fn target(args: &Value) -> Result<QueueTargetSpec, String> {
    if args
        .get("new_work")
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        let cwd = text(args, "cwd").ok_or("new_work needs cwd, an absolute directory")?;
        return Ok(QueueTargetSpec::NewWork {
            work: text(args, "work"),
            workspace: None,
            cwd: Some(cwd.into()),
            pipeline: text(args, "pipeline"),
            external: None,
            skill: None,
            context: None,
        });
    }
    if let Some(work) = text(args, "work") {
        let (path, alias) = match work.split_once('@') {
            Some((path, alias)) => (path.to_string(), Some(alias.to_string())),
            None => (work.clone(), None),
        };
        let (workspace, work) = path
            .split_once('/')
            .filter(|(w, id)| !w.is_empty() && !id.is_empty())
            .ok_or("work takes WORKSPACE/WORK or WORKSPACE/WORK@ALIAS")?;
        return Ok(QueueTargetSpec::Work {
            workspace: workspace.into(),
            work: work.into(),
            alias,
        });
    }
    let pane = text(args, "pane");
    let session_id = text(args, "session_id");
    if pane.is_none() && session_id.is_none() {
        return Err("add needs a target: pane, session_id, work, or new_work with cwd".into());
    }
    Ok(QueueTargetSpec::Agent {
        session_id,
        pane,
        socket: None,
        label: None,
    })
}

pub(crate) async fn call(client: &Client, args: &Value) -> Result<Value, String> {
    let action = text(args, "action").unwrap_or_default();
    let id = || text(args, "id").ok_or_else(|| format!("{action} needs id"));
    let item = |result: Result<tend::work_queue::QueueItem, tend::ipc::RuntimeError>| {
        result
            .map(|item| json!({"item": item}))
            .map_err(|error| error.to_string())
    };
    match action.as_str() {
        "list" => client
            .work_queue_list(
                args.get("include_finished")
                    .and_then(Value::as_bool)
                    .unwrap_or(false),
            )
            .await
            .map(|snapshot| json!(snapshot))
            .map_err(|error| error.to_string()),
        "add" => {
            let prompt = text(args, "prompt").ok_or("add needs prompt")?;
            let request_key =
                text(args, "request_key").unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
            let request = QueueAddRequest {
                request_key: request_key.clone(),
                title: text(args, "title"),
                prompt,
                target: target(args)?,
            };
            client
                .work_queue_add(&request)
                .await
                .map(|item| json!({"item": item, "request_key": request_key}))
                .map_err(|error| error.to_string())
        }
        "cancel" => item(client.work_queue_cancel(&id()?, None).await),
        "run_now" => item(client.work_queue_run_now(&id()?, None, false).await),
        other => Err(format!(
            "unknown action {other:?}; use add, list, cancel or run_now"
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn targets_come_from_the_named_fields() {
        assert!(matches!(
            target(&json!({"pane": "%3"})),
            Ok(QueueTargetSpec::Agent { pane: Some(p), session_id: None, .. }) if p == "%3"
        ));
        assert!(matches!(
            target(&json!({"work": "web/ENG-1@impl"})),
            Ok(QueueTargetSpec::Work { alias: Some(a), .. }) if a == "impl"
        ));
        assert!(matches!(
            target(&json!({"new_work": true, "cwd": "/repo", "work": "login"})),
            Ok(QueueTargetSpec::NewWork { work: Some(w), .. }) if w == "login"
        ));
        assert!(target(&json!({"new_work": true})).is_err());
        assert!(target(&json!({})).is_err());
        assert!(target(&json!({"work": "ENG-1"})).is_err());
    }
}

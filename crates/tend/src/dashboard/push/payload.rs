//! What a notification carries. By default nothing anyone wrote: the host
//! alias, counts and an opaque `ref`. The item's text only when the host
//! and the phone both allow it, at send time.
//!
//! The `ref` and the canonical line are shared with the iOS app through
//! `apps/shared/Fixtures/push-vectors.json`.
use hmac::{Hmac, Mac};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

/// The longest canonical line, in characters, the ellipsis included.
pub(super) const MAX_LINE_CHARS: usize = 120;

/// The kind of a queue entry, as the `ref` names it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub(super) enum Kind {
    /// A human request; its id is the request id.
    Request,
    /// A review item; its id is the review id.
    Review,
    /// A waiting agent; its id is its session id.
    Waiting,
}

impl Kind {
    pub(super) fn as_str(self) -> &'static str {
        match self {
            Self::Request => "request",
            Self::Review => "review",
            Self::Waiting => "waiting",
        }
    }

    /// The notification category that offers this kind's buttons (only
    /// ever sent with content).
    pub(super) fn category(self) -> &'static str {
        match self {
            Self::Request => "TEND_QUESTION",
            Self::Review => "TEND_REVIEW",
            Self::Waiting => "TEND_WAITING",
        }
    }
}

pub(super) fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write;
    bytes.iter().fold(String::new(), |mut out, byte| {
        let _ = write!(out, "{byte:02x}");
        out
    })
}

/// The first 16 bytes of HMAC-SHA256(`key`, host NUL kind NUL id), as hex.
pub(super) fn item_ref(key: &[u8], host: &str, kind: Kind, id: &str) -> String {
    let mut mac = Hmac::<Sha256>::new_from_slice(key).expect("HMAC accepts any key length");
    mac.update(host.as_bytes());
    mac.update(&[0]);
    mac.update(kind.as_str().as_bytes());
    mac.update(&[0]);
    mac.update(id.as_bytes());
    hex(&mac.finalize().into_bytes()[..16])
}

/// `a:` and the first 16 bytes of SHA-256(alias) as hex: 34 bytes for any
/// alias, under Apple's 64.
pub(super) fn collapse_id(host: &str) -> String {
    format!("a:{}", hex(&Sha256::digest(host.as_bytes())[..16]))
}

/// One line with its Markdown marks removed, or empty.
fn clean_line(line: &str) -> String {
    let mut rest = line.trim_start();
    // Repeated heading and quote marks, each with the spaces after it.
    while let Some(stripped) = rest.strip_prefix(['#', '>']) {
        rest = stripped.trim_start_matches(' ');
    }
    // One list marker.
    for marker in ["- ", "* ", "+ "] {
        if let Some(stripped) = rest.strip_prefix(marker) {
            rest = stripped;
            break;
        }
    }
    let kept: String = rest
        .chars()
        .filter(|c| !matches!(c, '*' | '_' | '`' | '~'))
        .collect();
    kept.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// The canonical line of an item's text: the first line that is not empty
/// once its Markdown marks are removed and its whitespace collapsed, at
/// most [`MAX_LINE_CHARS`] characters (119 and `…` when longer).
pub(super) fn canonical_line(text: &str) -> String {
    let line = text
        .lines()
        .map(clean_line)
        .find(|line| !line.is_empty())
        .unwrap_or_default();
    if line.chars().count() > MAX_LINE_CHARS {
        let mut cut: String = line.chars().take(MAX_LINE_CHARS - 1).collect();
        cut.push('…');
        cut
    } else {
        line
    }
}

/// The item text an alert with content shows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Content {
    pub kind: Kind,
    /// The canonical line.
    pub body: String,
    /// The asking agent's label.
    pub subtitle: String,
}

/// An alert for one host.
pub(super) fn alert(
    host: &str,
    count: usize,
    badge: usize,
    channel: &str,
    item: &str,
    content: Option<&Content>,
) -> Value {
    let title = format!("Tend: {host} needs you");
    let mut alert = json!({
        "title-loc-key": "PUSH_TITLE", "title-loc-args": [host], "title": title,
    });
    let category = if let Some(content) = content {
        alert["subtitle"] = json!(content.subtitle);
        alert["body"] = json!(content.body);
        content.kind.category()
    } else {
        alert["loc-key"] = json!("PUSH_BODY");
        alert["loc-args"] = json!([count.to_string()]);
        alert["body"] = json!(format!("{count} new"));
        "TEND_OPEN"
    };
    json!({
        "aps": {
            "alert": alert, "badge": badge, "sound": "default", "thread-id": host,
            "category": category,
        },
        "tend": { "v": 1, "channel": channel, "host": host, "ref": item },
    })
}

/// A badge-only update: no alert, no sound.
pub(super) fn badge(badge: usize) -> Value {
    json!({ "aps": { "badge": badge } })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vectors() -> Value {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../apps/shared/Fixtures/push-vectors.json"
        );
        serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
    }

    fn unhex(text: &str) -> Vec<u8> {
        (0..text.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&text[i..i + 2], 16).unwrap())
            .collect()
    }

    #[test]
    fn refs_match_the_shared_vectors() {
        let vectors = vectors();
        let cases = vectors["ref"].as_array().unwrap();
        assert!(cases.len() >= 3);
        for case in cases {
            let kind = match case["kind"].as_str().unwrap() {
                "request" => Kind::Request,
                "review" => Kind::Review,
                "waiting" => Kind::Waiting,
                other => panic!("unknown kind {other}"),
            };
            let got = item_ref(
                &unhex(case["key_hex"].as_str().unwrap()),
                case["host"].as_str().unwrap(),
                kind,
                case["id"].as_str().unwrap(),
            );
            assert_eq!(got, case["ref"].as_str().unwrap(), "{case}");
        }
    }

    #[test]
    fn canonical_lines_match_the_shared_vectors() {
        let vectors = vectors();
        let cases = vectors["canonical"].as_array().unwrap();
        assert!(cases.len() >= 8);
        for case in cases {
            let got = canonical_line(case["text"].as_str().unwrap());
            assert_eq!(got, case["line"].as_str().unwrap(), "{case}");
            assert!(got.chars().count() <= MAX_LINE_CHARS);
        }
    }

    #[test]
    fn the_ref_binds_host_kind_and_id_apart() {
        let key = [7u8; 32];
        let a = item_ref(&key, "mini", Kind::Request, "r1");
        assert_eq!(a.len(), 32);
        assert_ne!(a, item_ref(&key, "mini", Kind::Review, "r1"));
        assert_ne!(a, item_ref(&key, "mini2", Kind::Request, "r1"));
        // The NUL separators keep "a" + "b\0c" apart from "a\0b" + "c".
        assert_ne!(
            item_ref(&key, "ab", Kind::Request, "x"),
            item_ref(&key, "a", Kind::Request, "bx")
        );
        assert_ne!(a, item_ref(&[8u8; 32], "mini", Kind::Request, "r1"));
    }

    #[test]
    fn a_long_alias_gets_a_collapse_id_within_64_bytes() {
        let alias = "h".repeat(63);
        let id = collapse_id(&alias);
        assert_eq!(id.len(), 34);
        assert!(id.len() <= 64);
        assert!(id.starts_with("a:"));
        assert_ne!(id, collapse_id("mini"));
    }

    #[test]
    fn a_default_alert_names_only_the_host_and_counts() {
        let payload = alert("mini", 2, 3, "ch_x", "9f2c", None);
        assert_eq!(
            payload,
            json!({"aps": {"alert": {"title-loc-key": "PUSH_TITLE", "title-loc-args": ["mini"],
                       "loc-key": "PUSH_BODY", "loc-args": ["2"],
                       "title": "Tend: mini needs you", "body": "2 new"},
                 "badge": 3, "sound": "default", "thread-id": "mini",
                 "category": "TEND_OPEN"},
             "tend": {"v": 1, "channel": "ch_x", "host": "mini", "ref": "9f2c"}})
        );
        let content = Content {
            kind: Kind::Review,
            body: "check the migration".into(),
            subtitle: "claude".into(),
        };
        let payload = alert("mini", 1, 1, "ch_x", "9f2c", Some(&content));
        assert_eq!(payload["aps"]["category"], "TEND_REVIEW");
        assert_eq!(payload["aps"]["alert"]["body"], "check the migration");
        assert_eq!(payload["aps"]["alert"]["subtitle"], "claude");
        assert!(payload["aps"]["alert"].get("loc-key").is_none());
        assert_eq!(badge(0), json!({"aps": {"badge": 0}}));
    }
}

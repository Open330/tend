//! The APNs HTTP/2 client and its provider token.
//!
//! The endpoints are fixed; tests construct the sender with their own in
//! code ([`Apns::with_endpoint`]), never through config. No redirect is
//! followed. An error or log line never holds the request path (it carries
//! the device token) or the `authorization` header: reqwest's errors are
//! reduced to their kind here.
use base64::Engine as _;
use p256::ecdsa::{signature::Signer, Signature, SigningKey};
use serde_json::{json, Value};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use super::Environment;

const PRODUCTION: &str = "https://api.push.apple.com";
const SANDBOX: &str = "https://api.sandbox.push.apple.com";
const TIMEOUT: Duration = Duration::from_secs(10);
/// A provider token is renewed after this long (Apple refuses one older
/// than 60 minutes, and renewals more often than every 20).
pub(super) const TOKEN_RENEWAL_SECS: i64 = 40 * 60;
/// An alert Apple stores for an offline phone expires after this long.
pub(super) const ALERT_EXPIRATION_SECS: i64 = 6 * 3600;

/// The unix clock, injected so tests can move it.
pub(super) type Clock = Arc<dyn Fn() -> i64 + Send + Sync>;

/// One request to Apple.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Request {
    pub environment: Environment,
    /// The device token, lower-case hex: only ever put in the path.
    pub token: String,
    pub payload: Value,
    /// An alert (priority 10, expires in 6 h, with a collapse id) or a
    /// badge-only update (priority 5, expiration 0, no collapse id).
    pub collapse_id: Option<String>,
    pub alert: bool,
}

/// What Apple answered.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Outcome {
    Accepted,
    /// `410 Unregistered`, with Apple's timestamp in ms when it gave one.
    Unregistered(Option<i64>),
    /// `400 BadDeviceToken` or `DeviceTokenNotForTopic`.
    BadToken(&'static str),
    /// `TooManyProviderTokenUpdates`: back off the sender.
    TooManyTokenUpdates,
    /// `InvalidProviderToken`, `BadEnvironmentKeyIdInToken`,
    /// `MissingProviderToken`: turn this environment off.
    ProviderToken(String),
    /// `429`, `5xx`, a timeout, a connection failure, or
    /// `ExpiredProviderToken` again after the retry: back off the device.
    Retry(String),
    /// Anything else Apple refused (the reason only).
    Refused(String),
}

/// The ES256 provider token, cached for [`TOKEN_RENEWAL_SECS`].
pub(super) struct ProviderToken {
    key: SigningKey,
    key_id: String,
    team_id: String,
    cached: Option<(String, i64)>,
}

fn b64(bytes: &[u8]) -> String {
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}

impl ProviderToken {
    pub(super) fn new(key: SigningKey, key_id: String, team_id: String) -> Self {
        Self {
            key,
            key_id,
            team_id,
            cached: None,
        }
    }

    /// The token at `now`: the cached one, or a new one when it is
    /// [`TOKEN_RENEWAL_SECS`] old (or `renew` asks).
    pub(super) fn get(&mut self, now: i64, renew: bool) -> String {
        if let Some((token, iat)) = &self.cached {
            if !renew && now.saturating_sub(*iat) < TOKEN_RENEWAL_SECS {
                return token.clone();
            }
        }
        let header = json!({ "alg": "ES256", "kid": self.key_id });
        let claims = json!({ "iss": self.team_id, "iat": now });
        let signing_input = format!(
            "{}.{}",
            b64(header.to_string().as_bytes()),
            b64(claims.to_string().as_bytes())
        );
        let signature: Signature = self.key.sign(signing_input.as_bytes());
        let token = format!("{signing_input}.{}", b64(&signature.to_bytes()));
        self.cached = Some((token.clone(), now));
        token
    }
}

/// The sender to Apple.
pub(super) struct Apns {
    client: reqwest::Client,
    production: String,
    sandbox: String,
    topic: String,
    token: Mutex<ProviderToken>,
    clock: Clock,
}

fn clock() -> Clock {
    Arc::new(|| time::OffsetDateTime::now_utc().unix_timestamp())
}

impl Apns {
    /// The sender to Apple's own endpoints.
    pub(super) fn new(topic: String, token: ProviderToken) -> Result<Self, String> {
        let client = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(TIMEOUT)
            .https_only(true)
            .build()
            .map_err(|_| "the HTTP client could not be built".to_owned())?;
        Ok(Self {
            client,
            production: PRODUCTION.into(),
            sandbox: SANDBOX.into(),
            topic,
            token: Mutex::new(token),
            clock: clock(),
        })
    }

    /// A sender to a test endpoint, over HTTP/2 cleartext with prior
    /// knowledge, for both environments, on the given clock.
    #[cfg(test)]
    pub(super) fn with_endpoint(
        endpoint: &str,
        topic: String,
        token: ProviderToken,
        clock: Clock,
    ) -> Self {
        let client = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(TIMEOUT)
            .http2_prior_knowledge()
            .build()
            .expect("client");
        Self {
            client,
            production: format!("{endpoint}/production"),
            sandbox: format!("{endpoint}/development"),
            topic,
            token: Mutex::new(token),
            clock,
        }
    }

    fn provider_token(&self, renew: bool) -> String {
        let now = (self.clock)();
        self.token
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(now, renew)
    }

    /// Send one request. `ExpiredProviderToken` re-signs and retries once.
    pub(super) async fn send(&self, request: &Request) -> Outcome {
        let first = self.attempt(request, false).await;
        match first {
            Attempt::Expired => match self.attempt(request, true).await {
                Attempt::Expired => Outcome::Retry("403 ExpiredProviderToken".into()),
                Attempt::Done(outcome) => outcome,
            },
            Attempt::Done(outcome) => outcome,
        }
    }

    async fn attempt(&self, request: &Request, renew: bool) -> Attempt {
        let base = match request.environment {
            Environment::Production => &self.production,
            Environment::Development => &self.sandbox,
        };
        let now = (self.clock)();
        let mut builder = self
            .client
            .post(format!("{base}/3/device/{}", request.token))
            .header(
                "authorization",
                format!("bearer {}", self.provider_token(renew)),
            )
            .header("apns-topic", &self.topic)
            .header("apns-push-type", "alert")
            .header("apns-priority", if request.alert { "10" } else { "5" })
            .header(
                "apns-expiration",
                if request.alert {
                    (now + ALERT_EXPIRATION_SECS).to_string()
                } else {
                    "0".to_owned()
                },
            )
            .header("content-type", "application/json")
            .body(request.payload.to_string());
        if let Some(collapse) = &request.collapse_id {
            builder = builder.header("apns-collapse-id", collapse);
        }
        let response = match builder.send().await {
            Ok(response) => response,
            // The kind only: reqwest's own text names the URL.
            Err(error) => {
                return Attempt::Done(Outcome::Retry(
                    if error.is_timeout() {
                        "timeout"
                    } else if error.is_connect() {
                        "connection failed"
                    } else {
                        "request failed"
                    }
                    .into(),
                ))
            }
        };
        let status = response.status().as_u16();
        let body: Value = response
            .bytes()
            .await
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or(Value::Null);
        // Apple's reason is one word from its fixed list; anything longer
        // or odd is not echoed.
        let reason = body["reason"]
            .as_str()
            .filter(|r| r.len() <= 64 && r.chars().all(|c| c.is_ascii_alphanumeric()))
            .unwrap_or("")
            .to_owned();
        Attempt::Done(match (status, reason.as_str()) {
            (200, _) => Outcome::Accepted,
            (403, "ExpiredProviderToken") => return Attempt::Expired,
            (410, _) => Outcome::Unregistered(body["timestamp"].as_i64()),
            (400, "BadDeviceToken") => Outcome::BadToken("BadDeviceToken"),
            (400 | 403, "DeviceTokenNotForTopic") => Outcome::BadToken("DeviceTokenNotForTopic"),
            (_, "TooManyProviderTokenUpdates") => Outcome::TooManyTokenUpdates,
            (_, "InvalidProviderToken" | "BadEnvironmentKeyIdInToken" | "MissingProviderToken") => {
                Outcome::ProviderToken(reason)
            }
            (429 | 500..=599, _) => Outcome::Retry(format!("{status} {reason}")),
            _ => Outcome::Refused(format!("{status} {reason}")),
        })
    }
}

enum Attempt {
    Expired,
    Done(Outcome),
}

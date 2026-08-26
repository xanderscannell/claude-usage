//! Reads the true rate-limit state from Claude Code's own usage endpoint.
//!
//! Claude Code stores an OAuth token in `~/.claude/.credentials.json` and
//! calls `GET /api/oauth/usage` to render `/usage`. That response is the
//! only authoritative source for what percentage of a limit is actually
//! spent, so when it is reachable the panel uses it and treats the local
//! session logs purely as the shape of the curve.
//!
//! Deliberate constraints:
//!
//! - **Read only.** The credentials file is never written. Refresh tokens
//!   normally rotate, so refreshing here and failing to persist the result
//!   could invalidate the CLI's own login. An expired token is reported,
//!   not repaired - running any Claude Code command refreshes it.
//! - **The token goes to exactly one host** and is never logged, returned
//!   through a command, or included in an error message.
//! - **Every failure is soft.** This endpoint is undocumented and may
//!   change or vanish; the panel must keep working from local logs alone.

use serde::Serialize;
use std::path::PathBuf;
use std::time::Duration;

const ENDPOINT: &str = "https://api.anthropic.com/api/oauth/usage";
const BETA: &str = "oauth-2025-04-20";
const TIMEOUT: Duration = Duration::from_secs(8);

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Window {
    /// Percent, 0-100. The API reports a 0-1 fraction; converted here once.
    pub pct: f64,
    /// Unix seconds.
    pub resets_at: i64,
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Usage {
    pub five_hour: Option<Window>,
    pub seven_day: Option<Window>,
    pub seven_day_opus: Option<Window>,
}

impl Usage {
    pub fn is_empty(&self) -> bool {
        self.five_hour.is_none() && self.seven_day.is_none()
    }
}

#[derive(Debug)]
pub enum Error {
    NoCredentials,
    Expired,
    Unauthorized,
    Offline(String),
    Unexpected(String),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoCredentials => write!(f, "Not signed in to Claude Code on this machine."),
            Self::Expired => write!(f, "Claude Code's token has expired. Run any Claude Code command to refresh it."),
            Self::Unauthorized => write!(f, "Claude Code's token was rejected. Run any Claude Code command to refresh it."),
            Self::Offline(why) => write!(f, "Could not reach the usage endpoint: {why}"),
            Self::Unexpected(why) => write!(f, "Unexpected response from the usage endpoint: {why}"),
        }
    }
}

fn credentials_path() -> Option<PathBuf> {
    let p = dirs::home_dir()?.join(".claude").join(".credentials.json");
    p.is_file().then_some(p)
}

/// Depth-first search for any object carrying an `accessToken`, so the
/// exact wrapper key does not matter across Claude Code versions.
fn find_token(v: &serde_json::Value) -> Option<(String, Option<i64>)> {
    match v {
        serde_json::Value::Object(map) => {
            if let Some(tok) = map.get("accessToken").and_then(|t| t.as_str()) {
                if !tok.is_empty() {
                    return Some((tok.to_string(), map.get("expiresAt").and_then(|e| e.as_i64())));
                }
            }
            map.values().find_map(find_token)
        }
        serde_json::Value::Array(items) => items.iter().find_map(find_token),
        _ => None,
    }
}

fn now_secs() -> i64 {
    chrono::Utc::now().timestamp()
}

/// Reads the access token. The returned string is a secret: it must only
/// ever be handed to `fetch`, never logged or surfaced.
fn read_token() -> Result<String, Error> {
    let path = credentials_path().ok_or(Error::NoCredentials)?;
    let raw = std::fs::read_to_string(path).map_err(|_| Error::NoCredentials)?;
    let json: serde_json::Value = serde_json::from_str(&raw).map_err(|_| Error::NoCredentials)?;
    let (token, expires_at) = find_token(&json).ok_or(Error::NoCredentials)?;

    // `expiresAt` has been stored in both seconds and milliseconds.
    if let Some(exp) = expires_at {
        let exp_secs = if exp > 1_000_000_000_000 { exp / 1000 } else { exp };
        if exp_secs <= now_secs() {
            return Err(Error::Expired);
        }
    }
    Ok(token)
}

/// `resets_at` comes back as an RFC3339 timestamp; accept a raw unix
/// value too in case that ever changes.
fn instant(v: &serde_json::Value) -> Option<i64> {
    if let Some(n) = v.as_f64() {
        return Some(if n > 1e12 { (n / 1000.0) as i64 } else { n as i64 });
    }
    chrono::DateTime::parse_from_rfc3339(v.as_str()?).ok().map(|d| d.timestamp())
}

fn window(v: Option<&serde_json::Value>) -> Option<Window> {
    let obj = v?;
    // This endpoint reports whole percent (25.0 means 25%), unlike the
    // `anthropic-ratelimit-*` response headers, which use a 0-1 fraction.
    let pct = obj.get("utilization")?.as_f64()?;
    Some(Window { pct, resets_at: instant(obj.get("resets_at")?)? })
}

fn parse(body: &str) -> Result<Usage, Error> {
    let v: serde_json::Value =
        serde_json::from_str(body).map_err(|e| Error::Unexpected(e.to_string()))?;
    let usage = Usage {
        five_hour: window(v.get("five_hour")),
        seven_day: window(v.get("seven_day")),
        seven_day_opus: window(v.get("seven_day_opus")),
    };
    if usage.is_empty() {
        // Report the field names only; the body may carry account details.
        let keys: Vec<&str> = v.as_object().map(|o| o.keys().map(|k| k.as_str()).collect()).unwrap_or_default();
        return Err(Error::Unexpected(format!("no rate limit windows in {keys:?}")));
    }
    Ok(usage)
}

/// Fetches live utilization. Blocking; call from a blocking context.
pub fn fetch() -> Result<Usage, Error> {
    parse(&fetch_raw()?)
}

/// Performs the request and returns the response body. The body can
/// carry account details, so callers must not log it wholesale.
fn fetch_raw() -> Result<String, Error> {
    let token = read_token()?;

    let response = ureq::get(ENDPOINT)
        .timeout(TIMEOUT)
        .set("Authorization", &format!("Bearer {token}"))
        .set("Content-Type", "application/json")
        .set("anthropic-beta", BETA)
        .set("User-Agent", concat!("claude-usage-monitor/", env!("CARGO_PKG_VERSION")))
        .call();

    match response {
        Ok(r) => r.into_string().map_err(|e| Error::Unexpected(e.to_string())),
        Err(ureq::Error::Status(401 | 403, _)) => Err(Error::Unauthorized),
        Err(ureq::Error::Status(code, _)) => Err(Error::Unexpected(format!("HTTP {code}"))),
        Err(ureq::Error::Transport(t)) => Err(Error::Offline(t.to_string())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_a_token_regardless_of_wrapper_key() {
        for raw in [
            r#"{"claudeAiOauth":{"accessToken":"abc","expiresAt":99999999999999}}"#,
            r#"{"oauth":{"accessToken":"abc","expiresAt":99999999999999}}"#,
            r#"{"accessToken":"abc"}"#,
            r#"{"a":{"b":[{"accessToken":"abc"}]}}"#,
        ] {
            let v: serde_json::Value = serde_json::from_str(raw).unwrap();
            assert_eq!(find_token(&v).map(|t| t.0), Some("abc".into()), "failed on {raw}");
        }
        let v: serde_json::Value = serde_json::from_str(r#"{"accessToken":""}"#).unwrap();
        assert!(find_token(&v).is_none(), "an empty token is not a token");
    }

    /// Verbatim shape returned by the live endpoint.
    #[test]
    fn parses_the_real_response() {
        let body = r#"{
            "five_hour":{"limit_dollars":null,"remaining_dollars":null,
                "resets_at":"2026-08-26T08:00:00.120864+00:00",
                "used_dollars":null,"utilization":25.0},
            "seven_day":{"limit_dollars":null,"remaining_dollars":null,
                "resets_at":"2026-08-29T08:00:00.120883+00:00",
                "used_dollars":null,"utilization":33.0},
            "limits":[{"group":"session","kind":"session","percent":25,
                "resets_at":"2026-08-26T08:00:00.120864+00:00","is_active":false}]
        }"#;
        let u = parse(body).unwrap();
        let fh = u.five_hour.unwrap();
        assert!((fh.pct - 25.0).abs() < 1e-9, "whole percent, not a fraction: {}", fh.pct);
        // 2026-08-26T08:00:00Z
        assert_eq!(fh.resets_at, 1787731200);
        assert!((u.seven_day.unwrap().pct - 33.0).abs() < 1e-9);
        assert!(u.seven_day_opus.is_none(), "absent windows stay absent");
    }

    #[test]
    fn tolerates_numeric_timestamps_and_rejects_junk() {
        let u = parse(r#"{"five_hour":{"utilization":42.0,"resets_at":1787731200}}"#).unwrap();
        let fh = u.five_hour.unwrap();
        assert!((fh.pct - 42.0).abs() < 1e-9);
        assert_eq!(fh.resets_at, 1787731200);
        assert!(parse(r#"{"something_else":1}"#).is_err());
        assert!(parse(r#"{"five_hour":{"utilization":1}}"#).is_err(), "no reset means no window");
    }
}

#[cfg(test)]
mod live_tests {
    /// Hits the real endpoint with the real token. Ignored by default so
    /// the suite stays offline: `cargo test -- --ignored --nocapture`.
    /// Prints utilization only - the token is never surfaced.
    #[test]
    #[ignore]
    fn live_fetch() {
        match super::fetch() {
            Ok(u) => {
                for (name, w) in [
                    ("five_hour", u.five_hour),
                    ("seven_day", u.seven_day),
                    ("seven_day_opus", u.seven_day_opus),
                ] {
                    match w {
                        Some(w) => eprintln!(
                            "{name}: {:.1}%  resets_at={} (in {:.2}h)",
                            w.pct,
                            w.resets_at,
                            (w.resets_at - chrono::Utc::now().timestamp()) as f64 / 3600.0
                        ),
                        None => eprintln!("{name}: absent"),
                    }
                }
            }
            Err(e) => eprintln!("LIVE FETCH FAILED: {e}"),
        }
    }

    /// Prints only the rate-limit subtrees, so unrelated account fields
    /// stay out of the log.
    #[test]
    #[ignore]
    fn live_shape() {
        let body = super::fetch_raw().expect("fetch");
        let v: serde_json::Value = serde_json::from_str(&body).expect("json");
        for k in ["five_hour", "seven_day", "seven_day_opus", "seven_day_sonnet", "limits"] {
            match v.get(k) {
                Some(sub) => eprintln!("{k} = {}", serde_json::to_string(sub).unwrap()),
                None => eprintln!("{k} = <absent>"),
            }
        }
    }
}

// Official Codex plan limits from the endpoint Codex's own /status polls:
// GET https://chatgpt.com/backend-api/wham/usage. The rollouts can't serve as
// the source any more: Codex keeps one "latest" rate-limit slot that every
// quota bucket overwrites, so the persisted token_count snapshots are often
// another bucket or null windows, and they only change while Codex makes
// requests (a window rolling over or a redeemed reset credit never reaches
// disk). codex.rs's rollout reading remains the fallback.
//
// This integration is intentionally READ-ONLY, like claude_limits.rs: it reads
// the ChatGPT access token Codex stores and sends it only to OpenAI. It never
// refreshes it, because Codex's refresh tokens are single-use — using one here
// would sign Codex out. When the token has expired the last good snapshot is
// shown as stale until Codex refreshes its own login.

use crate::claude_limits::LimitWindow;
use base64::Engine as _;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::Path;
use std::time::Duration;

const USAGE_URL: &str = "https://chatgpt.com/backend-api/wham/usage";
// Codex's credential-store item when cli_auth_credentials_store = keyring/auto.
const KEYCHAIN_SERVICE: &str = "Codex Auth";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CodexLiveQuota {
    pub five_hour: Option<LimitWindow>,
    pub weekly: Option<LimitWindow>,
    /// "ok" | "stale" | "no_token" | "error"
    pub status: String,
    pub message: Option<String>,
    pub fetched_at: String,
    pub plan_type: Option<String>,
    /// Earned "reset credits" that can clear the windows early.
    pub reset_credits: Option<u64>,
}

impl CodexLiveQuota {
    fn failure(status: &str, message: String) -> Self {
        Self {
            five_hour: None,
            weekly: None,
            status: status.to_string(),
            message: Some(message),
            fetched_at: chrono::Utc::now().to_rfc3339(),
            plan_type: None,
            reset_credits: None,
        }
    }
}

struct Credentials {
    access: String,
    account_id: Option<String>,
}

/// auth.json's `tokens`, or the same JSON from the Keychain when Codex keeps
/// it there (then auth.json is absent).
fn read_credentials(home: &Path) -> Result<Credentials, String> {
    let blob = match std::fs::read_to_string(home.join("auth.json")) {
        Ok(text) => serde_json::from_str::<Value>(&text)
            .map_err(|e| format!("Codex auth.json is not valid JSON: {e}"))?,
        Err(_) => read_keychain(home)?,
    };
    credentials_from(&blob)
}

fn credentials_from(blob: &Value) -> Result<Credentials, String> {
    let tokens = blob
        .get("tokens")
        .filter(|t| t.is_object())
        .ok_or("Codex is not signed in with ChatGPT — API-key logins have no plan limits")?;
    let access = tokens
        .get("access_token")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .ok_or("Codex credential has no access token")?
        .to_string();
    let account_id = tokens
        .get("account_id")
        .and_then(|v| v.as_str())
        .map(String::from)
        .or_else(|| {
            tokens
                .get("id_token")
                .and_then(|v| v.as_str())
                .and_then(jwt_account_id)
        });
    Ok(Credentials { access, account_id })
}

/// The ChatGPT account id claim of an id_token JWT (unverified — it is only
/// echoed back to OpenAI as a header).
fn jwt_account_id(jwt: &str) -> Option<String> {
    let payload = jwt.split('.').nth(1)?;
    let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(payload.trim_end_matches('='))
        .ok()?;
    let claims: Value = serde_json::from_slice(&bytes).ok()?;
    claims
        .get("https://api.openai.com/auth")?
        .get("chatgpt_account_id")?
        .as_str()
        .map(String::from)
}

/// Codex's Keychain account name: "cli|" + the first 16 hex digits of
/// sha256(canonical CODEX_HOME).
fn keychain_account(home: &Path) -> String {
    use sha2::{Digest, Sha256};
    let canonical = home.canonicalize().unwrap_or_else(|_| home.to_path_buf());
    let digest = Sha256::digest(canonical.to_string_lossy().as_bytes());
    let hex: String = digest.iter().map(|b| format!("{b:02x}")).collect();
    format!("cli|{}", &hex[..16])
}

#[cfg(target_os = "macos")]
fn read_keychain(home: &Path) -> Result<Value, String> {
    let out = std::process::Command::new("security")
        .args(["find-generic-password", "-w", "-s", KEYCHAIN_SERVICE, "-a"])
        .arg(keychain_account(home))
        .output()
        .map_err(|e| format!("failed to read Keychain: {e}"))?;
    if !out.status.success() {
        return Err("Codex credentials not found — sign in to Codex with ChatGPT".into());
    }
    serde_json::from_slice(&out.stdout)
        .map_err(|e| format!("Codex Keychain credential is not valid JSON: {e}"))
}

#[cfg(not(target_os = "macos"))]
fn read_keychain(home: &Path) -> Result<Value, String> {
    let _ = (KEYCHAIN_SERVICE, keychain_account(home));
    Err("Codex credentials not found — sign in to Codex with ChatGPT".into())
}

fn fetch_json(creds: &Credentials) -> Result<Value, String> {
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(15))
        .user_agent("token-counter/0.2")
        .build()
        .map_err(|e| e.to_string())?;
    let mut request = client
        .get(USAGE_URL)
        .header("Authorization", format!("Bearer {}", creds.access));
    if let Some(account) = &creds.account_id {
        request = request.header("ChatGPT-Account-Id", account);
    }
    let response = request.send().map_err(|e| e.to_string())?;
    let status = response.status().as_u16();
    if status != 200 {
        return Err(format!("HTTP {status}"));
    }
    response
        .json()
        .map_err(|e| format!("usage response was not JSON: {e}"))
}

/// One /wham/usage window → (window seconds, LimitWindow).
fn parse_window(v: Option<&Value>, now: i64) -> Option<(u64, LimitWindow)> {
    let w = v.filter(|w| w.is_object())?;
    let used_pct = w.get("used_percent")?.as_f64()?;
    let seconds = w
        .get("limit_window_seconds")
        .and_then(|x| x.as_u64())
        .unwrap_or(0);
    let reset = w.get("reset_at").and_then(|x| x.as_i64()).or_else(|| {
        w.get("reset_after_seconds")
            .and_then(|x| x.as_i64())
            .map(|s| now.saturating_add(s.max(0)))
    });
    Some((
        seconds,
        LimitWindow {
            used_pct,
            resets_at: reset
                .and_then(|r| chrono::DateTime::from_timestamp(r, 0))
                .map(|d| d.to_rfc3339()),
        },
    ))
}

/// The main bucket's 5h and weekly windows, told apart by length (±5%, as
/// Codex's /status labels them) and falling back to primary/secondary order.
fn parse_usage(json: &Value, now: i64) -> (Option<LimitWindow>, Option<LimitWindow>) {
    let Some(limit) = json.get("rate_limit").filter(|v| v.is_object()) else {
        return (None, None);
    };
    let mut five = None;
    let mut weekly = None;
    let mut unplaced = Vec::new();
    for key in ["primary_window", "secondary_window"] {
        let Some((seconds, w)) = parse_window(limit.get(key), now) else {
            continue;
        };
        match crate::codex::window_kind(seconds / 60) {
            Some(true) if five.is_none() => five = Some(w),
            Some(false) if weekly.is_none() => weekly = Some(w),
            _ => unplaced.push(w),
        }
    }
    for w in unplaced {
        if five.is_none() {
            five = Some(w);
        } else if weekly.is_none() {
            weekly = Some(w);
        }
    }
    (five, weekly)
}

pub fn fetch() -> CodexLiveQuota {
    let Some(home) = crate::codex::codex_home() else {
        return CodexLiveQuota::failure("no_token", "no home directory".into());
    };
    let creds = match read_credentials(&home) {
        Ok(creds) => creds,
        Err(e) => return CodexLiveQuota::failure("no_token", e),
    };
    match fetch_json(&creds) {
        Ok(json) => {
            let (five_hour, weekly) = parse_usage(&json, chrono::Utc::now().timestamp());
            if five_hour.is_none() && weekly.is_none() {
                return CodexLiveQuota::failure(
                    "error",
                    "OpenAI returned no Codex usage window".into(),
                );
            }
            CodexLiveQuota {
                five_hour,
                weekly,
                status: "ok".into(),
                message: None,
                fetched_at: chrono::Utc::now().to_rfc3339(),
                plan_type: json
                    .get("plan_type")
                    .and_then(|v| v.as_str())
                    .map(String::from),
                reset_credits: json
                    .get("rate_limit_reset_credits")
                    .and_then(|c| c.get("available_count"))
                    .and_then(|v| v.as_u64()),
            }
        }
        Err(e) => {
            let (status, message) = if e.contains("401") || e.contains("403") {
                (
                    "stale",
                    "Codex login expired — open Codex to refresh it".to_string(),
                )
            } else if e.contains("429") {
                ("stale", "Codex usage endpoint is rate limited".to_string())
            } else {
                ("error", e)
            };
            CodexLiveQuota::failure(status, message)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    #[ignore = "reads the real Codex login and calls OpenAI"]
    fn smoke_fetch_real() {
        let quota = fetch();
        eprintln!(
            "status={} plan={:?} 5h={:?} weekly={:?} message={:?}",
            quota.status,
            quota.plan_type,
            quota.five_hour.as_ref().map(|w| w.used_pct),
            quota.weekly.as_ref().map(|w| w.used_pct),
            quota.message
        );
        assert_eq!(quota.status, "ok", "{:?}", quota.message);
    }

    #[test]
    fn maps_windows_by_length_not_position() {
        let json = json!({
            "plan_type": "pro",
            "rate_limit": {
                "allowed": true,
                "limit_reached": false,
                // Weekly reported first, 5h second.
                "primary_window": {"used_percent": 44, "limit_window_seconds": 604800, "reset_after_seconds": 100, "reset_at": 1792483200},
                "secondary_window": {"used_percent": 7.5, "limit_window_seconds": 18000, "reset_after_seconds": 50, "reset_at": 1792400000},
            },
            "additional_rate_limits": [{"limit_name": "gpt-reserve", "metered_feature": "codex_luna", "rate_limit": null}],
            "rate_limit_reset_credits": {"available_count": 2},
        });
        let (five, weekly) = parse_usage(&json, 0);
        assert_eq!(five.as_ref().unwrap().used_pct, 7.5);
        assert_eq!(weekly.as_ref().unwrap().used_pct, 44.0);
        assert_eq!(
            weekly.unwrap().resets_at.as_deref(),
            Some("2026-10-20T08:00:00+00:00")
        );
    }

    #[test]
    fn unknown_window_lengths_fall_back_to_order_and_nulls_are_skipped() {
        let json = json!({
            "rate_limit": {
                "primary_window": {"used_percent": 30, "limit_window_seconds": 7200, "reset_after_seconds": 60, "reset_at": 0},
                "secondary_window": null,
            }
        });
        let (five, weekly) = parse_usage(&json, 1000);
        assert_eq!(five.unwrap().used_pct, 30.0);
        assert!(weekly.is_none());
        assert_eq!(
            parse_usage(&json!({"rate_limit": null}), 0)
                .0
                .map(|w| w.used_pct),
            None
        );
    }

    #[test]
    fn credentials_come_from_tokens_with_id_token_fallback() {
        let claims = json!({"https://api.openai.com/auth": {"chatgpt_account_id": "acct-9"}});
        let payload = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .encode(serde_json::to_vec(&claims).unwrap());
        let blob = json!({
            "auth_mode": "chatgpt",
            "OPENAI_API_KEY": null,
            "tokens": {"id_token": format!("h.{payload}.s"), "access_token": "at", "refresh_token": "rt"},
            "last_refresh": "2026-10-20T00:00:00Z",
        });
        let creds = credentials_from(&blob).unwrap();
        assert_eq!(creds.access, "at");
        assert_eq!(creds.account_id.as_deref(), Some("acct-9"));

        let api_key_only = json!({"OPENAI_API_KEY": "sk-x"});
        assert!(credentials_from(&api_key_only).is_err());
    }

    #[test]
    fn keychain_account_matches_codex_store_key() {
        // A path that can't be canonicalized is hashed as given; Codex keeps
        // the first 16 hex digits of sha256("/nonexistent/.codex").
        let account = keychain_account(Path::new("/nonexistent/.codex"));
        assert_eq!(account, "cli|9571d6c99fb0e3f4");
    }
}

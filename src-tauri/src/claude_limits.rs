// Official Claude subscription limits from the same private endpoint used by
// Claude Code's /usage command and the companion Claude Usage Monitor app.
//
// This integration is intentionally READ-ONLY: it reads Claude Code's OAuth
// access token from Keychain and sends it only to Anthropic. It never rotates
// or writes credentials, because Token Counter and Claude Usage Monitor may be
// running together and refresh-token rotation is single-use.

use serde::{Deserialize, Serialize};
use std::time::Duration;

const KEYCHAIN_SERVICE: &str = "Claude Code-credentials";
const USAGE_URL: &str = "https://api.anthropic.com/api/oauth/usage";
const BETA_HEADER: &str = "oauth-2025-04-20";
const ANTHROPIC_VERSION: &str = "2023-06-01";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LimitWindow {
    pub used_pct: f64,
    pub resets_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClaudeQuota {
    pub five_hour: Option<LimitWindow>,
    pub weekly: Option<LimitWindow>,
    /// "ok" | "stale" | "no_token" | "error"
    pub status: String,
    pub message: Option<String>,
    pub fetched_at: String,
    pub subscription: Option<String>,
}

impl ClaudeQuota {
    fn failure(status: &str, message: String, subscription: Option<String>) -> Self {
        Self {
            five_hour: None,
            weekly: None,
            status: status.to_string(),
            message: Some(message),
            fetched_at: chrono::Utc::now().to_rfc3339(),
            subscription,
        }
    }
}

struct TokenInfo {
    access: String,
    subscription: Option<String>,
}

fn read_token() -> Result<TokenInfo, String> {
    let out = std::process::Command::new("security")
        .args(["find-generic-password", "-w", "-s", KEYCHAIN_SERVICE])
        .output()
        .map_err(|e| format!("failed to read Keychain: {e}"))?;
    if !out.status.success() {
        return Err("Claude Code credentials not found — sign in to Claude Code".into());
    }

    let blob: serde_json::Value = serde_json::from_slice(&out.stdout)
        .map_err(|e| format!("Claude Code credential is not valid JSON: {e}"))?;
    let oauth = &blob["claudeAiOauth"];
    let access = oauth["accessToken"]
        .as_str()
        .ok_or("Claude Code credential has no OAuth access token")?
        .to_string();
    Ok(TokenInfo {
        access,
        subscription: oauth["subscriptionType"].as_str().map(String::from),
    })
}

fn parse_window(v: Option<&serde_json::Value>) -> Option<LimitWindow> {
    let obj = v?.as_object()?;
    Some(LimitWindow {
        used_pct: obj.get("utilization")?.as_f64()?,
        resets_at: obj
            .get("resets_at")
            .and_then(|x| x.as_str())
            .map(String::from),
    })
}

fn fetch_json(access: &str) -> Result<serde_json::Value, String> {
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(15))
        .user_agent("token-counter/0.2")
        .build()
        .map_err(|e| e.to_string())?;
    let response = client
        .get(USAGE_URL)
        .header("Authorization", format!("Bearer {access}"))
        .header("anthropic-beta", BETA_HEADER)
        .header("anthropic-version", ANTHROPIC_VERSION)
        .send()
        .map_err(|e| e.to_string())?;
    let status = response.status().as_u16();
    if status != 200 {
        return Err(format!("HTTP {status}"));
    }
    response
        .json()
        .map_err(|e| format!("usage response was not JSON: {e}"))
}

pub fn fetch() -> ClaudeQuota {
    let token = match read_token() {
        Ok(token) => token,
        Err(e) => return ClaudeQuota::failure("no_token", e, None),
    };
    match fetch_json(&token.access) {
        Ok(json) => {
            let five_hour = parse_window(json.get("five_hour"));
            let weekly = parse_window(json.get("seven_day"));
            if five_hour.is_none() && weekly.is_none() {
                return ClaudeQuota::failure(
                    "error",
                    "Anthropic returned no active 5h or weekly window".into(),
                    token.subscription,
                );
            }
            ClaudeQuota {
                five_hour,
                weekly,
                status: "ok".into(),
                message: None,
                fetched_at: chrono::Utc::now().to_rfc3339(),
                subscription: token.subscription,
            }
        }
        Err(e) => {
            let (status, message) = if e.contains("401") {
                (
                    "stale",
                    "Claude OAuth token expired — open Claude Code to refresh".to_string(),
                )
            } else if e.contains("429") {
                (
                    "stale",
                    "Anthropic usage endpoint is rate limited".to_string(),
                )
            } else {
                ("error", e)
            };
            ClaudeQuota::failure(status, message, token.subscription)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "reads the real macOS Keychain and Anthropic endpoint"]
    fn smoke_fetch_real() {
        let quota = fetch();
        eprintln!(
            "status={} 5h={:?} weekly={:?} fetched_at={}",
            quota.status,
            quota.five_hour.as_ref().map(|w| w.used_pct),
            quota.weekly.as_ref().map(|w| w.used_pct),
            quota.fetched_at
        );
        assert_eq!(quota.status, "ok", "{:?}", quota.message);
        assert!(quota.five_hour.is_some());
        assert!(quota.weekly.is_some());
    }

    #[test]
    fn parses_active_windows_and_ignores_nulls() {
        let json: serde_json::Value = serde_json::from_str(
            r#"{
                "five_hour":{"utilization":74.0,"resets_at":"2026-07-03T08:30:00Z"},
                "seven_day":{"utilization":31.5,"resets_at":"2026-07-08T00:00:00Z"},
                "seven_day_opus":null
            }"#,
        )
        .unwrap();
        let five = parse_window(json.get("five_hour")).unwrap();
        let weekly = parse_window(json.get("seven_day")).unwrap();
        assert_eq!(five.used_pct, 74.0);
        assert_eq!(weekly.used_pct, 31.5);
        assert_eq!(five.resets_at.as_deref(), Some("2026-07-03T08:30:00Z"));
        assert!(parse_window(json.get("seven_day_opus")).is_none());
    }
}

// Official Claude subscription limits from the same private endpoint used by
// Claude Code's /usage command and the companion Claude Usage Monitor app.
//
// It reads Claude Code's OAuth access token from Keychain and sends it only to
// Anthropic. Until 2026-11-06 this was strictly read-only; now an expired token
// (HTTP 401) is refreshed in place under the guards in claude_refresh.rs,
// because Claude Code in the desktop app never refreshes this Keychain item.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::time::Duration;

const KEYCHAIN_SERVICE: &str = "Claude Code-credentials";
const USAGE_URL: &str = "https://api.anthropic.com/api/oauth/usage";
const BETA_HEADER: &str = "oauth-2025-04-20";
const ANTHROPIC_VERSION: &str = "2023-06-01";
/// The message a 429 from the usage endpoint carries (lib.rs keys its backoff on it).
pub const RATE_LIMITED: &str = "Anthropic usage endpoint is rate limited";

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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub breakdown: Option<WeekBreakdown>,
}

/// One product's percentage of the weekly usage so far.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProductShare {
    pub key: String,
    pub percent: f64,
}

/// `seven_day_breakdown`: how the week's usage splits across Claude Code, Chats,
/// Cowork, Claude Duet. Only Claude Code has local logs, so this is all there
/// is on the others (see usage.rs, ProductShare use).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WeekBreakdown {
    pub window_start: String,
    pub rows: Vec<ProductShare>,
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
            breakdown: None,
        }
    }
}

struct TokenInfo {
    access: String,
    subscription: Option<String>,
}

fn read_token() -> Result<TokenInfo, String> {
    let blob = crate::claude_refresh::read_blob(KEYCHAIN_SERVICE)?;
    let oauth = &blob["claudeAiOauth"];
    let access = oauth["accessToken"]
        .as_str()
        .filter(|a| !a.is_empty())
        .ok_or(crate::claude_refresh::SIGNED_OUT)?
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

fn parse_breakdown(v: Option<&serde_json::Value>) -> Option<WeekBreakdown> {
    let v = v?;
    Some(WeekBreakdown {
        window_start: v.get("window_started_at")?.as_str()?.to_string(),
        rows: v
            .get("rows")?
            .as_array()?
            .iter()
            .filter_map(|r| {
                Some(ProductShare {
                    key: r.get("key")?.as_str()?.to_string(),
                    percent: r.get("percent")?.as_f64()?,
                })
            })
            .collect(),
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
    let mut result = fetch_json(&token.access);
    if result.as_ref().is_err_and(|e| e.contains("401")) {
        result = match crate::claude_refresh::refreshed_access(KEYCHAIN_SERVICE, &token.access) {
            Ok(access) => fetch_json(&access),
            Err(message) => return ClaudeQuota::failure("stale", message, token.subscription),
        };
    }
    match result {
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
                breakdown: parse_breakdown(json.get("seven_day_breakdown")),
            }
        }
        Err(e) => {
            let (status, message) = if e.contains("401") {
                (
                    "stale",
                    "Claude OAuth token rejected even after a refresh".to_string(),
                )
            } else if e.contains("429") {
                ("stale", RATE_LIMITED.to_string())
            } else {
                ("error", e)
            };
            ClaudeQuota::failure(status, message, token.subscription)
        }
    }
}

/// One reading of the 5h and weekly meters by the Claude desktop app.
#[derive(Debug, Clone, PartialEq)]
struct DesktopSample {
    at: DateTime<Utc>,
    five_hour: f64,
    weekly: f64,
}

// plan-usage-history.json, {"samples":[{"t":<ms>,"u":{"fh":<5h %>,"sd":<weekly %>}}]}.
fn newest_sample(json: &serde_json::Value) -> Option<DesktopSample> {
    json.get("samples")?
        .as_array()?
        .iter()
        .filter_map(|s| {
            Some(DesktopSample {
                at: DateTime::from_timestamp_millis(s.get("t")?.as_i64()?)?,
                five_hour: s.get("u")?.get("fh")?.as_f64()?,
                weekly: s.get("u")?.get("sd")?.as_f64()?,
            })
        })
        .max_by_key(|s| s.at)
}

// The first weekly reset after `at`. The reset recurs every 7 days, so one seen
// in an old snapshot still gives the window the sample belongs to.
fn weekly_reset_after(known: &str, at: DateTime<Utc>) -> Option<DateTime<Utc>> {
    let known = DateTime::parse_from_rfc3339(known)
        .ok()?
        .with_timezone(&Utc);
    let week = chrono::Duration::days(7);
    let elapsed = (at - known).num_milliseconds();
    Some(known + week * (elapsed.div_euclid(week.num_milliseconds()) as i32 + 1))
}

fn apply_sample(quota: ClaudeQuota, sample: &DesktopSample) -> ClaudeQuota {
    let snapshot_at = DateTime::parse_from_rfc3339(&quota.fetched_at)
        .ok()
        .map(|t| t.with_timezone(&Utc));
    let has_snapshot = quota.five_hour.is_some() || quota.weekly.is_some();
    if has_snapshot && snapshot_at.is_some_and(|t| t >= sample.at) {
        return quota;
    }
    let reset = quota
        .weekly
        .as_ref()
        .and_then(|w| w.resets_at.as_deref())
        .and_then(|r| weekly_reset_after(r, sample.at));
    ClaudeQuota {
        five_hour: Some(LimitWindow {
            used_pct: sample.five_hour,
            // The sample carries no reset time, but the window it read ends
            // within 5h, so the UI won't keep showing it past that.
            resets_at: Some((sample.at + chrono::Duration::hours(5)).to_rfc3339()),
        }),
        weekly: Some(LimitWindow {
            used_pct: sample.weekly,
            resets_at: reset.map(|r| r.to_rfc3339()),
        }),
        status: "stale".into(),
        message: quota
            .message
            .map(|m| format!("{m}; showing the Claude desktop app's last reading")),
        fetched_at: sample.at.to_rfc3339(),
        subscription: quota.subscription,
        breakdown: None,
    }
}

/// The desktop app signs in on its own and never refreshes the Keychain token.
/// We refresh it now (claude_refresh.rs), but a signed-out Claude Code or a
/// deferred refresh still leaves no live read. The desktop app keeps its own
/// meter readings, sparse but needing no token. When the live read failed, use
/// the newest one if it beats the snapshot.
pub fn with_desktop_sample(quota: ClaudeQuota) -> ClaudeQuota {
    if quota.status == "ok" {
        return quota;
    }
    let sample = dirs::data_dir()
        .and_then(|d| std::fs::read(d.join("Claude").join("plan-usage-history.json")).ok())
        .and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok())
        .and_then(|j| newest_sample(&j));
    match sample {
        Some(s) => apply_sample(quota, &s),
        None => quota,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stale_snapshot(fetched_at: &str) -> ClaudeQuota {
        ClaudeQuota {
            five_hour: Some(LimitWindow {
                used_pct: 14.0,
                resets_at: Some("2026-10-28T18:49:59Z".into()),
            }),
            weekly: Some(LimitWindow {
                used_pct: 3.0,
                resets_at: Some("2026-11-04T05:00:00Z".into()),
            }),
            status: "stale".into(),
            message: Some("token expired".into()),
            fetched_at: fetched_at.into(),
            subscription: Some("max".into()),
            breakdown: None,
        }
    }

    fn at(s: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(s).unwrap().with_timezone(&Utc)
    }

    #[test]
    fn parses_the_week_breakdown() {
        let json: serde_json::Value = serde_json::from_str(
            r#"{"as_of":"2026-11-02T07:07:50+00:00",
                "window_started_at":"2026-10-28T05:00:01.027108+00:00",
                "rows":[{"key":"claude_code","display_name":"Claude Code","percent":80},
                        {"key":"chat","display_name":"Chats","percent":2},
                        {"key":"bad"}]}"#,
        )
        .unwrap();
        let b = parse_breakdown(Some(&json)).unwrap();
        assert_eq!(b.window_start, "2026-10-28T05:00:01.027108+00:00");
        assert_eq!(b.rows.len(), 2);
        assert_eq!(b.rows[1].key, "chat");
        assert_eq!(b.rows[1].percent, 2.0);
        assert!(parse_breakdown(None).is_none());
    }

    #[test]
    fn newest_sample_ignores_order_and_bad_rows() {
        let json: serde_json::Value = serde_json::from_str(
            r#"{"version":2,"samples":[
                {"t":1793596200722,"org":"a","u":{"fh":0,"sd":69}},
                {"t":1793589820704,"org":"a","u":{"fh":100,"sd":69}},
                {"t":1793599999999,"org":"a","u":{"fh":1}},
                {"org":"a"}
            ]}"#,
        )
        .unwrap();
        let s = newest_sample(&json).unwrap();
        assert_eq!(s.at.timestamp_millis(), 1793596200722);
        assert_eq!((s.five_hour, s.weekly), (0.0, 69.0));
        assert!(newest_sample(&serde_json::json!({})).is_none());
    }

    #[test]
    fn newer_sample_replaces_a_stale_snapshot() {
        let s = DesktopSample {
            at: at("2026-11-02T05:10:00Z"),
            five_hour: 12.0,
            weekly: 69.0,
        };
        let q = apply_sample(stale_snapshot("2026-10-28T16:22:06Z"), &s);
        assert_eq!(q.status, "stale");
        assert_eq!(q.weekly.as_ref().unwrap().used_pct, 69.0);
        assert_eq!(q.five_hour.as_ref().unwrap().used_pct, 12.0);
        assert_eq!(
            at(q.weekly.unwrap().resets_at.as_deref().unwrap()),
            at("2026-11-04T05:00:00Z")
        );
        assert_eq!(
            at(q.five_hour.unwrap().resets_at.as_deref().unwrap()),
            at("2026-11-02T10:10:00Z")
        );
        assert_eq!(at(&q.fetched_at), s.at);
        assert!(q.message.unwrap().contains("desktop app"));
    }

    #[test]
    fn older_sample_keeps_the_snapshot() {
        let s = DesktopSample {
            at: at("2026-10-28T02:06:00Z"),
            five_hour: 0.0,
            weekly: 0.0,
        };
        let q = apply_sample(stale_snapshot("2026-10-28T16:22:06Z"), &s);
        assert_eq!(q.weekly.unwrap().used_pct, 3.0);
    }

    #[test]
    fn sample_fills_in_when_no_snapshot_exists() {
        let failed = ClaudeQuota::failure("no_token", "signed out".into(), None);
        let s = DesktopSample {
            at: Utc::now() - chrono::Duration::hours(1),
            five_hour: 5.0,
            weekly: 20.0,
        };
        let q = apply_sample(failed, &s);
        assert_eq!(q.status, "stale");
        assert_eq!(q.weekly.as_ref().unwrap().used_pct, 20.0);
        assert!(q.weekly.unwrap().resets_at.is_none());
    }

    #[test]
    fn weekly_reset_steps_to_the_sample_window() {
        let known = "2026-10-28T05:00:00Z";
        assert_eq!(
            weekly_reset_after(known, at("2026-11-02T05:10:00Z")),
            Some(at("2026-11-04T05:00:00Z"))
        );
        assert_eq!(
            weekly_reset_after("2026-11-04T05:00:00Z", at("2026-11-02T05:10:00Z")),
            Some(at("2026-11-04T05:00:00Z"))
        );
        assert_eq!(
            weekly_reset_after("2026-11-04T05:00:00Z", at("2026-11-12T00:00:00Z")),
            Some(at("2026-11-18T05:00:00Z"))
        );
    }

    #[test]
    #[ignore = "reads the real desktop app sample file"]
    fn smoke_desktop_sample_real() {
        let q = with_desktop_sample(stale_snapshot("2026-10-28T16:22:06Z"));
        eprintln!(
            "status={} 5h={:?} weekly={:?} as_of={} reset={:?}",
            q.status,
            q.five_hour.as_ref().map(|w| w.used_pct),
            q.weekly.as_ref().map(|w| w.used_pct),
            q.fetched_at,
            q.weekly.and_then(|w| w.resets_at),
        );
    }

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

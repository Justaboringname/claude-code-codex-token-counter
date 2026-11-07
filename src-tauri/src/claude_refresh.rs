// Refreshes Claude Code's OAuth token in the Keychain when the usage endpoint
// answers 401 (added 2026-11-06 at the user's request).
//
// Why: the token lasts ~8h and only a terminal `claude` refreshes it. Claude
// Code in the desktop app authenticates through the app and never touches this
// Keychain item, so a desktop-only day left the official limits stale (10-29,
// 11-04).
//
// The refresh token is single-use, so this has to coexist with a terminal
// `claude` (and Claude Usage Monitor, which refreshes the same item):
// - Only on a 401, i.e. after the token really expired. An active terminal
//   `claude` refreshes before that, so a 401 means nobody else just did.
// - Skipped while a non-desktop Claude Code transcript was written in the
//   last 2 minutes: that session will refresh on its own.
// - Re-read the item first; if its access token already changed, use that.
// - The token request matches Claude Code's (same endpoint, client id and
//   body); the write goes through `security`, like Claude Code, under Claude
//   Code's own write lock (`~/.claude/.storage-write.lock`, proper-lockfile).
// - Compare-and-swap: the new tokens are written only if the item still holds
//   the refresh token we used, or an empty one (Claude Code blanks a refresh
//   token it saw rejected; writing then signs it back in). Every other field
//   of the blob (MCP logins etc.) is kept.
// - invalid_grant on an unchanged item means signed out: that refresh token
//   is never retried; other failures back off 10 minutes.
// Tokens never leave this process except to Anthropic and `security`.

use serde_json::Value;
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant, SystemTime};

const TOKEN_URL: &str = "https://platform.claude.com/v1/oauth/token";
const CLIENT_ID: &str = "9d1c250a-e61b-44d9-88ed-5944d1962f5e";
const RETRY_AFTER_FAILURE: Duration = Duration::from_secs(10 * 60);
const CLI_QUIET: Duration = Duration::from_secs(2 * 60);
// proper-lockfile treats a lock whose mtime is older than this as abandoned.
const LOCK_STALE: Duration = Duration::from_secs(15);
// Claude Code's own cut-over from `security -i` (stdin) to argv.
const STDIN_LIMIT: usize = 4000;

/// New tokens whose Keychain write failed. The refresh token they replace is
/// spent, so they are kept and written on the next try.
struct Pending {
    used_refresh: String,
    blob: Value,
}

struct State {
    last_failure: Option<Instant>,
    /// sha256 of a refresh token Anthropic rejected; never sent again.
    dead: Option<[u8; 32]>,
    pending: Option<Pending>,
}

static STATE: Mutex<State> = Mutex::new(State {
    last_failure: None,
    dead: None,
    pending: None,
});

pub const SIGNED_OUT: &str = "Claude Code is signed out — run `claude auth login` in Terminal";

fn account() -> Option<String> {
    let user = std::env::var("USER").ok()?;
    let ok = !user.is_empty()
        && user
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'));
    Some(if ok { user } else { "claude-code-user".into() })
}

/// The account the item is stored under (its `acct` attribute), else $USER.
fn keychain_account(service: &str) -> Option<String> {
    let out = std::process::Command::new("security")
        .args(["find-generic-password", "-s", service])
        .output()
        .ok()?;
    let attrs = String::from_utf8_lossy(&out.stdout);
    parse_acct(&attrs).or_else(account)
}

fn parse_acct(attrs: &str) -> Option<String> {
    let line = attrs
        .lines()
        .find(|l| l.trim_start().starts_with("\"acct\""))?;
    let value = line.split_once("=\"")?.1.strip_suffix('"')?;
    (!value.is_empty()).then(|| value.to_string())
}

pub fn read_blob(service: &str) -> Result<Value, String> {
    let mut args = vec!["find-generic-password"];
    let acct = account();
    if let Some(a) = acct.as_deref() {
        args.extend(["-a", a]);
    }
    args.extend(["-w", "-s", service]);
    let out = std::process::Command::new("security")
        .args(&args)
        .output()
        .map_err(|e| format!("failed to read Keychain: {e}"))?;
    if !out.status.success() {
        return Err("Claude Code credentials not found — sign in to Claude Code".into());
    }
    let text = String::from_utf8_lossy(&out.stdout);
    let text = text.trim();
    // `security -w` prints non-ASCII data as hex.
    let bytes = if text.starts_with('{') {
        text.as_bytes().to_vec()
    } else {
        decode_hex(text).ok_or("Claude Code credential is not valid JSON")?
    };
    serde_json::from_slice(&bytes)
        .map_err(|e| format!("Claude Code credential is not valid JSON: {e}"))
}

fn write_blob(service: &str, acct: &str, blob: &Value) -> Result<(), String> {
    let hex = encode_hex(blob.to_string().as_bytes());
    let line = format!("add-generic-password -U -a \"{acct}\" -s \"{service}\" -X \"{hex}\"\n");
    let status = if line.len() <= STDIN_LIMIT {
        use std::io::Write;
        let mut child = std::process::Command::new("security")
            .arg("-i")
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .map_err(|e| format!("security: {e}"))?;
        child
            .stdin
            .take()
            .ok_or("security: no stdin")?
            .write_all(line.as_bytes())
            .map_err(|e| format!("security: {e}"))?;
        child.wait().map_err(|e| format!("security: {e}"))?
    } else {
        std::process::Command::new("security")
            .args([
                "add-generic-password",
                "-U",
                "-a",
                acct,
                "-s",
                service,
                "-X",
                &hex,
            ])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .map_err(|e| format!("security: {e}"))?
    };
    if status.success() {
        Ok(())
    } else {
        Err(format!("Keychain write failed ({status})"))
    }
}

/// The old refresh token is spent once a refresh succeeded; losing the write
/// would sign Claude Code out at its next refresh, so try twice.
fn write_with_retry(service: &str, acct: &str, blob: &Value) -> Result<(), String> {
    write_blob(service, acct, blob).or_else(|_| {
        std::thread::sleep(Duration::from_millis(300));
        write_blob(service, acct, blob)
    })
}

fn encode_hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

fn decode_hex(s: &str) -> Option<Vec<u8>> {
    if !s.len().is_multiple_of(2) {
        return None;
    }
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(s.get(i..i + 2)?, 16).ok())
        .collect()
}

fn oauth_str<'a>(blob: &'a Value, key: &str) -> &'a str {
    blob["claudeAiOauth"][key].as_str().unwrap_or("")
}

fn digest(s: &str) -> [u8; 32] {
    Sha256::digest(s.as_bytes()).into()
}

// ---- Claude Code's write lock ----------------------------------------------

fn config_dir() -> Option<PathBuf> {
    match std::env::var("CLAUDE_SECURESTORAGE_CONFIG_DIR") {
        Ok(d) if !d.is_empty() => return Some(PathBuf::from(d)),
        _ => {}
    }
    match std::env::var("CLAUDE_CONFIG_DIR") {
        Ok(d) if !d.is_empty() => Some(PathBuf::from(d)),
        _ => dirs::home_dir().map(|h| h.join(".claude")),
    }
}

/// proper-lockfile's lock: a directory created with mkdir, abandoned once its
/// mtime is older than 15 s. Released on drop.
struct WriteLock(PathBuf);

impl WriteLock {
    fn acquire(dir: &Path) -> Result<Self, String> {
        let path = dir.join(".storage-write.lock");
        for attempt in 0..20 {
            match std::fs::create_dir(&path) {
                Ok(()) => return Ok(Self(path)),
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                    let abandoned = std::fs::metadata(&path)
                        .and_then(|m| m.modified())
                        .ok()
                        .and_then(|t| SystemTime::now().duration_since(t).ok())
                        .is_some_and(|age| age > LOCK_STALE);
                    if abandoned {
                        let _ = std::fs::remove_dir(&path);
                        continue;
                    }
                    std::thread::sleep(Duration::from_millis(100 + 50 * attempt));
                }
                Err(e) => return Err(format!("write lock: {e}")),
            }
        }
        Err("Claude Code is holding its credential write lock".into())
    }
}

impl Drop for WriteLock {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir(&self.0);
    }
}

// ---- Is a terminal Claude Code at work? ------------------------------------

/// Any `"entrypoint":"…"` other than the desktop app's in this transcript tail.
fn tail_has_terminal_entrypoint(tail: &str) -> bool {
    const KEY: &str = "\"entrypoint\":\"";
    tail.match_indices(KEY).any(|(i, _)| {
        let rest = &tail[i + KEY.len()..];
        let value = rest.split('"').next().unwrap_or("");
        !value.is_empty() && value != "claude-desktop"
    })
}

fn read_tail(path: &Path, bytes: u64) -> Option<String> {
    use std::io::{Read, Seek, SeekFrom};
    let mut f = std::fs::File::open(path).ok()?;
    let len = f.metadata().ok()?.len();
    f.seek(SeekFrom::Start(len.saturating_sub(bytes))).ok()?;
    let mut buf = Vec::new();
    f.read_to_end(&mut buf).ok()?;
    Some(String::from_utf8_lossy(&buf).into_owned())
}

/// A transcript written in the last 2 minutes by Claude Code outside the
/// desktop app (terminal, `claude -p`, SDK). Errors count as yes.
fn terminal_claude_active(projects: &Path) -> bool {
    let Some(pattern) = projects
        .join("**")
        .join("*.jsonl")
        .to_str()
        .map(String::from)
    else {
        return true;
    };
    let Ok(paths) = glob::glob(&pattern) else {
        return true;
    };
    let now = SystemTime::now();
    paths.flatten().any(|p| {
        let recent = std::fs::metadata(&p)
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| now.duration_since(t).ok())
            .is_some_and(|age| age < CLI_QUIET);
        recent && read_tail(&p, 64 * 1024).is_none_or(|t| tail_has_terminal_entrypoint(&t))
    })
}

// ---- The refresh -----------------------------------------------------------

fn token_request(refresh: &str, scopes: &Value) -> Value {
    let mut body = serde_json::json!({
        "grant_type": "refresh_token",
        "refresh_token": refresh,
        "client_id": CLIENT_ID,
    });
    let scope: Vec<&str> = scopes
        .as_array()
        .map(|a| a.iter().filter_map(Value::as_str).collect())
        .unwrap_or_default();
    if !scope.is_empty() {
        body["scope"] = Value::String(scope.join(" "));
    }
    body
}

/// The blob with the token response applied, every other field untouched.
fn apply_response(
    mut blob: Value,
    used_refresh: &str,
    resp: &Value,
    now_ms: i64,
) -> Result<Value, String> {
    let access = resp["access_token"]
        .as_str()
        .filter(|s| !s.is_empty())
        .ok_or("token response has no access_token")?;
    let oauth = blob["claudeAiOauth"]
        .as_object_mut()
        .ok_or("credential has no claudeAiOauth")?;
    let refresh = resp["refresh_token"].as_str().unwrap_or(used_refresh);
    let expires_in = resp["expires_in"].as_i64().unwrap_or(8 * 3600);
    oauth.insert("accessToken".into(), access.into());
    oauth.insert("refreshToken".into(), refresh.into());
    oauth.insert("expiresAt".into(), (now_ms + expires_in * 1000).into());
    if let Some(r) = resp["refresh_token_expires_in"].as_i64() {
        oauth.insert("refreshTokenExpiresAt".into(), (now_ms + r * 1000).into());
    }
    if let Some(scope) = resp["scope"].as_str().filter(|s| !s.trim().is_empty()) {
        let scopes: Vec<Value> = scope.split_whitespace().map(Value::from).collect();
        oauth.insert("scopes".into(), Value::Array(scopes));
    }
    Ok(blob)
}

/// Compare-and-swap test: still our refresh token, or blanked after a reject.
fn may_overwrite(current_refresh: &str, used_refresh: &str) -> bool {
    current_refresh == used_refresh || current_refresh.is_empty()
}

fn is_invalid_grant(body: &str) -> bool {
    serde_json::from_str::<Value>(body)
        .ok()
        .and_then(|v| v["error"].as_str().map(|e| e == "invalid_grant"))
        .unwrap_or_else(|| body.contains("invalid_grant"))
}

/// A usable access token after `stale` got a 401: someone else's newer one,
/// or ours from a refresh. Err is the message to show.
pub fn refreshed_access(service: &str, stale: &str) -> Result<String, String> {
    let mut state = STATE.lock().unwrap_or_else(|e| e.into_inner());

    let blob = read_blob(service)?;

    // Tokens from an earlier refresh whose write failed are the live ones,
    // unless someone has written newer tokens since.
    if let Some(p) = state.pending.take() {
        if may_overwrite(oauth_str(&blob, "refreshToken"), &p.used_refresh) {
            let access = oauth_str(&p.blob, "accessToken").to_string();
            let written = keychain_account(service)
                .ok_or_else(|| "no Keychain account".to_string())
                .and_then(|acct| write_with_retry(service, &acct, &p.blob));
            if let Err(e) = written {
                eprintln!("[claude_refresh] {e}");
                state.pending = Some(p);
            }
            return Ok(access);
        }
    }

    let current = oauth_str(&blob, "accessToken");
    if !current.is_empty() && current != stale {
        return Ok(current.to_string());
    }
    let refresh = oauth_str(&blob, "refreshToken").to_string();
    if refresh.is_empty() || state.dead == Some(digest(&refresh)) {
        return Err(SIGNED_OUT.into());
    }
    if state
        .last_failure
        .is_some_and(|t| t.elapsed() < RETRY_AFTER_FAILURE)
    {
        return Err(
            "Claude OAuth token expired; refresh failed recently, retrying in a few minutes".into(),
        );
    }
    let dir = config_dir().ok_or("no home directory")?;
    if terminal_claude_active(&dir.join("projects")) {
        return Err(
            "Claude OAuth token expired; Claude Code in Terminal is active and will refresh it"
                .into(),
        );
    }
    // Know where to write before spending the refresh token.
    let acct = keychain_account(service)
        .ok_or("can't tell which Keychain account holds the Claude Code credential")?;

    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(10))
        .user_agent("token-counter/0.2")
        .build()
        .map_err(|e| e.to_string())?;
    let sent = client
        .post(TOKEN_URL)
        .json(&token_request(&refresh, &blob["claudeAiOauth"]["scopes"]))
        .send();
    let (code, body) = match sent {
        Ok(r) => {
            let code = r.status().as_u16();
            (code, r.text().unwrap_or_default())
        }
        Err(e) => {
            state.last_failure = Some(Instant::now());
            return Err(format!("token refresh failed: {e}"));
        }
    };

    if code != 200 {
        // Someone may have rotated it between our read and our request.
        if let Ok(now) = read_blob(service) {
            let access = oauth_str(&now, "accessToken");
            if oauth_str(&now, "refreshToken") != refresh && !access.is_empty() && access != stale {
                return Ok(access.to_string());
            }
        }
        if (code == 400 || code == 401) && is_invalid_grant(&body) {
            state.dead = Some(digest(&refresh));
            return Err(SIGNED_OUT.into());
        }
        state.last_failure = Some(Instant::now());
        return Err(format!("token refresh failed: HTTP {code}"));
    }

    let resp: Value = serde_json::from_str(&body).map_err(|e| {
        state.last_failure = Some(Instant::now());
        format!("token refresh response was not JSON: {e}")
    })?;
    let now_ms = chrono::Utc::now().timestamp_millis();

    // From here the old refresh token is spent: nothing may drop the new
    // tokens. The lock is advisory, so a busy lock doesn't stop the write.
    let lock = WriteLock::acquire(&dir)
        .map_err(|e| eprintln!("[claude_refresh] {e}; writing without it"))
        .ok();
    let latest = read_blob(service).unwrap_or_else(|_| blob.clone());
    let latest_refresh = oauth_str(&latest, "refreshToken");
    if !may_overwrite(latest_refresh, &refresh) {
        // Rotated by someone else meanwhile: theirs is current, ours unused.
        let access = oauth_str(&latest, "accessToken");
        return if access.is_empty() {
            Err(SIGNED_OUT.into())
        } else {
            Ok(access.to_string())
        };
    }
    let updated = apply_response(latest, &refresh, &resp, now_ms)
        .or_else(|_| apply_response(blob.clone(), &refresh, &resp, now_ms))?;
    match write_with_retry(service, &acct, &updated) {
        Ok(()) => state.last_failure = None,
        Err(e) => {
            eprintln!("[claude_refresh] {e}; keeping the new tokens to write later");
            state.pending = Some(Pending {
                used_refresh: refresh,
                blob: updated.clone(),
            });
        }
    }
    drop(lock);
    Ok(oauth_str(&updated, "accessToken").to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn blob() -> Value {
        serde_json::json!({
            "claudeAiOauth": {
                "accessToken": "old-access",
                "refreshToken": "old-refresh",
                "expiresAt": 1,
                "scopes": ["user:inference", "user:profile"],
                "subscriptionType": "max",
                "rateLimitTier": "tier"
            },
            "mcpOAuth": {"server|x": {"accessToken": "mcp"}}
        })
    }

    #[test]
    fn response_updates_tokens_and_keeps_everything_else() {
        let resp = serde_json::json!({
            "access_token": "new-access",
            "refresh_token": "new-refresh",
            "expires_in": 28800,
            "refresh_token_expires_in": 100,
            "scope": "user:inference user:profile user:plugins"
        });
        let b = apply_response(blob(), "old-refresh", &resp, 1_000).unwrap();
        let o = &b["claudeAiOauth"];
        assert_eq!(o["accessToken"], "new-access");
        assert_eq!(o["refreshToken"], "new-refresh");
        assert_eq!(o["expiresAt"], 1_000 + 28_800_000);
        assert_eq!(o["refreshTokenExpiresAt"], 101_000);
        assert_eq!(o["scopes"].as_array().unwrap().len(), 3);
        assert_eq!(o["subscriptionType"], "max");
        assert_eq!(o["rateLimitTier"], "tier");
        assert_eq!(b["mcpOAuth"]["server|x"]["accessToken"], "mcp");
    }

    #[test]
    fn response_without_refresh_token_keeps_the_used_one() {
        let resp = serde_json::json!({"access_token": "a", "expires_in": 60});
        let b = apply_response(blob(), "old-refresh", &resp, 0).unwrap();
        assert_eq!(b["claudeAiOauth"]["refreshToken"], "old-refresh");
        assert_eq!(b["claudeAiOauth"]["scopes"].as_array().unwrap().len(), 2);
        assert!(apply_response(blob(), "r", &serde_json::json!({}), 0).is_err());
    }

    #[test]
    fn request_matches_claude_code() {
        let body = token_request("r", &blob()["claudeAiOauth"]["scopes"]);
        assert_eq!(body["grant_type"], "refresh_token");
        assert_eq!(body["refresh_token"], "r");
        assert_eq!(body["client_id"], CLIENT_ID);
        assert_eq!(body["scope"], "user:inference user:profile");
        assert!(token_request("r", &Value::Null).get("scope").is_none());
    }

    #[test]
    fn overwrite_only_our_or_a_blanked_token() {
        assert!(may_overwrite("old", "old"));
        assert!(may_overwrite("", "old"));
        assert!(!may_overwrite("newer-from-cli", "old"));
    }

    #[test]
    fn invalid_grant_detection() {
        assert!(is_invalid_grant(
            r#"{"error":"invalid_grant","error_description":"x"}"#
        ));
        assert!(!is_invalid_grant(r#"{"error":"invalid_request"}"#));
        assert!(is_invalid_grant("oops invalid_grant"));
    }

    #[test]
    fn entrypoints_other_than_desktop_count_as_terminal() {
        assert!(!tail_has_terminal_entrypoint(
            r#"{"entrypoint":"claude-desktop","x":1}"#
        ));
        assert!(tail_has_terminal_entrypoint(r#"{"entrypoint":"cli"}"#));
        assert!(tail_has_terminal_entrypoint(
            r#"{"entrypoint":"claude-desktop"}{"entrypoint":"sdk-cli"}"#
        ));
        assert!(!tail_has_terminal_entrypoint("no entrypoint here"));
    }

    #[test]
    fn parses_the_item_account() {
        let attrs = "keychain: \"/x.keychain-db\"\nattributes:\n    \"acct\"<blob>=\"someone\"\n    \"svce\"<blob>=\"Claude Code-credentials\"\n";
        assert_eq!(parse_acct(attrs).as_deref(), Some("someone"));
        assert!(parse_acct("attributes:\n    \"acct\"<blob>=<NULL>\n").is_none());
        assert!(parse_acct("").is_none());
    }

    #[test]
    fn hex_round_trip() {
        let s = r#"{"a":"é"}"#;
        assert_eq!(decode_hex(&encode_hex(s.as_bytes())).unwrap(), s.as_bytes());
        assert!(decode_hex("abc").is_none());
        assert!(decode_hex("zz").is_none());
    }

    #[test]
    fn write_lock_is_exclusive_and_takes_over_abandoned_locks() {
        let dir = std::env::temp_dir().join(format!("tc-lock-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let lock = WriteLock::acquire(&dir).unwrap();
        assert!(dir.join(".storage-write.lock").is_dir());
        drop(lock);
        assert!(!dir.join(".storage-write.lock").exists());

        // An abandoned lock (mtime > 15 s ago) is taken over.
        let path = dir.join(".storage-write.lock");
        std::fs::create_dir(&path).unwrap();
        let old = SystemTime::now() - Duration::from_secs(60);
        std::fs::File::open(&path)
            .unwrap()
            .set_modified(old)
            .unwrap();
        let lock = WriteLock::acquire(&dir).unwrap();
        drop(lock);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn recent_terminal_transcript_blocks_refresh() {
        let dir = std::env::temp_dir().join(format!("tc-proj-{}", std::process::id()));
        let sub = dir.join("-Users-x");
        std::fs::create_dir_all(&sub).unwrap();
        std::fs::write(sub.join("a.jsonl"), "{\"entrypoint\":\"claude-desktop\"}\n").unwrap();
        assert!(!terminal_claude_active(&dir));
        std::fs::write(sub.join("b.jsonl"), "{\"entrypoint\":\"cli\"}\n").unwrap();
        assert!(terminal_claude_active(&dir));
        let old = SystemTime::now() - Duration::from_secs(600);
        std::fs::File::options()
            .write(true)
            .open(sub.join("b.jsonl"))
            .unwrap()
            .set_modified(old)
            .unwrap();
        assert!(!terminal_claude_active(&dir));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    #[ignore = "rotates the real Claude Code OAuth tokens once"]
    fn smoke_refresh_real() {
        const SVC: &str = "Claude Code-credentials";
        let keys = |v: &Value| -> Vec<String> {
            v.as_object()
                .map(|o| o.keys().cloned().collect())
                .unwrap_or_default()
        };
        let before = read_blob(SVC).unwrap();
        let old_access = oauth_str(&before, "accessToken").to_string();
        let old_refresh = oauth_str(&before, "refreshToken").to_string();
        let got = refreshed_access(SVC, &old_access);
        let after = read_blob(SVC).unwrap();
        let o = &after["claudeAiOauth"];
        let hours = (o["expiresAt"].as_i64().unwrap_or(0) - chrono::Utc::now().timestamp_millis())
            as f64
            / 3.6e6;
        eprintln!("refresh ok={} msg={:?}", got.is_ok(), got.as_ref().err());
        eprintln!(
            "access changed={} refresh changed={} returned==stored={} expires in {hours:.2}h",
            oauth_str(&after, "accessToken") != old_access,
            oauth_str(&after, "refreshToken") != old_refresh,
            got.as_deref().ok() == Some(oauth_str(&after, "accessToken")),
        );
        eprintln!(
            "top keys same={} oauth keys same={} oauth keys={:?}",
            keys(&before) == keys(&after),
            keys(&before["claudeAiOauth"]) == keys(o),
            keys(o)
        );
        eprintln!(
            "refreshTokenExpiresAt={:?} scopes={} sub={:?} mcp entries {}→{}",
            o["refreshTokenExpiresAt"]
                .as_i64()
                .and_then(chrono::DateTime::from_timestamp_millis),
            o["scopes"],
            o["subscriptionType"].as_str(),
            keys(&before["mcpOAuth"]).len(),
            keys(&after["mcpOAuth"]).len(),
        );
        let q = crate::claude_limits::fetch();
        eprintln!(
            "fetch status={} weekly={:?}",
            q.status,
            q.weekly.map(|w| w.used_pct)
        );
        assert!(got.is_ok());
        assert_eq!(q.status, "ok");
    }

    #[test]
    #[ignore = "writes a throwaway item to the real login Keychain"]
    fn smoke_keychain_round_trip() {
        let service = "Token Counter-selftest";
        let mut big = blob();
        big["pad"] = Value::String("x".repeat(9000));
        let acct = account().unwrap();
        write_blob(service, &acct, &big).unwrap();
        assert_eq!(keychain_account(service).as_deref(), Some(acct.as_str()));
        assert_eq!(read_blob(service).unwrap(), big);
        big["claudeAiOauth"]["accessToken"] = "changed".into();
        write_blob(service, &acct, &big).unwrap();
        assert_eq!(
            read_blob(service).unwrap()["claudeAiOauth"]["accessToken"],
            "changed"
        );
        let small = blob();
        write_blob(service, &acct, &small).unwrap();
        assert_eq!(read_blob(service).unwrap(), small);
        let _ = std::process::Command::new("security")
            .args(["delete-generic-password", "-s", service])
            .output();
    }
}

// Cloud sync: this Mac ("primary") publishes an end-to-end encrypted snapshot
// of everything the popover shows; other Macs ("viewers") and the phone web
// app download it, decrypt it locally and render it with the same UI. The
// server (a few Supabase RPCs) only ever sees ciphertext.
//
// Pairing code = "tc1." + base64url(JSON {v:1, u: server url, k: publishable
// key, s: base64url(K)}), K = 32 random bytes. From K:
//   channel = hex(SHA256("tc-sync/channel/v1" ‖ K)[..16])   row id on the server
//   token   = base64url(SHA256("tc-sync/auth/v1" ‖ K))      write/read secret
//   encKey  = SHA256("tc-sync/enc/v1" ‖ K)                  AES-256-GCM key
// Envelope = base64(0x01 ‖ nonce(12) ‖ AES-GCM(encKey, nonce,
//   aad = "tc-sync/snapshot/v1:" + channel, gzip(snapshot JSON))).
// The TS side (src/sync/protocol.ts) implements the same contract; both are
// pinned by the same test vectors and by tests/fixtures/sync_envelope_vector.json.
//
// Never log or put into an error message: the code, K, the token, encKey, or
// the server URL (it comes from the code).

use crate::claude_limits::ClaudeQuota;
use crate::codex_limits::CodexLiveQuota;
use crate::usage::{self, Usage};
use aes_gcm::aead::{Aead, AeadCore, KeyInit, OsRng, Payload};
use aes_gcm::{Aes256Gcm, Nonce};
use base64::engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD};
use base64::Engine as _;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::{Condvar, Mutex, MutexGuard, OnceLock};
use std::time::Duration;
use tauri::Emitter;

const CODE_PREFIX: &str = "tc1.";
const ENVELOPE_VERSION: u8 = 1;
const NONCE_LEN: usize = 12;
#[cfg(test)]
const TAG_LEN: usize = 16;
const SNAPSHOT_SCHEMA: u32 = 1;
/// The server rejects payloads over 2,000,000 characters.
const MAX_PAYLOAD_CHARS: usize = 2_000_000;
const DEFAULT_PLAN: &str = "max5x";
const APP_VERSION: &str = env!("CARGO_PKG_VERSION");

/// Publisher cadence: a snapshot is built every minute, uploaded when its
/// content changed, and at least every 10 minutes as a heartbeat.
const TICK: Duration = Duration::from_secs(60);
/// First build after launch waits a little, so the popover's own warm-up scans
/// of all ranges go first.
const FIRST_TICK: Duration = Duration::from_secs(20);
const HEARTBEAT_SECS: i64 = 10 * 60;
const BACKOFF_START_SECS: u64 = 60;
const BACKOFF_MAX_SECS: u64 = 10 * 60;
const HTTP_TIMEOUT: Duration = Duration::from_secs(30);

// ---------------------------------------------------------------------------
// Pairing code and key derivation
// ---------------------------------------------------------------------------

/// The three values derived from K. Secrets: Debug is redacted.
pub struct Keys {
    pub channel: String,
    pub token: String,
    enc_key: [u8; 32],
}

/// A parsed pairing code: where the server is, and the derived keys.
pub struct Pairing {
    url: String,
    api_key: String,
    pub keys: Keys,
}

// Manual Debug impls so `unwrap_err()` and friends can never print a secret.
impl std::fmt::Debug for Keys {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Keys { .. }")
    }
}

impl std::fmt::Debug for Pairing {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Pairing { .. }")
    }
}

fn hash_with_label(label: &str, secret: &[u8; 32]) -> [u8; 32] {
    let mut h = Sha256::new();
    h.update(label.as_bytes());
    h.update(secret);
    h.finalize().into()
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// base64url without padding; a stray trailing "=" is tolerated.
fn b64url_decode(s: &str) -> Option<Vec<u8>> {
    URL_SAFE_NO_PAD.decode(s.trim_end_matches('=')).ok()
}

pub fn derive_keys(secret: &[u8; 32]) -> Keys {
    Keys {
        channel: hex(&hash_with_label("tc-sync/channel/v1", secret)[..16]),
        token: URL_SAFE_NO_PAD.encode(hash_with_label("tc-sync/auth/v1", secret)),
        enc_key: hash_with_label("tc-sync/enc/v1", secret),
    }
}

/// Parses and validates a pairing code. Error messages never echo the code.
pub fn parse_code(code: &str) -> Result<Pairing, String> {
    const NOT_A_CODE: &str = "That isn't a Token Counter pairing code.";
    let body = code
        .trim()
        .strip_prefix(CODE_PREFIX)
        .ok_or(NOT_A_CODE)?
        .trim();
    let json = b64url_decode(body).ok_or(NOT_A_CODE)?;
    let v: Value = serde_json::from_slice(&json).map_err(|_| NOT_A_CODE)?;
    if !v.is_object() {
        return Err(NOT_A_CODE.into());
    }
    if v.get("v").and_then(Value::as_u64) != Some(1) {
        return Err("This pairing code is from a different app version.".into());
    }
    let field = |name: &str| {
        v.get(name)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .ok_or_else(|| "This pairing code is incomplete.".to_string())
    };
    let url = field("u")?.trim_end_matches('/').to_string();
    let api_key = field("k")?.to_string();
    let secret: [u8; 32] = b64url_decode(field("s")?)
        .and_then(|b| b.try_into().ok())
        .ok_or("This pairing code is damaged.")?;
    Ok(Pairing {
        url,
        api_key,
        keys: derive_keys(&secret),
    })
}

/// Builds a code from its parts (test vectors and fixtures; the owner makes
/// real codes when registering a channel).
#[cfg(test)]
pub fn make_code(url: &str, api_key: &str, secret: &[u8; 32]) -> String {
    // Key order as JSON.stringify writes it (serde_json's map would sort).
    let s = |v: &str| Value::String(v.to_string()).to_string();
    let json = format!(
        r#"{{"v":1,"u":{},"k":{},"s":{}}}"#,
        s(url),
        s(api_key),
        s(&URL_SAFE_NO_PAD.encode(secret))
    );
    format!("{CODE_PREFIX}{}", URL_SAFE_NO_PAD.encode(json))
}

// ---------------------------------------------------------------------------
// Envelope
// ---------------------------------------------------------------------------

fn aad(channel: &str) -> String {
    format!("tc-sync/snapshot/v1:{channel}")
}

/// 0x01 ‖ nonce ‖ AES-256-GCM(plaintext), base64 with padding. The caller
/// supplies the nonce; it must never repeat for one key (see `seal`).
fn seal_raw(keys: &Keys, nonce: &[u8; NONCE_LEN], plaintext: &[u8]) -> Result<String, String> {
    let cipher = Aes256Gcm::new_from_slice(&keys.enc_key).map_err(|_| "bad key length")?;
    let aad = aad(&keys.channel);
    let sealed = cipher
        .encrypt(
            Nonce::from_slice(nonce),
            Payload {
                msg: plaintext,
                aad: aad.as_bytes(),
            },
        )
        .map_err(|_| "encryption failed")?;
    let mut out = Vec::with_capacity(1 + NONCE_LEN + sealed.len());
    out.push(ENVELOPE_VERSION);
    out.extend_from_slice(nonce);
    out.extend_from_slice(&sealed);
    Ok(STANDARD.encode(out))
}

fn gzip(bytes: &[u8]) -> Result<Vec<u8>, String> {
    let mut enc = flate2::write::GzEncoder::new(
        Vec::with_capacity(bytes.len() / 4),
        flate2::Compression::default(),
    );
    enc.write_all(bytes)
        .and_then(|_| enc.finish())
        .map_err(|e| format!("compression failed: {e}"))
}

/// gzip, then seal with a fresh random nonce.
pub fn seal(keys: &Keys, json: &[u8]) -> Result<String, String> {
    let nonce: [u8; NONCE_LEN] = Aes256Gcm::generate_nonce(&mut OsRng).into();
    seal_raw(keys, &nonce, &gzip(json)?)
}

#[cfg(test)]
#[derive(Debug, PartialEq)]
enum OpenError {
    NotBase64,
    BadVersion,
    TooShort,
    DecryptFailed,
    BadGzip,
}

#[cfg(test)]
fn open_raw(keys: &Keys, payload: &str) -> Result<Vec<u8>, OpenError> {
    let bytes = STANDARD
        .decode(payload.trim())
        .map_err(|_| OpenError::NotBase64)?;
    if bytes.len() < 1 + NONCE_LEN + TAG_LEN {
        return Err(OpenError::TooShort);
    }
    if bytes[0] != ENVELOPE_VERSION {
        return Err(OpenError::BadVersion);
    }
    let cipher = Aes256Gcm::new_from_slice(&keys.enc_key).map_err(|_| OpenError::DecryptFailed)?;
    let aad = aad(&keys.channel);
    cipher
        .decrypt(
            Nonce::from_slice(&bytes[1..1 + NONCE_LEN]),
            Payload {
                msg: &bytes[1 + NONCE_LEN..],
                aad: aad.as_bytes(),
            },
        )
        .map_err(|_| OpenError::DecryptFailed)
}

/// Decrypt and gunzip (tests and the live round trip; viewers are TS).
#[cfg(test)]
fn open(keys: &Keys, payload: &str) -> Result<Vec<u8>, OpenError> {
    use std::io::Read as _;
    let gz = open_raw(keys, payload)?;
    let mut out = Vec::new();
    flate2::read::GzDecoder::new(gz.as_slice())
        .take(64 << 20)
        .read_to_end(&mut out)
        .map_err(|_| OpenError::BadGzip)?;
    Ok(out)
}

// ---------------------------------------------------------------------------
// Snapshot
// ---------------------------------------------------------------------------

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct DeviceInfo {
    pub name: String,
    pub app_version: String,
}

/// `get_usage(range, plan)` for every range.
#[derive(Serialize, Clone)]
pub struct UsageByRange {
    pub session: Usage,
    pub today: Usage,
    pub week: Usage,
    pub month: Usage,
    pub all: Usage,
}

/// Schema 1: everything the popover shows, as the commands return it.
#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct Snapshot {
    pub schema: u32,
    pub generated_at: String,
    pub device: DeviceInfo,
    pub plan: String,
    pub usage: UsageByRange,
    pub claude_quota: Option<ClaudeQuota>,
    pub codex_quota: Option<CodexLiveQuota>,
}

fn now_iso() -> String {
    Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

/// The five ranges, each under COMPUTE_LOCK on its own so the popover's scans
/// queue between them instead of behind all five.
fn compute_ranges(plan: &str) -> UsageByRange {
    let one = |range: &str| {
        let _guard = crate::COMPUTE_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        usage::compute_usage_with_plan(range, plan)
    };
    UsageByRange {
        session: one("session"),
        today: one("today"),
        week: one("week"),
        month: one("month"),
        all: one("all"),
    }
}

/// The snapshot builder the publisher uses. Quotas are passed in (the
/// publisher reads them through the commands' cached paths before calling).
pub fn build_snapshot(
    device_name: &str,
    plan: &str,
    claude_quota: Option<ClaudeQuota>,
    codex_quota: Option<CodexLiveQuota>,
) -> Snapshot {
    let usage = compute_ranges(plan);
    Snapshot {
        schema: SNAPSHOT_SCHEMA,
        generated_at: now_iso(),
        device: DeviceInfo {
            name: device_name.to_string(),
            app_version: APP_VERSION.to_string(),
        },
        plan: plan.to_string(),
        usage,
        claude_quota,
        codex_quota,
    }
}

// Content hash: decides whether a snapshot is worth uploading. Hashes a
// canonical form of the JSON without the fields that change on every build or
// quota fetch although nothing the user sees changed:
// - `generatedAt`, `claudeQuota.fetchedAt`, `codexQuota.fetchedAt` (per spec);
// - every `resetsAt` of the two live quotas: Claude's carries sub-second
//   jitter that differs per fetch, and Codex reports an idle window's reset as
//   fetch time + window, so it moves every minute. Used percentages still move
//   the hash, and the 10-minute heartbeat bounds how stale a reset time gets;
// - object key order (`byModel` is a HashMap) and the last bits of floats
//   (totals are summed in HashMap order, so equal scans can differ by an ulp):
//   keys are sorted and floats written with 10 significant digits.
// Only the hash input is normalized; the uploaded JSON is untouched.
fn strip_volatile(v: &mut Value) {
    let Some(top) = v.as_object_mut() else {
        return;
    };
    top.remove("generatedAt");
    for quota in ["claudeQuota", "codexQuota"] {
        let Some(q) = top.get_mut(quota).and_then(Value::as_object_mut) else {
            continue;
        };
        q.remove("fetchedAt");
        for window in ["fiveHour", "weekly"] {
            if let Some(w) = q.get_mut(window).and_then(Value::as_object_mut) {
                w.remove("resetsAt");
            }
        }
    }
}

fn write_canonical(v: &Value, out: &mut String) {
    match v {
        Value::Object(map) => {
            let mut keys: Vec<&String> = map.keys().collect();
            keys.sort();
            out.push('{');
            for (i, k) in keys.into_iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                out.push_str(&Value::String(k.clone()).to_string());
                out.push(':');
                write_canonical(&map[k], out);
            }
            out.push('}');
        }
        Value::Array(items) => {
            out.push('[');
            for (i, item) in items.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                write_canonical(item, out);
            }
            out.push(']');
        }
        Value::Number(n) if n.is_f64() => {
            out.push_str(&format!("{:.9e}", n.as_f64().unwrap_or(0.0)));
        }
        other => out.push_str(&other.to_string()),
    }
}

fn content_hash(snapshot: &Snapshot) -> [u8; 32] {
    let mut v = serde_json::to_value(snapshot).unwrap_or(Value::Null);
    strip_volatile(&mut v);
    let mut canonical = String::new();
    write_canonical(&v, &mut canonical);
    Sha256::digest(canonical.as_bytes()).into()
}

// ---------------------------------------------------------------------------
// Config (sync.json in the app-data dir, 0600)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Role {
    Off,
    Primary,
    Viewer,
}

impl Role {
    fn parse(s: Option<&str>) -> Option<Self> {
        match s.map(str::trim) {
            None | Some("") | Some("off") => Some(Role::Off),
            Some("primary") => Some(Role::Primary),
            Some("viewer") => Some(Role::Viewer),
            Some(_) => None,
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Role::Off => "off",
            Role::Primary => "primary",
            Role::Viewer => "viewer",
        }
    }
}

/// sync.json. Every field optional; unknown fields are ignored. Holds the
/// pairing code, hence 0600 and a redacted Debug.
#[derive(Serialize, Deserialize, Default, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
struct SyncConfig {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    role: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    code: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    device_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    plan: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    web_url: Option<String>,
}

// Redacted: the code never reaches a log or an assert message.
impl std::fmt::Debug for SyncConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SyncConfig")
            .field("role", &self.role)
            .field("code", &self.code.as_ref().map(|_| "<redacted>"))
            .field("device_name", &self.device_name)
            .field("plan", &self.plan)
            .field("web_url", &self.web_url)
            .finish()
    }
}

impl SyncConfig {
    /// An unknown role in a hand-edited file counts as off.
    fn role(&self) -> Role {
        Role::parse(self.role.as_deref()).unwrap_or(Role::Off)
    }

    fn pairing(&self) -> Option<Result<Pairing, String>> {
        self.code.as_deref().map(parse_code)
    }

    fn device_name(&self) -> String {
        self.device_name
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(String::from)
            .unwrap_or_else(|| default_device_name().to_string())
    }

    fn plan(&self) -> String {
        self.plan
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .unwrap_or(DEFAULT_PLAN)
            .to_string()
    }
}

fn config_path() -> Option<PathBuf> {
    dirs::data_dir().map(|d| d.join("claude-code-token-counter").join("sync.json"))
}

fn load_config_at(path: &Path) -> SyncConfig {
    std::fs::read(path)
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default()
}

fn load_config() -> SyncConfig {
    config_path()
        .map(|p| load_config_at(&p))
        .unwrap_or_default()
}

// Serializes read-modify-write of sync.json (and its tmp file name).
static CONFIG_LOCK: Mutex<()> = Mutex::new(());

/// tmp + rename, the tmp created 0600 before anything is written to it, so
/// the code is never readable by others, not even for a moment.
fn save_config_at(path: &Path, config: &SyncConfig) -> Result<(), String> {
    let fail = |e: std::io::Error| format!("couldn't save the sync settings: {e}");
    let dir = path.parent().ok_or("couldn't save the sync settings")?;
    std::fs::create_dir_all(dir).map_err(fail)?;
    let json = serde_json::to_vec_pretty(config).map_err(|e| e.to_string())?;
    let tmp = path.with_extension(format!("json.tmp-{}", std::process::id()));
    let _ = std::fs::remove_file(&tmp);
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let written = options.open(&tmp).and_then(|mut f| {
        f.write_all(&json)?;
        f.sync_all()
    });
    if let Err(e) = written.and_then(|_| std::fs::rename(&tmp, path)) {
        let _ = std::fs::remove_file(&tmp);
        return Err(fail(e));
    }
    Ok(())
}

/// "Alice's Mac Studio" → "Mac Studio". macOS writes the curly apostrophe.
fn strip_possessive(name: &str) -> String {
    let name = name.trim();
    let cut = ["'s ", "\u{2019}s "]
        .iter()
        .filter_map(|sep| name.find(sep).map(|i| i + sep.len()))
        .min();
    match cut.map(|i| name[i..].trim()) {
        Some(rest) if !rest.is_empty() => rest.to_string(),
        _ => name.to_string(),
    }
}

fn default_device_name() -> &'static str {
    static NAME: OnceLock<String> = OnceLock::new();
    NAME.get_or_init(|| {
        computer_name()
            .map(|n| strip_possessive(&n))
            .filter(|n| !n.is_empty())
            .unwrap_or_else(|| "Mac".to_string())
    })
}

#[cfg(target_os = "macos")]
fn computer_name() -> Option<String> {
    let out = std::process::Command::new("/usr/sbin/scutil")
        .args(["--get", "ComputerName"])
        .output()
        .ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).trim().to_string())
}

#[cfg(not(target_os = "macos"))]
fn computer_name() -> Option<String> {
    None
}

// ---------------------------------------------------------------------------
// Wire types for the commands
// ---------------------------------------------------------------------------

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct SyncConfigView {
    role: String,
    code: Option<String>,
    device_name: String,
    plan: String,
    web_url: Option<String>,
}

#[derive(Serialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SyncStatus {
    role: String,
    paired: bool,
    device_name: String,
    last_upload_at: Option<String>,
    last_version: Option<i64>,
    last_error: Option<String>,
    last_error_at: Option<String>,
    uploading: bool,
}

fn config_view(config: &SyncConfig) -> SyncConfigView {
    SyncConfigView {
        role: config.role().as_str().to_string(),
        code: config.code.clone(),
        device_name: config.device_name(),
        plan: config.plan(),
        web_url: config.web_url.clone(),
    }
}

// ---------------------------------------------------------------------------
// Publisher state
// ---------------------------------------------------------------------------

/// What the publisher knows about its uploads to `channel`. In memory only: a
/// restart uploads once on the first build.
struct PublishState {
    channel: Option<String>,
    last_upload_at: Option<DateTime<Utc>>,
    last_version: Option<i64>,
    last_hash: Option<[u8; 32]>,
    last_error: Option<String>,
    last_error_at: Option<DateTime<Utc>>,
    failures: u32,
    next_attempt_at: Option<DateTime<Utc>>,
    uploading: bool,
}

impl PublishState {
    const fn new() -> Self {
        Self {
            channel: None,
            last_upload_at: None,
            last_version: None,
            last_hash: None,
            last_error: None,
            last_error_at: None,
            failures: 0,
            next_attempt_at: None,
            uploading: false,
        }
    }

    /// Forget everything (new code, new role); an upload in flight stays shown.
    fn reset(&mut self) {
        *self = Self {
            uploading: self.uploading,
            ..Self::new()
        };
    }

    /// Start tracking `channel`; a different one than before starts fresh, so
    /// a newly pasted code uploads at once instead of after the heartbeat.
    fn adopt_channel(&mut self, channel: &str) {
        if self.channel.as_deref() != Some(channel) {
            self.reset();
            self.channel = Some(channel.to_string());
        }
    }

    fn in_backoff(&self, now: DateTime<Utc>) -> bool {
        self.next_attempt_at.is_some_and(|t| now < t)
    }

    fn upload_due(&self, hash: &[u8; 32], now: DateTime<Utc>) -> bool {
        self.last_hash.as_ref() != Some(hash)
            || self
                .last_upload_at
                .is_none_or(|t| (now - t).num_seconds() >= HEARTBEAT_SECS)
    }

    fn record_success(&mut self, channel: &str, hash: [u8; 32], version: i64, at: DateTime<Utc>) {
        // A code changed mid-upload: these figures belong to the old channel.
        if self.channel.as_deref() != Some(channel) {
            return;
        }
        self.last_upload_at = Some(at);
        self.last_version = Some(version);
        self.last_hash = Some(hash);
        self.last_error = None;
        self.last_error_at = None;
        self.failures = 0;
        self.next_attempt_at = None;
    }

    fn record_failure(&mut self, channel: &str, error: &str, at: DateTime<Utc>) {
        if self.channel.as_deref() != Some(channel) {
            return;
        }
        self.failures = self.failures.saturating_add(1);
        self.last_error = Some(error.to_string());
        self.last_error_at = Some(at);
        self.next_attempt_at = Some(
            at + chrono::Duration::from_std(next_delay(self.failures))
                .unwrap_or(chrono::Duration::zero()),
        );
    }
}

/// 60 s, 2, 4, 8 min, then 10 min for every later consecutive failure.
fn next_delay(failures: u32) -> Duration {
    let shift = failures.saturating_sub(1).min(16);
    Duration::from_secs((BACKOFF_START_SECS << shift).min(BACKOFF_MAX_SECS))
}

static STATE: Mutex<PublishState> = Mutex::new(PublishState::new());
// One publish at a time (the 60 s thread and "Sync now").
static PUBLISH_LOCK: Mutex<()> = Mutex::new(());
// Wakes the publisher thread early for a forced publish.
static WAKE: Mutex<bool> = Mutex::new(false);
static WAKE_CV: Condvar = Condvar::new();
static APP: OnceLock<tauri::AppHandle> = OnceLock::new();

fn state() -> MutexGuard<'static, PublishState> {
    STATE.lock().unwrap_or_else(|e| e.into_inner())
}

fn iso(t: Option<DateTime<Utc>>) -> Option<String> {
    t.map(|t| t.to_rfc3339_opts(chrono::SecondsFormat::Secs, true))
}

fn status_of(config: &SyncConfig, st: &PublishState) -> SyncStatus {
    let role = config.role();
    SyncStatus {
        role: role.as_str().to_string(),
        paired: role != Role::Off && config.pairing().is_some_and(|p| p.is_ok()),
        device_name: config.device_name(),
        last_upload_at: iso(st.last_upload_at),
        last_version: st.last_version,
        last_error: st.last_error.clone(),
        last_error_at: iso(st.last_error_at),
        uploading: st.uploading,
    }
}

fn current_status(config: &SyncConfig) -> SyncStatus {
    status_of(config, &state())
}

fn emit_status(status: &SyncStatus) {
    if let Some(app) = APP.get() {
        let _ = app.emit("sync-status", status.clone());
    }
}

fn wake_publisher() {
    *WAKE.lock().unwrap_or_else(|e| e.into_inner()) = true;
    WAKE_CV.notify_all();
}

/// Sleeps up to `timeout`; true when woken for a forced publish.
fn wait_for_wake(timeout: Duration) -> bool {
    let pending = WAKE.lock().unwrap_or_else(|e| e.into_inner());
    let (mut pending, _) = WAKE_CV
        .wait_timeout_while(pending, timeout, |p| !*p)
        .unwrap_or_else(|e| e.into_inner());
    std::mem::take(&mut *pending)
}

// ---------------------------------------------------------------------------
// Server RPCs
// ---------------------------------------------------------------------------

#[derive(Debug)]
enum RpcError {
    /// 401/403: wrong token or the channel isn't registered.
    Forbidden,
    Network(&'static str),
    Http(u16, String),
    BadResponse,
    Insecure,
}

impl std::fmt::Display for RpcError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RpcError::Forbidden => f.write_str("This pairing code isn't registered on the server."),
            RpcError::Network(what) => write!(f, "Couldn't reach the sync server ({what})."),
            RpcError::Http(status, msg) if msg.is_empty() => {
                write!(f, "The sync server answered HTTP {status}.")
            }
            RpcError::Http(status, msg) => {
                write!(f, "The sync server answered HTTP {status}: {msg}")
            }
            RpcError::BadResponse => f.write_str("The sync server sent an unexpected reply."),
            RpcError::Insecure => {
                f.write_str("This pairing code's server address doesn't use https.")
            }
        }
    }
}

/// reqwest's error text carries the URL (from the code), so only the kind of
/// failure is kept.
fn network_error(e: &reqwest::Error) -> RpcError {
    RpcError::Network(if e.is_timeout() {
        "timed out"
    } else if e.is_connect() {
        "no connection"
    } else {
        "request failed"
    })
}

/// The token travels in the body, so only https leaves the machine (plain
/// http only to a local test server).
fn is_allowed_server(url: &str) -> bool {
    if url.starts_with("https://") {
        return true;
    }
    let host = url
        .strip_prefix("http://")
        .and_then(|rest| rest.split(['/', ':']).next());
    matches!(host, Some("localhost" | "127.0.0.1"))
}

/// One shared client (connection reuse; it lives for the whole session, so
/// it is never dropped inside the async runtime).
fn http_client() -> Result<&'static reqwest::blocking::Client, RpcError> {
    static CLIENT: OnceLock<reqwest::blocking::Client> = OnceLock::new();
    if let Some(c) = CLIENT.get() {
        return Ok(c);
    }
    let client = reqwest::blocking::Client::builder()
        .timeout(HTTP_TIMEOUT)
        .user_agent(concat!("token-counter/", env!("CARGO_PKG_VERSION")))
        .build()
        .map_err(|e| network_error(&e))?;
    Ok(CLIENT.get_or_init(|| client))
}

fn rpc(pairing: &Pairing, name: &str, body: &Value) -> Result<Value, RpcError> {
    let url = &pairing.url;
    if !is_allowed_server(url) {
        return Err(RpcError::Insecure);
    }
    let response = http_client()?
        .post(format!("{url}/rest/v1/rpc/{name}"))
        .header("apikey", &pairing.api_key)
        .header("Content-Type", "application/json")
        .body(body.to_string())
        .send()
        .map_err(|e| network_error(&e))?;
    let status = response.status().as_u16();
    let text = response.text().map_err(|e| network_error(&e))?;
    match status {
        200..=299 => serde_json::from_str(&text).map_err(|_| RpcError::BadResponse),
        401 | 403 => Err(RpcError::Forbidden),
        _ => {
            // PostgREST errors are {"code","message",...}; keep a short message.
            let msg = serde_json::from_str::<Value>(&text)
                .ok()
                .and_then(|v| v.get("message").and_then(Value::as_str).map(String::from))
                .unwrap_or_default();
            Err(RpcError::Http(status, msg.chars().take(160).collect()))
        }
    }
}

fn auth_body(keys: &Keys) -> Value {
    serde_json::json!({"p_channel": keys.channel, "p_token": keys.token})
}

/// `tc_head`: proves the code is registered (and how far uploads got).
fn tc_head(pairing: &Pairing) -> Result<Value, RpcError> {
    let head = rpc(pairing, "tc_head", &auth_body(&pairing.keys))?;
    if head.is_object() {
        Ok(head)
    } else {
        Err(RpcError::BadResponse)
    }
}

/// `tc_put`: stores the envelope, returns the new version number.
fn tc_put(pairing: &Pairing, payload: &str, generated_at: &str) -> Result<i64, RpcError> {
    let body = serde_json::json!({
        "p_channel": pairing.keys.channel,
        "p_token": pairing.keys.token,
        "p_payload": payload,
        "p_device": Value::Null,
        "p_generated_at": generated_at,
    });
    rpc(pairing, "tc_put", &body)?
        .as_i64()
        .ok_or(RpcError::BadResponse)
}

/// `tc_get`: head fields + "payload" (tests; viewers are TS).
#[cfg(test)]
fn tc_get(pairing: &Pairing) -> Result<Value, RpcError> {
    rpc(pairing, "tc_get", &auth_body(&pairing.keys))
}

// ---------------------------------------------------------------------------
// Publishing
// ---------------------------------------------------------------------------

/// Serialize, encrypt and upload; the version on success.
fn upload(pairing: &Pairing, snapshot: &Snapshot) -> Result<i64, String> {
    let json = serde_json::to_vec(snapshot).map_err(|e| format!("couldn't encode: {e}"))?;
    let payload = seal(&pairing.keys, &json)?;
    if payload.len() > MAX_PAYLOAD_CHARS {
        return Err(format!(
            "The snapshot is too large to upload ({} characters).",
            payload.len()
        ));
    }
    tc_put(pairing, &payload, &snapshot.generated_at).map_err(|e| e.to_string())
}

/// The snapshot the publisher uploads: quotas first, through the commands'
/// cached paths (at most one fetch a minute per provider, shared with the
/// popover; a fresh Claude read also refreshes the weekly reset the Week range
/// starts at), then the five ranges.
fn live_snapshot(config: &SyncConfig) -> Snapshot {
    let claude = crate::claude_quota(false);
    let codex = crate::codex_quota(false);
    build_snapshot(
        &config.device_name(),
        &config.plan(),
        Some(claude),
        Some(codex),
    )
}

/// One publish attempt. `forced` (Sync now, a new code) skips the backoff and
/// the content check. Ok(status) also when nothing needed uploading; Err only
/// when this Mac isn't a configured primary or the upload failed.
fn publish_cycle(forced: bool) -> Result<SyncStatus, String> {
    publish_cycle_with(forced, load_config, live_snapshot)
}

/// `publish_cycle` with the config and snapshot sources passed in (the live
/// test feeds a temp config and synthetic snapshots).
fn publish_cycle_with(
    forced: bool,
    load: impl FnOnce() -> SyncConfig,
    make_snapshot: impl FnOnce(&SyncConfig) -> Snapshot,
) -> Result<SyncStatus, String> {
    let _publishing = PUBLISH_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let config = load();
    if config.role() != Role::Primary {
        return Err("This Mac isn't set to upload.".into());
    }
    let pairing = match config.pairing() {
        Some(p) => p?,
        None => return Err("No pairing code is set.".into()),
    };
    let channel = pairing.keys.channel.clone();
    {
        let mut st = state();
        st.adopt_channel(&channel);
        if !forced && st.in_backoff(Utc::now()) {
            return Ok(status_of(&config, &st));
        }
    }

    let snapshot = make_snapshot(&config);
    let hash = content_hash(&snapshot);

    let started = {
        let mut st = state();
        if !forced && !st.upload_due(&hash, Utc::now()) {
            return Ok(status_of(&config, &st));
        }
        st.uploading = true;
        status_of(&config, &st)
    };
    emit_status(&started);

    let result = upload(&pairing, &snapshot);
    let status = {
        let mut st = state();
        st.uploading = false;
        match &result {
            Ok(version) => st.record_success(&channel, hash, *version, Utc::now()),
            Err(e) => st.record_failure(&channel, e, Utc::now()),
        }
        status_of(&config, &st)
    };
    emit_status(&status);
    match result {
        Ok(_) => Ok(status),
        Err(e) => {
            eprintln!("sync upload failed: {e}");
            Err(e)
        }
    }
}

/// Started in `setup`. Re-reads sync.json every cycle, so changes made in the
/// UI apply without a restart; does nothing unless this Mac is the primary.
pub fn spawn_publisher(app: tauri::AppHandle) {
    let _ = APP.set(app);
    let spawned = std::thread::Builder::new()
        .name("sync-publisher".into())
        .spawn(|| {
            // Resolve the scutil name here, not first under the status lock.
            default_device_name();
            let mut wait = FIRST_TICK;
            loop {
                let forced = wait_for_wake(wait);
                wait = TICK;
                // A panic in one cycle must not end publishing for the session.
                let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    publish_cycle(forced)
                }));
                if outcome.is_err() {
                    state().uploading = false;
                    eprintln!("sync publish cycle panicked");
                }
            }
        });
    if let Err(e) = spawned {
        eprintln!("sync publisher failed to start: {e}");
    }
}

// ---------------------------------------------------------------------------
// Settings changes
// ---------------------------------------------------------------------------

/// A `sync_set_config` call. For `code`, `device_name` and `web_url`: None
/// keeps the stored value, an empty string clears it.
struct ConfigChange {
    role: String,
    code: Option<String>,
    device_name: Option<String>,
    web_url: Option<String>,
}

fn merge_field(stored: &Option<String>, change: Option<String>) -> Option<String> {
    match change {
        None => stored.clone(),
        Some(v) => Some(v.trim().to_string()).filter(|v| !v.is_empty()),
    }
}

/// Validates and persists a change; `check` is the server round trip (one
/// `tc_head`), run for every role but off. Nothing is written when it fails.
/// Returns the saved config and whether the publisher state must start over.
fn apply_config_change(
    path: &Path,
    change: ConfigChange,
    check: impl FnOnce(&Pairing) -> Result<(), String>,
) -> Result<(SyncConfig, bool), String> {
    let role = Role::parse(Some(&change.role))
        .ok_or_else(|| format!("Unknown sync role \"{}\".", change.role.trim()))?;
    let _lock = CONFIG_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let stored = load_config_at(path);
    let mut next = stored.clone();
    next.role = Some(role.as_str().to_string());
    // A code pasted across lines: base64url has no whitespace, drop it all.
    let code = change
        .code
        .map(|c| c.split_whitespace().collect::<String>());
    next.code = merge_field(&stored.code, code);
    next.device_name =
        merge_field(&stored.device_name, change.device_name).map(|n| n.chars().take(64).collect());
    next.web_url = merge_field(&stored.web_url, change.web_url);
    if let Some(url) = &next.web_url {
        if !url.starts_with("https://") && !url.starts_with("http://") {
            return Err("The web app address must start with https://".into());
        }
    }
    if role != Role::Off {
        let pairing = next.pairing().ok_or("Paste a pairing code first.")??;
        check(&pairing)?;
    }
    save_config_at(path, &next)?;
    let restart = stored.role() != role || stored.code != next.code;
    Ok((next, restart))
}

fn set_config_blocking(change: ConfigChange) -> Result<SyncStatus, String> {
    let path = config_path().ok_or("No app-data folder to save the sync settings in.")?;
    let (config, restart) = apply_config_change(&path, change, |pairing| {
        tc_head(pairing).map(|_| ()).map_err(|e| e.to_string())
    })?;
    if restart {
        state().reset();
    }
    let status = current_status(&config);
    if config.role() == Role::Primary {
        wake_publisher();
    }
    emit_status(&status);
    Ok(status)
}

// ---------------------------------------------------------------------------
// Commands (all off the main thread: they touch disk, scutil or the network)
// ---------------------------------------------------------------------------

#[tauri::command]
pub async fn sync_get_config() -> SyncConfigView {
    tauri::async_runtime::spawn_blocking(|| config_view(&load_config()))
        .await
        .unwrap_or_else(|_| config_view(&SyncConfig::default()))
}

#[tauri::command]
pub async fn sync_set_config(
    role: String,
    code: Option<String>,
    device_name: Option<String>,
    web_url: Option<String>,
) -> Result<SyncStatus, String> {
    let change = ConfigChange {
        role,
        code,
        device_name,
        web_url,
    };
    tauri::async_runtime::spawn_blocking(move || set_config_blocking(change))
        .await
        .map_err(|e| format!("sync task failed: {e}"))?
}

#[tauri::command]
pub async fn sync_status() -> SyncStatus {
    tauri::async_runtime::spawn_blocking(|| current_status(&load_config()))
        .await
        .unwrap_or_else(|_| current_status(&SyncConfig::default()))
}

#[tauri::command]
pub async fn sync_publish_now() -> Result<SyncStatus, String> {
    tauri::async_runtime::spawn_blocking(|| publish_cycle(true))
        .await
        .map_err(|e| format!("sync task failed: {e}"))?
}

#[cfg(test)]
mod tests {
    use super::*;

    const VEC_CODE: &str = "tc1.eyJ2IjoxLCJ1IjoiaHR0cHM6Ly9leGFtcGxlLnN1cGFiYXNlLmNvIiwiayI6InNiX3B1Ymxpc2hhYmxlX3Rlc3QiLCJzIjoiQUFFQ0F3UUZCZ2NJQ1FvTERBME9EeEFSRWhNVUZSWVhHQmthR3h3ZEhoOCJ9";
    const VEC_CHANNEL: &str = "0e8a2bff0cb4c93cd3e39875d39afdf0";
    const VEC_TOKEN: &str = "sXBEM3Hi3FGry4uvVVu9X3DYi_HsyBYyg2pXQyVye5s";
    const VEC_ENC_KEY: &str = "18653618e6a8509563c53d8bfd2a0b3e715f14002cc1dce14fd22e781009669a";
    const VEC_NONCE: [u8; 12] = [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11];
    const VEC_CT_TAG: &str = "f6bb5cb0871ede9f3d5f5de7245bcc101a85458c4f";
    const VEC_ENVELOPE: &str = "AQABAgMEBQYHCAkKC/a7XLCHHt6fPV9d5yRbzBAahUWMTw==";

    fn vec_secret() -> [u8; 32] {
        std::array::from_fn(|i| i as u8)
    }

    fn b64u_json(v: &Value) -> String {
        format!("{CODE_PREFIX}{}", URL_SAFE_NO_PAD.encode(v.to_string()))
    }

    fn fixture_path() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests")
            .join("fixtures")
            .join("sync_envelope_vector.json")
    }

    /// A fresh, empty directory under the system temp dir.
    fn temp_dir(tag: &str) -> PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir =
            std::env::temp_dir().join(format!("tc-sync-test-{tag}-{}-{nanos}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn cleanup(dir: &Path) {
        if let Ok(entries) = std::fs::read_dir(dir) {
            for e in entries.flatten() {
                let _ = std::fs::remove_file(e.path());
            }
        }
        let _ = std::fs::remove_dir(dir);
    }

    // -- §6 vectors ---------------------------------------------------------

    #[test]
    fn vector_code_parses_and_derives() {
        let p = parse_code(VEC_CODE).unwrap();
        assert_eq!(p.url, "https://example.supabase.co");
        assert_eq!(p.api_key, "sb_publishable_test");
        assert_eq!(p.keys.channel, VEC_CHANNEL);
        assert_eq!(p.keys.token, VEC_TOKEN);
        assert_eq!(hex(&p.keys.enc_key), VEC_ENC_KEY);
        // The same code rebuilt from its parts, and derive_keys on raw K.
        assert_eq!(
            make_code(
                "https://example.supabase.co",
                "sb_publishable_test",
                &vec_secret()
            ),
            VEC_CODE
        );
        let k = derive_keys(&vec_secret());
        assert_eq!(
            (k.channel.as_str(), k.token.as_str()),
            (VEC_CHANNEL, VEC_TOKEN)
        );
    }

    #[test]
    fn aes_gcm_known_answer() {
        let keys = derive_keys(&vec_secret());
        assert_eq!(
            aad(&keys.channel),
            format!("tc-sync/snapshot/v1:{VEC_CHANNEL}")
        );
        let envelope = seal_raw(&keys, &VEC_NONCE, b"hello").unwrap();
        assert_eq!(envelope, VEC_ENVELOPE);
        let raw = STANDARD.decode(&envelope).unwrap();
        assert_eq!(hex(&raw[1 + NONCE_LEN..]), VEC_CT_TAG);
        assert_eq!(open_raw(&keys, VEC_ENVELOPE).unwrap(), b"hello");
    }

    #[test]
    fn code_parsing_trims_and_ignores_unknown_fields() {
        let s = URL_SAFE_NO_PAD.encode(vec_secret());
        let p = parse_code(&format!("  \n{VEC_CODE}\t ")).unwrap();
        assert_eq!(p.keys.channel, VEC_CHANNEL);
        let extra = b64u_json(&serde_json::json!({
            "v": 1, "u": "https://example.supabase.co/", "k": "sb_publishable_test",
            "s": s, "label": "Mac Studio", "x": [1, 2]
        }));
        let p = parse_code(&extra).unwrap();
        assert_eq!(p.keys.token, VEC_TOKEN);
        assert_eq!(
            p.url, "https://example.supabase.co",
            "trailing slash dropped"
        );
    }

    #[test]
    fn code_parsing_rejects_bad_codes() {
        let s = URL_SAFE_NO_PAD.encode(vec_secret());
        let short = URL_SAFE_NO_PAD.encode(&vec_secret()[..31]);
        let long = URL_SAFE_NO_PAD.encode([7u8; 33]);
        let bad = [
            String::new(),
            "tc1.".into(),
            VEC_CODE.replacen("tc1.", "tc2.", 1),
            VEC_CODE.replacen("tc1.", "", 1),
            "tc1.!!!not-base64!!!".into(),
            format!("tc1.{}", URL_SAFE_NO_PAD.encode("not json")),
            format!("tc1.{}", URL_SAFE_NO_PAD.encode("[1,2]")),
            b64u_json(&serde_json::json!({"v": 2, "u": "https://a", "k": "k", "s": s})),
            b64u_json(&serde_json::json!({"v": "1", "u": "https://a", "k": "k", "s": s})),
            b64u_json(&serde_json::json!({"u": "https://a", "k": "k", "s": s})),
            b64u_json(&serde_json::json!({"v": 1, "k": "k", "s": s})),
            b64u_json(&serde_json::json!({"v": 1, "u": "https://a", "s": s})),
            b64u_json(&serde_json::json!({"v": 1, "u": "https://a", "k": "", "s": s})),
            b64u_json(&serde_json::json!({"v": 1, "u": "https://a", "k": "k"})),
            b64u_json(&serde_json::json!({"v": 1, "u": "https://a", "k": "k", "s": short})),
            b64u_json(&serde_json::json!({"v": 1, "u": "https://a", "k": "k", "s": long})),
            b64u_json(&serde_json::json!({"v": 1, "u": "https://a", "k": "k", "s": "@@"})),
        ];
        for code in &bad {
            let err = parse_code(code).unwrap_err();
            assert!(!err.contains(&s), "error must not echo the secret");
        }
    }

    // -- envelope -------------------------------------------------------------

    #[test]
    fn envelope_round_trip_uses_fresh_nonces() {
        let keys = derive_keys(&vec_secret());
        let json = serde_json::to_vec(&sample_snapshot()).unwrap();
        let a = seal(&keys, &json).unwrap();
        let b = seal(&keys, &json).unwrap();
        assert_ne!(a, b, "a fresh nonce per seal");
        assert_eq!(open(&keys, &a).unwrap(), json);
        assert_eq!(open(&keys, &b).unwrap(), json);
        // The plaintext inside is gzip (RFC 1952 magic).
        assert_eq!(&open_raw(&keys, &a).unwrap()[..2], &[0x1f, 0x8b]);
    }

    #[test]
    fn envelope_rejects_bad_input() {
        let keys = derive_keys(&vec_secret());
        let good = seal(&keys, b"{\"schema\":1}").unwrap();
        let mut raw = STANDARD.decode(&good).unwrap();

        let mut wrong_version = raw.clone();
        wrong_version[0] = 2;
        assert_eq!(
            open(&keys, &STANDARD.encode(&wrong_version)),
            Err(OpenError::BadVersion)
        );
        assert_eq!(
            open(&keys, &STANDARD.encode(&raw[..1 + NONCE_LEN + TAG_LEN - 1])),
            Err(OpenError::TooShort)
        );
        assert_eq!(open(&keys, ""), Err(OpenError::TooShort));
        assert_eq!(open(&keys, "not base64 at all!"), Err(OpenError::NotBase64));

        let other = derive_keys(&[9u8; 32]);
        assert_eq!(open(&other, &good), Err(OpenError::DecryptFailed));
        // Same key, other channel: the AAD binds the envelope to its channel.
        let moved = Keys {
            channel: other.channel.clone(),
            token: String::new(),
            enc_key: keys.enc_key,
        };
        assert_eq!(open(&moved, &good), Err(OpenError::DecryptFailed));

        let last = raw.len() - 1;
        raw[last] ^= 1;
        assert_eq!(
            open(&keys, &STANDARD.encode(&raw)),
            Err(OpenError::DecryptFailed)
        );

        // Authentic but not gzip.
        let plain = seal_raw(&keys, &VEC_NONCE, b"hello").unwrap();
        assert_eq!(open(&keys, &plain), Err(OpenError::BadGzip));
    }

    // -- snapshot + fixture ---------------------------------------------------

    fn model(input: f64, out: f64, cw: f64, cr: f64, cost: f64) -> usage::ModelUsage {
        let mut m = usage::ModelUsage::default();
        m.input = input;
        m.out = out;
        m.cache_write = cw;
        m.cache_read = cr;
        m.cost = Some(cost);
        m.price_source = Some(usage::PriceSource::Litellm);
        m
    }

    fn sample_usage(range: &str, scale: f64) -> Usage {
        let mut by_model = std::collections::HashMap::new();
        by_model.insert(
            "claude-opus-5-5".to_string(),
            model(
                1250.5 * scale,
                310.25 * scale,
                8200.0 * scale,
                98000.0 * scale,
                87.42 * scale,
            ),
        );
        by_model.insert(
            "claude-fable-5-1".to_string(),
            model(
                220.0 * scale,
                95.5 * scale,
                1500.0 * scale,
                21000.0 * scale,
                31.07 * scale,
            ),
        );
        by_model.insert(
            "gpt-6-astra".to_string(),
            model(
                640.0 * scale,
                12.75 * scale,
                0.0,
                5100.0 * scale,
                12.3 * scale,
            ),
        );
        // Summed in a fixed order: HashMap order would change the last bits.
        let mut models: Vec<&usage::ModelUsage> = Vec::new();
        for id in ["claude-fable-5-1", "claude-opus-5-5", "gpt-6-astra"] {
            models.push(&by_model[id]);
        }
        let total_cost = models.iter().filter_map(|m| m.cost).sum();
        let total_tokens = models
            .iter()
            .map(|m| m.input + m.out + m.cache_write + m.cache_read)
            .sum();
        Usage {
            by_model,
            by_tool: vec![
                usage::ToolUsage {
                    name: "Bash".into(),
                    calls: (412.0 * scale) as u64,
                    tokens: 0,
                },
                usage::ToolUsage {
                    name: "Read".into(),
                    calls: (288.0 * scale) as u64,
                    tokens: 0,
                },
                usage::ToolUsage {
                    name: "Edit".into(),
                    calls: (97.0 * scale) as u64,
                    tokens: 0,
                },
            ],
            sparkline: (0..12).map(|i| (i % 5) as f64 * 1.5 * scale).collect(),
            sparkline_codex: (0..12).map(|i| (i % 3) as f64 * 0.25 * scale).collect(),
            codex_quota: None,
            spark_left: "12h ago".into(),
            spark_right: "now".into(),
            range_label: match range {
                "session" => "Current session",
                "today" => "Today",
                "week" => "This week",
                "month" => "This month",
                _ => "All-time",
            }
            .into(),
            total_cost,
            total_cost_est: 1.25 * scale,
            total_tokens,
            delta_pct: Some(12.5),
            credit_used_5h: 18_500_000.0,
            credit_used_week: 240_000_000.0,
            credit_limit_5h: 60_000_000.0,
            credit_limit_week: 756_000_000.0,
            plan_label: "Max 5x".into(),
            plan_id: "max5x".into(),
            session_start: Some("2026-11-09T06:12:00+00:00".into()),
            tokens_5h: 41_000_000.0,
            tokens_week: 520_000_000.0,
            priority_events_5h: 0,
            active_block_start: Some("2026-11-09T05:40:00+00:00".into()),
            token_limit_5h: 350_000_000.0,
            price_health: None,
        }
    }

    /// Synthetic, real-shaped snapshot (the fixture's content; the repo is
    /// public, so no real numbers).
    fn sample_snapshot() -> Snapshot {
        use crate::claude_limits::LimitWindow;
        Snapshot {
            schema: SNAPSHOT_SCHEMA,
            generated_at: "2026-11-09T08:00:00Z".into(),
            device: DeviceInfo {
                name: "Mac Studio".into(),
                app_version: "0.2.1".into(),
            },
            plan: "max5x".into(),
            usage: UsageByRange {
                session: sample_usage("session", 0.05),
                today: sample_usage("today", 0.2),
                week: sample_usage("week", 1.0),
                month: sample_usage("month", 3.5),
                all: sample_usage("all", 9.0),
            },
            claude_quota: Some(ClaudeQuota {
                five_hour: Some(LimitWindow {
                    used_pct: 25.0,
                    resets_at: Some("2026-11-09T11:50:00.612317+00:00".into()),
                }),
                weekly: Some(LimitWindow {
                    used_pct: 44.0,
                    resets_at: Some("2026-11-11T05:00:00.612334+00:00".into()),
                }),
                status: "ok".into(),
                message: None,
                fetched_at: "2026-11-09T07:59:30.106859+00:00".into(),
                subscription: Some("max".into()),
                breakdown: None,
            }),
            codex_quota: Some(CodexLiveQuota {
                five_hour: Some(LimitWindow {
                    used_pct: 0.0,
                    resets_at: Some("2026-11-09T12:59:30+00:00".into()),
                }),
                weekly: Some(LimitWindow {
                    used_pct: 61.0,
                    resets_at: Some("2026-11-11T06:43:34+00:00".into()),
                }),
                status: "ok".into(),
                message: None,
                fetched_at: "2026-11-09T07:59:30.179967+00:00".into(),
                plan_type: Some("plus".into()),
                reset_credits: Some(1),
            }),
        }
    }

    /// `v` as a reader of its JSON text sees it. serde_json's default float
    /// parser can land an ulp off (43.050000000000004 → 43.05), so compare
    /// parsed text with parsed text.
    fn as_parsed<T: Serialize>(v: &T) -> Value {
        serde_json::from_str(&serde_json::to_string(v).unwrap()).unwrap()
    }

    fn fixture_value() -> Value {
        let snapshot = serde_json::to_value(sample_snapshot()).unwrap();
        let pairing = parse_code(VEC_CODE).unwrap();
        let json = serde_json::to_vec(&snapshot).unwrap();
        let payload = seal_raw(&pairing.keys, &VEC_NONCE, &gzip(&json).unwrap()).unwrap();
        serde_json::json!({"code": VEC_CODE, "payload": payload, "json": snapshot})
    }

    /// Regenerates the fixture the TS self-test opens:
    /// `TC_SYNC_WRITE_FIXTURE=1 cargo test write_sync_fixture -- --ignored`.
    #[test]
    #[ignore = "writes tests/fixtures/sync_envelope_vector.json"]
    fn write_sync_fixture() {
        if std::env::var_os("TC_SYNC_WRITE_FIXTURE").is_none() {
            eprintln!("set TC_SYNC_WRITE_FIXTURE=1 to write the fixture");
            return;
        }
        let path = fixture_path();
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let mut text = serde_json::to_string_pretty(&fixture_value()).unwrap();
        text.push('\n');
        std::fs::write(&path, text).unwrap();
        eprintln!("wrote {}", path.display());
    }

    #[test]
    fn fixture_opens_and_matches_the_schema() {
        let text = std::fs::read_to_string(fixture_path()).expect(
            "fixture missing: TC_SYNC_WRITE_FIXTURE=1 cargo test write_sync_fixture -- --ignored",
        );
        let fixture: Value = serde_json::from_str(&text).unwrap();
        let pairing = parse_code(fixture["code"].as_str().unwrap()).unwrap();
        let opened = open(&pairing.keys, fixture["payload"].as_str().unwrap()).unwrap();
        let opened: Value = serde_json::from_slice(&opened).unwrap();
        assert_eq!(opened, fixture["json"]);
        // Still what the current Snapshot type serializes (else regenerate).
        assert_eq!(fixture["code"], VEC_CODE);
        assert_eq!(
            fixture["json"],
            as_parsed(&sample_snapshot()),
            "fixture is stale: TC_SYNC_WRITE_FIXTURE=1 cargo test write_sync_fixture -- --ignored"
        );
    }

    #[test]
    fn snapshot_serializes_the_spec_shape() {
        let v = serde_json::to_value(sample_snapshot()).unwrap();
        let keys: Vec<&str> = v.as_object().unwrap().keys().map(String::as_str).collect();
        for k in [
            "schema",
            "generatedAt",
            "device",
            "plan",
            "usage",
            "claudeQuota",
            "codexQuota",
        ] {
            assert!(keys.contains(&k), "missing {k}");
        }
        assert_eq!(v["schema"], 1);
        assert_eq!(v["device"]["appVersion"], "0.2.1");
        for r in ["session", "today", "week", "month", "all"] {
            assert!(v["usage"][r]["byModel"].is_object(), "usage.{r}");
        }
        assert_eq!(v["claudeQuota"]["fiveHour"]["usedPct"], 25.0);
        let mut none = sample_snapshot();
        none.claude_quota = None;
        none.codex_quota = None;
        let v = serde_json::to_value(none).unwrap();
        assert!(v["claudeQuota"].is_null() && v.get("codexQuota").is_some());
    }

    // -- content hash ---------------------------------------------------------

    #[test]
    fn content_hash_ignores_volatile_fields() {
        let base = sample_snapshot();
        let h = content_hash(&base);

        let mut s = base.clone();
        s.generated_at = "2026-11-09T08:01:00Z".into();
        let cq = s.claude_quota.as_mut().unwrap();
        cq.fetched_at = "2026-11-09T08:00:31.968199+00:00".into();
        cq.five_hour.as_mut().unwrap().resets_at = Some("2026-11-09T11:50:00.968199+00:00".into());
        let xq = s.codex_quota.as_mut().unwrap();
        xq.fetched_at = "2026-11-09T08:00:31+00:00".into();
        xq.five_hour.as_mut().unwrap().resets_at = Some("2026-11-09T13:00:31+00:00".into());
        assert_eq!(content_hash(&s), h);

        // Anything the user sees changes it.
        let mut s = base.clone();
        s.claude_quota
            .as_mut()
            .unwrap()
            .weekly
            .as_mut()
            .unwrap()
            .used_pct = 45.0;
        assert_ne!(content_hash(&s), h);
        let mut s = base.clone();
        s.usage.week.total_cost += 0.01;
        assert_ne!(content_hash(&s), h);
        let mut s = base.clone();
        s.codex_quota.as_mut().unwrap().status = "stale".into();
        assert_ne!(content_hash(&s), h);
        let mut s = base.clone();
        s.device.name = "MacBook Pro".into();
        assert_ne!(content_hash(&s), h);
    }

    #[test]
    fn content_hash_ignores_map_order_and_float_noise() {
        let base = sample_snapshot();
        let h = content_hash(&base);
        // Rebuild byModel in another insertion order (a new HashMap seed too).
        let mut s = base.clone();
        let mut entries: Vec<_> = s.usage.all.by_model.drain().collect();
        entries.reverse();
        s.usage.all.by_model = entries.into_iter().collect();
        assert_eq!(content_hash(&s), h);
        // An ulp of summation-order noise.
        let mut s = base.clone();
        let t = s.usage.month.total_cost;
        s.usage.month.total_cost = f64::from_bits(t.to_bits() + 1);
        assert_ne!(s.usage.month.total_cost, t);
        assert_eq!(content_hash(&s), h);

        let mut a = String::new();
        let mut b = String::new();
        write_canonical(&serde_json::json!({"b": 1, "a": [1.5, "x", null]}), &mut a);
        write_canonical(&serde_json::json!({"a": [1.5, "x", null], "b": 1}), &mut b);
        assert_eq!(a, b);
        assert_eq!(a, r#"{"a":[1.500000000e0,"x",null],"b":1}"#);
    }

    // -- publisher state ------------------------------------------------------

    #[test]
    fn backoff_doubles_to_ten_minutes() {
        let secs: Vec<u64> = (1..=8).map(|n| next_delay(n).as_secs()).collect();
        assert_eq!(secs, [60, 120, 240, 480, 600, 600, 600, 600]);
        assert_eq!(next_delay(u32::MAX).as_secs(), 600);
    }

    #[test]
    fn uploads_on_change_heartbeat_and_new_channel() {
        let t0 = DateTime::parse_from_rfc3339("2026-11-09T08:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let min = |m: i64| t0 + chrono::Duration::minutes(m);
        let (h1, h2) = ([1u8; 32], [2u8; 32]);
        let mut st = PublishState::new();
        st.adopt_channel("aaa");
        assert!(st.upload_due(&h1, t0), "nothing uploaded yet");
        st.record_success("aaa", h1, 7, t0);
        assert!(!st.upload_due(&h1, min(1)));
        assert!(!st.upload_due(&h1, min(9)));
        assert!(st.upload_due(&h1, min(10)), "heartbeat");
        assert!(st.upload_due(&h2, min(1)), "content changed");

        // Failures back off and keep the last good upload on show.
        st.record_failure("aaa", "Couldn't reach the sync server (timed out).", min(2));
        assert!(st.in_backoff(min(2)) && !st.in_backoff(min(3)));
        st.record_failure("aaa", "again", min(3));
        assert!(st.in_backoff(min(4)) && !st.in_backoff(min(5)));
        assert_eq!(st.last_version, Some(7));
        assert_eq!(st.last_error.as_deref(), Some("again"));
        st.record_success("aaa", h2, 8, min(5));
        assert!(!st.in_backoff(min(5)));
        assert_eq!((st.failures, st.last_error.as_deref()), (0, None));

        // A result for a channel no longer tracked is dropped.
        st.adopt_channel("bbb");
        assert!(st.last_version.is_none() && st.upload_due(&h2, min(6)));
        st.record_success("aaa", h2, 9, min(6));
        assert!(st.last_version.is_none());
        st.record_failure("aaa", "x", min(6));
        assert!(st.last_error.is_none());
    }

    #[test]
    fn status_reports_paired_only_with_a_role_and_valid_code() {
        let mut st = PublishState::new();
        st.last_version = Some(3);
        st.last_upload_at = Some(Utc::now());
        let mut config = SyncConfig {
            role: Some("primary".into()),
            code: Some(VEC_CODE.into()),
            device_name: Some("Mac Studio".into()),
            ..Default::default()
        };
        let s = status_of(&config, &st);
        assert!(s.paired);
        assert_eq!(
            (s.role.as_str(), s.device_name.as_str()),
            ("primary", "Mac Studio")
        );
        assert_eq!(s.last_version, Some(3));
        let v = serde_json::to_value(&s).unwrap();
        for k in [
            "role",
            "paired",
            "deviceName",
            "lastUploadAt",
            "lastVersion",
            "lastError",
            "lastErrorAt",
            "uploading",
        ] {
            assert!(v.get(k).is_some(), "missing {k}");
        }
        config.role = Some("off".into());
        assert!(!status_of(&config, &st).paired);
        config.role = Some("viewer".into());
        config.code = Some("tc1.broken".into());
        assert!(!status_of(&config, &st).paired);
    }

    // -- config -----------------------------------------------------------------

    #[test]
    fn config_round_trip_is_private_and_atomic() {
        let dir = temp_dir("config");
        let path = dir.join("sync.json");
        assert!(load_config_at(&path) == SyncConfig::default());
        assert_eq!(load_config_at(&path).role(), Role::Off);

        let config = SyncConfig {
            role: Some("primary".into()),
            code: Some(VEC_CODE.into()),
            device_name: Some("Mac Studio".into()),
            plan: Some("max20".into()),
            web_url: Some("https://example.com/tokens/".into()),
        };
        save_config_at(&path, &config).unwrap();
        assert!(load_config_at(&path) == config);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&path).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
        }
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("\"deviceName\"") && text.contains("\"webUrl\""));
        let leftovers: Vec<_> = std::fs::read_dir(&dir).unwrap().flatten().collect();
        assert_eq!(leftovers.len(), 1, "no tmp file left behind");

        // An existing 0644 file is replaced by a 0600 one.
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
            save_config_at(&path, &config).unwrap();
            let mode = std::fs::metadata(&path).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
        }

        // Unknown fields ignored; garbage or an unknown role reads as off.
        std::fs::write(&path, r#"{"role":"viewer","future":true}"#).unwrap();
        assert_eq!(load_config_at(&path).role(), Role::Viewer);
        std::fs::write(&path, r#"{"role":"boss"}"#).unwrap();
        assert_eq!(load_config_at(&path).role(), Role::Off);
        std::fs::write(&path, "not json").unwrap();
        assert_eq!(load_config_at(&path).role(), Role::Off);
        assert_eq!(load_config_at(&path).plan(), "max5x");
        cleanup(&dir);
    }

    fn change(role: &str, code: Option<&str>) -> ConfigChange {
        ConfigChange {
            role: role.into(),
            code: code.map(String::from),
            device_name: None,
            web_url: None,
        }
    }

    #[test]
    fn config_changes_validate_before_saving() {
        let dir = temp_dir("change");
        let path = dir.join("sync.json");
        let ok = |_: &Pairing| Ok(());
        let refused = |_: &Pairing| Err(RpcError::Forbidden.to_string());

        // Off needs no code and no server.
        let (c, _) = apply_config_change(&path, change("off", None), |_| {
            panic!("off must not call the server")
        })
        .unwrap();
        assert_eq!(c.role(), Role::Off);

        assert_eq!(
            apply_config_change(&path, change("primary", None), ok).unwrap_err(),
            "Paste a pairing code first."
        );
        assert!(apply_config_change(&path, change("primary", Some("tc1.nope")), ok).is_err());
        assert!(apply_config_change(&path, change("admin", Some(VEC_CODE)), ok).is_err());
        let err =
            apply_config_change(&path, change("viewer", Some(VEC_CODE)), refused).unwrap_err();
        assert_eq!(err, "This pairing code isn't registered on the server.");
        assert_eq!(
            load_config_at(&path).role(),
            Role::Off,
            "nothing saved on failure"
        );

        let mut seen = None;
        let (c, restart) = apply_config_change(&path, change("primary", Some(VEC_CODE)), |p| {
            seen = Some(p.keys.channel.clone());
            Ok(())
        })
        .unwrap();
        assert!(restart);
        assert_eq!(seen.as_deref(), Some(VEC_CHANNEL));
        assert!(load_config_at(&path) == c);
        // A code pasted with line breaks is stored clean, and is no change.
        let wrapped = format!("{}\n  {}", &VEC_CODE[..40], &VEC_CODE[40..]);
        let (c, restart) =
            apply_config_change(&path, change("primary", Some(&wrapped)), ok).unwrap();
        assert!(!restart);
        assert_eq!(c.code.as_deref(), Some(VEC_CODE));

        // None keeps, "" clears; same role + code needs no restart.
        let (c, restart) = apply_config_change(
            &path,
            ConfigChange {
                role: "primary".into(),
                code: None,
                device_name: Some("  Studio  ".into()),
                web_url: Some("https://example.com/t/".into()),
            },
            ok,
        )
        .unwrap();
        assert!(!restart);
        assert_eq!(c.code.as_deref(), Some(VEC_CODE));
        assert_eq!(c.device_name(), "Studio");
        let (c, _) = apply_config_change(
            &path,
            ConfigChange {
                role: "primary".into(),
                code: None,
                device_name: Some(String::new()),
                web_url: Some(String::new()),
            },
            ok,
        )
        .unwrap();
        assert!(c.device_name.is_none() && c.web_url.is_none());
        assert!(apply_config_change(
            &path,
            ConfigChange {
                role: "primary".into(),
                code: None,
                device_name: None,
                web_url: Some("javascript:alert(1)".into()),
            },
            ok,
        )
        .is_err());

        // Switching off keeps the code for later; clearing it while active fails.
        let (c, restart) = apply_config_change(&path, change("off", None), ok).unwrap();
        assert!(restart);
        assert_eq!(c.code.as_deref(), Some(VEC_CODE));
        assert!(apply_config_change(&path, change("viewer", Some("")), ok).is_err());
        let (c, _) = apply_config_change(&path, change("off", Some("")), ok).unwrap();
        assert!(c.code.is_none());
        cleanup(&dir);
    }

    #[test]
    fn possessive_prefix_is_stripped() {
        assert_eq!(strip_possessive("Alice's Mac Studio"), "Mac Studio");
        assert_eq!(
            strip_possessive("Alice\u{2019}s MacBook Pro"),
            "MacBook Pro"
        );
        assert_eq!(strip_possessive("  Mac Studio \n"), "Mac Studio");
        assert_eq!(strip_possessive("Bob's"), "Bob's");
        assert_eq!(strip_possessive("Bob's "), "Bob's");
        assert_eq!(strip_possessive("Ann’s Bob's Mac"), "Bob's Mac");
        assert!(!default_device_name().is_empty());
    }

    #[test]
    fn rpc_refuses_plain_http_servers() {
        let code = make_code("http://example.com", "k", &vec_secret());
        let p = parse_code(&code).unwrap();
        assert!(matches!(tc_head(&p), Err(RpcError::Insecure)));
        assert!(is_allowed_server("https://x.supabase.co"));
        assert!(is_allowed_server("http://localhost:54321"));
        assert!(is_allowed_server("http://127.0.0.1"));
        assert!(!is_allowed_server("http://localhost.example.com"));
        assert!(!is_allowed_server("http://127.0.0.1.example.com/x"));
        assert!(!is_allowed_server("ftp://localhost"));
    }

    // -- live + real-data smoke (ignored) ---------------------------------------

    /// Uploads a synthetic snapshot to the test channel and reads it back:
    /// `TC_SYNC_TEST_CODE=… cargo test live_round_trip -- --ignored --nocapture`.
    #[test]
    #[ignore = "network: uploads to the test channel in TC_SYNC_TEST_CODE"]
    fn live_round_trip() {
        let Some(code) = std::env::var("TC_SYNC_TEST_CODE")
            .ok()
            .filter(|c| !c.trim().is_empty())
        else {
            eprintln!("TC_SYNC_TEST_CODE unset; skipping");
            return;
        };
        let pairing = parse_code(&code).expect("TC_SYNC_TEST_CODE is not a valid code");
        let before = tc_head(&pairing).expect("tc_head");
        let before_version = before["version"].as_i64().unwrap_or(0);

        let mut snapshot = sample_snapshot();
        snapshot.generated_at = now_iso();
        snapshot.device.name = "Rust live test".into();
        let version = upload(&pairing, &snapshot).expect("tc_put");
        assert!(
            version > before_version,
            "version {version} after {before_version}"
        );

        let head = tc_head(&pairing).expect("tc_head after put");
        assert_eq!(head["version"].as_i64(), Some(version));
        assert!(
            head["device"].is_null(),
            "device name stays inside the envelope"
        );
        let got = tc_get(&pairing).expect("tc_get");
        assert_eq!(got["version"].as_i64(), Some(version));
        let opened = open(&pairing.keys, got["payload"].as_str().expect("payload")).unwrap();
        let opened: Value = serde_json::from_slice(&opened).unwrap();
        assert_eq!(opened, as_parsed(&snapshot));

        // A wrong token is refused with the friendly message.
        let wrong = Pairing {
            url: pairing.url.clone(),
            api_key: pairing.api_key.clone(),
            keys: Keys {
                channel: pairing.keys.channel.clone(),
                token: URL_SAFE_NO_PAD.encode([0u8; 32]),
                enc_key: [0u8; 32],
            },
        };
        let err = tc_head(&wrong).unwrap_err();
        assert!(matches!(err, RpcError::Forbidden), "{err}");
        eprintln!(
            "live round trip ok: version {before_version} -> {version}, generatedAt {}",
            got["generatedAt"]
        );
    }

    /// The publisher's decisions against the real server, with a temp config
    /// and synthetic snapshots: first upload, unchanged content skipped,
    /// changed content and "Sync now" uploaded, an unregistered code backs off.
    /// `TC_SYNC_TEST_CODE=… cargo test live_publisher -- --ignored --nocapture`
    #[test]
    #[ignore = "network: uploads to the test channel in TC_SYNC_TEST_CODE"]
    fn live_publisher_cycle() {
        let Some(code) = std::env::var("TC_SYNC_TEST_CODE")
            .ok()
            .filter(|c| !c.trim().is_empty())
        else {
            eprintln!("TC_SYNC_TEST_CODE unset; skipping");
            return;
        };
        let config = SyncConfig {
            role: Some("primary".into()),
            code: Some(code.clone()),
            device_name: Some("Rust publisher test".into()),
            ..Default::default()
        };
        let snap = |weekly: f64| {
            move |c: &SyncConfig| {
                let mut s = sample_snapshot();
                s.generated_at = now_iso();
                s.device.name = c.device_name();
                let cq = s.claude_quota.as_mut().unwrap();
                cq.fetched_at = Utc::now().to_rfc3339();
                cq.weekly.as_mut().unwrap().used_pct = weekly;
                s
            }
        };
        let cfg = || config.clone();

        let first = publish_cycle_with(false, cfg, snap(44.0)).expect("first upload");
        let v1 = first.last_version.expect("version after first upload");
        assert!(first.paired && !first.uploading && first.last_error.is_none());
        // Same content, new timestamps: nothing to upload.
        let same = publish_cycle_with(false, cfg, snap(44.0)).unwrap();
        assert_eq!(same.last_version, Some(v1));
        assert_eq!(same.last_upload_at, first.last_upload_at);
        let changed = publish_cycle_with(false, cfg, snap(45.0)).unwrap();
        assert_eq!(changed.last_version, Some(v1 + 1));
        let forced = publish_cycle_with(true, cfg, snap(45.0)).unwrap();
        assert_eq!(forced.last_version, Some(v1 + 2));

        // What landed is the last snapshot, decryptable with the code.
        let pairing = parse_code(&code).unwrap();
        let got = tc_get(&pairing).unwrap();
        assert_eq!(got["version"].as_i64(), Some(v1 + 2));
        let opened = open(&pairing.keys, got["payload"].as_str().unwrap()).unwrap();
        let opened: Value = serde_json::from_slice(&opened).unwrap();
        assert_eq!(opened["claudeQuota"]["weekly"]["usedPct"], 45.0);
        assert_eq!(opened["device"]["name"], "Rust publisher test");

        // A well-formed code the server doesn't know: error, then backoff.
        let mut secret = [0u8; 32];
        secret.copy_from_slice(&Sha256::digest(now_iso().as_bytes()));
        let stranger = SyncConfig {
            code: Some(make_code(&pairing.url, &pairing.api_key, &secret)),
            ..config.clone()
        };
        let err = publish_cycle_with(false, || stranger.clone(), snap(1.0)).unwrap_err();
        assert_eq!(err, "This pairing code isn't registered on the server.");
        let backing_off = publish_cycle_with(
            false,
            || stranger.clone(),
            |_| panic!("no snapshot is built while backing off"),
        )
        .unwrap();
        assert_eq!(backing_off.last_error.as_deref(), Some(err.as_str()));
        assert!(backing_off.last_version.is_none() && backing_off.last_error_at.is_some());
        // Back on the registered code: state starts over and uploads at once.
        let back = publish_cycle_with(false, cfg, snap(45.0)).unwrap();
        assert_eq!(back.last_version, Some(v1 + 3));
        assert!(back.last_error.is_none());
        eprintln!("live publisher ok: versions {v1}..{}", v1 + 3);
    }

    /// Builds a REAL snapshot through `build_snapshot` from this machine's
    /// data and prints only sizes and timings; nothing is uploaded. Quotas come
    /// from the persisted cache files (no Keychain, no network). Under
    /// cfg(test) usage skips the all-time store, the price table and product
    /// rows, so All is window-only here.
    /// `cargo test --release smoke_real_snapshot -- --ignored --nocapture`
    #[test]
    #[ignore = "reads this machine's real transcripts"]
    fn smoke_real_snapshot() {
        let claude = crate::load_quota_cache::<ClaudeQuota>("claude_quota.json");
        let codex = crate::load_quota_cache::<CodexLiveQuota>("codex_quota.json");
        let quotas = (claude.is_some(), codex.is_some());
        let mut hashes = Vec::new();
        let mut strict = Vec::new();
        let mut values = Vec::new();
        for pass in 1..=2 {
            let t = std::time::Instant::now();
            let snapshot = build_snapshot("smoke", "max5x", claude.clone(), codex.clone());
            let built = t.elapsed();
            let json = serde_json::to_vec(&snapshot).unwrap();
            let gz = gzip(&json).unwrap();
            let keys = derive_keys(&[0x5a; 32]);
            let t = std::time::Instant::now();
            let envelope = seal(&keys, &json).unwrap();
            let sealed = t.elapsed();
            eprintln!(
                "pass {pass}: build {:.2}s, json {} B, gzip {} B, envelope {} chars ({:.2}% of limit), seal {:.1} ms, quotas cached={quotas:?}",
                built.as_secs_f64(),
                json.len(),
                gz.len(),
                envelope.len(),
                envelope.len() as f64 / MAX_PAYLOAD_CHARS as f64 * 100.0,
                sealed.as_secs_f64() * 1000.0,
            );
            assert!(envelope.len() < MAX_PAYLOAD_CHARS);
            hashes.push(content_hash(&snapshot));
            let mut stripped = serde_json::to_value(&snapshot).unwrap();
            strip_volatile(&mut stripped);
            values.push(stripped);
            // What a hash of the spec's three-field strip alone would see.
            let mut v = serde_json::to_value(&snapshot).unwrap();
            v.as_object_mut().unwrap().remove("generatedAt");
            for q in ["claudeQuota", "codexQuota"] {
                if let Some(o) = v.get_mut(q).and_then(Value::as_object_mut) {
                    o.remove("fetchedAt");
                }
            }
            strict.push(Sha256::digest(serde_json::to_vec(&v).unwrap()));
        }
        eprintln!(
            "back-to-back builds: canonical hash equal = {}, plain-serialization hash equal = {}",
            hashes[0] == hashes[1],
            strict[0] == strict[1]
        );
        // Which fields changed between the passes (paths only, no values).
        let mut changed = Vec::new();
        diff_paths(&values[0], &values[1], String::new(), &mut changed);
        eprintln!(
            "changed paths ({}): {:?}",
            changed.len(),
            &changed[..changed.len().min(40)]
        );
    }

    fn diff_paths(a: &Value, b: &Value, path: String, out: &mut Vec<String>) {
        match (a, b) {
            (Value::Object(x), Value::Object(y)) => {
                let mut keys: Vec<&String> = x.keys().chain(y.keys()).collect();
                keys.sort();
                keys.dedup();
                for k in keys {
                    let (va, vb) = (
                        x.get(k).unwrap_or(&Value::Null),
                        y.get(k).unwrap_or(&Value::Null),
                    );
                    diff_paths(va, vb, format!("{path}.{k}"), out);
                }
            }
            (Value::Array(x), Value::Array(y)) if x.len() == y.len() => {
                for (i, (va, vb)) in x.iter().zip(y).enumerate() {
                    diff_paths(va, vb, format!("{path}[{i}]"), out);
                }
            }
            _ => {
                let (mut ca, mut cb) = (String::new(), String::new());
                write_canonical(a, &mut ca);
                write_canonical(b, &mut cb);
                if ca != cb {
                    out.push(path);
                }
            }
        }
    }
}

// Token Counter cloud sync — wire protocol (SPEC §1–§3).
//
// Pure module: runs in browsers (WKWebView, iOS Safari 16.4+) and in Node ≥ 20
// (scripts/sync-selftest.mjs imports this file directly through Node's type
// stripping). So: WebCrypto + CompressionStream + fetch + atob/btoa only, no
// `window`, no Buffer, `import type` for types, no enums/namespaces.
//
// The server only ever sees ciphertext. A pairing code carries the Supabase
// URL, its publishable key and a 32-byte secret K; everything else (channel,
// auth token, AES key) is derived from K. Never log a code, K or derived keys.
import type { Snapshot } from '../types.ts';

export type SyncErrorKind =
  | 'bad-code' // not a parsable tc1 pairing code
  | 'forbidden' // server refused the channel/token (unregistered or revoked)
  | 'decrypt-failed' // envelope malformed or GCM auth failed (code doesn't match)
  | 'bad-snapshot' // decrypted fine but not a snapshot this app understands
  | 'network' // fetch failed, timed out, or a non-auth server error
  | 'empty'; // channel exists but nothing was uploaded yet

export class SyncError extends Error {
  kind: SyncErrorKind;
  status: number | null;
  constructor(kind: SyncErrorKind, message: string, status: number | null = null) {
    super(message);
    this.name = 'SyncError';
    this.kind = kind;
    this.status = status;
  }
}

export function isSyncError(e: unknown): e is SyncError {
  return e instanceof SyncError;
}

export interface PairingCode {
  url: string; // Supabase project URL, no trailing slash
  apiKey: string; // publishable key
  secret: Uint8Array; // K, 32 bytes
}

export interface SyncKeys {
  url: string;
  apiKey: string;
  channel: string; // 32 lowercase hex chars
  token: string; // base64url, no padding
  encKey: Uint8Array; // AES-256 key bytes
}

const CODE_PREFIX = 'tc1.';
const ENVELOPE_VERSION = 1;
const NONCE_LEN = 12;
const TAG_LEN = 16;
const enc = new TextEncoder();

// ---------- bytes helpers ----------

function concat(...parts: Uint8Array[]): Uint8Array<ArrayBuffer> {
  const out = new Uint8Array(parts.reduce((n, p) => n + p.length, 0));
  let o = 0;
  for (const p of parts) {
    out.set(p, o);
    o += p.length;
  }
  return out;
}

export function toHex(b: Uint8Array): string {
  let s = '';
  for (const x of b) s += x.toString(16).padStart(2, '0');
  return s;
}

function bytesToBinary(b: Uint8Array): string {
  let s = '';
  const CHUNK = 0x8000;
  for (let i = 0; i < b.length; i += CHUNK) {
    s += String.fromCharCode.apply(null, Array.from(b.subarray(i, i + CHUNK)));
  }
  return s;
}

function binaryToBytes(s: string): Uint8Array<ArrayBuffer> {
  const out = new Uint8Array(s.length);
  for (let i = 0; i < s.length; i++) out[i] = s.charCodeAt(i);
  return out;
}

export function base64Encode(b: Uint8Array): string {
  return btoa(bytesToBinary(b));
}

export function base64Decode(s: string): Uint8Array<ArrayBuffer> {
  return binaryToBytes(atob(s));
}

export function base64UrlEncode(b: Uint8Array): string {
  return base64Encode(b).replace(/\+/g, '-').replace(/\//g, '_').replace(/=+$/, '');
}

// Accepts base64url with or without padding. Throws on anything else.
export function base64UrlDecode(s: string): Uint8Array<ArrayBuffer> {
  if (!/^[A-Za-z0-9_-]*={0,2}$/.test(s)) throw new Error('not base64url');
  const t = s.replace(/=+$/, '').replace(/-/g, '+').replace(/_/g, '/');
  if (t.length % 4 === 1) throw new Error('bad base64url length');
  return base64Decode(t + '='.repeat((4 - (t.length % 4)) % 4));
}

async function sha256(data: Uint8Array<ArrayBuffer>): Promise<Uint8Array<ArrayBuffer>> {
  return new Uint8Array(await crypto.subtle.digest('SHA-256', data));
}

// ---------- pairing code + key derivation (§2) ----------

/** Parse a `tc1.` pairing code. Surrounding (and wrapped-line) whitespace is
 *  ignored, unknown JSON fields too. Throws SyncError('bad-code'). */
export function parseCode(input: string): PairingCode {
  const bad = (why: string) => new SyncError('bad-code', `Not a Token Counter pairing code (${why}).`);
  const s = (input ?? '').replace(/\s+/g, '');
  if (!s.startsWith(CODE_PREFIX)) throw bad('it should start with “tc1.”');
  let obj: unknown;
  try {
    const json = new TextDecoder('utf-8', { fatal: true }).decode(base64UrlDecode(s.slice(CODE_PREFIX.length)));
    obj = JSON.parse(json);
  } catch {
    throw bad('it is cut off or garbled');
  }
  if (!obj || typeof obj !== 'object' || Array.isArray(obj)) throw bad('no payload');
  const o = obj as Record<string, unknown>;
  if (o.v !== 1) throw bad(`unsupported version ${String(o.v)}`);
  if (typeof o.u !== 'string' || !o.u) throw bad('missing server URL');
  if (typeof o.k !== 'string' || !o.k) throw bad('missing server key');
  if (typeof o.s !== 'string' || !o.s) throw bad('missing secret');
  let url: URL;
  try {
    url = new URL(o.u);
  } catch {
    throw bad('invalid server URL');
  }
  if (url.protocol !== 'https:' && url.protocol !== 'http:') throw bad('invalid server URL');
  let secret: Uint8Array;
  try {
    secret = base64UrlDecode(o.s);
  } catch {
    throw bad('invalid secret');
  }
  if (secret.length !== 32) throw bad('secret is not 32 bytes');
  return { url: o.u.replace(/\/+$/, ''), apiKey: o.k, secret };
}

/** Build a pairing code (tests / dev tooling; codes are normally made by the owner). */
export function encodeCode(c: PairingCode): string {
  const json = JSON.stringify({ v: 1, u: c.url, k: c.apiKey, s: base64UrlEncode(c.secret) });
  return CODE_PREFIX + base64UrlEncode(enc.encode(json));
}

async function labelled(label: string, secret: Uint8Array): Promise<Uint8Array<ArrayBuffer>> {
  return sha256(concat(enc.encode(label), secret));
}

export async function deriveKeys(code: PairingCode | string): Promise<SyncKeys> {
  const c = typeof code === 'string' ? parseCode(code) : code;
  const [ch, tok, key] = await Promise.all([
    labelled('tc-sync/channel/v1', c.secret),
    labelled('tc-sync/auth/v1', c.secret),
    labelled('tc-sync/enc/v1', c.secret),
  ]);
  return {
    url: c.url,
    apiKey: c.apiKey,
    channel: toHex(ch.subarray(0, 16)),
    token: base64UrlEncode(tok),
    encKey: key,
  };
}

// ---------- envelope (§3) ----------

const aesKeys = new WeakMap<Uint8Array, Promise<CryptoKey>>();

function aesKey(keys: SyncKeys): Promise<CryptoKey> {
  let k = aesKeys.get(keys.encKey);
  if (!k) {
    k = crypto.subtle.importKey('raw', new Uint8Array(keys.encKey), 'AES-GCM', false, ['encrypt', 'decrypt']);
    aesKeys.set(keys.encKey, k);
  }
  return k;
}

function aad(keys: SyncKeys): Uint8Array<ArrayBuffer> {
  return enc.encode(`tc-sync/snapshot/v1:${keys.channel}`) as Uint8Array<ArrayBuffer>;
}

/** Encrypt raw bytes into an envelope string. `nonce` is for test vectors
 *  only — production callers must let it default to a fresh random one. */
export async function sealBytes(plain: Uint8Array, keys: SyncKeys, nonce?: Uint8Array): Promise<string> {
  const iv = nonce ? new Uint8Array(nonce) : crypto.getRandomValues(new Uint8Array(NONCE_LEN));
  if (iv.length !== NONCE_LEN) throw new Error('nonce must be 12 bytes');
  const ct = new Uint8Array(
    await crypto.subtle.encrypt(
      { name: 'AES-GCM', iv, additionalData: aad(keys), tagLength: 128 },
      await aesKey(keys),
      new Uint8Array(plain),
    ),
  );
  return base64Encode(concat(Uint8Array.of(ENVELOPE_VERSION), iv, ct));
}

/** Decrypt an envelope string to raw bytes (still gzip'd for snapshots). */
export async function openBytes(payload: string, keys: SyncKeys): Promise<Uint8Array<ArrayBuffer>> {
  const fail = (why: string) => new SyncError('decrypt-failed', why);
  let raw: Uint8Array<ArrayBuffer>;
  try {
    raw = base64Decode(payload.trim());
  } catch {
    throw fail('The synced data is not valid base64.');
  }
  if (raw.length < 1 + NONCE_LEN + TAG_LEN) throw fail('The synced data is truncated.');
  if (raw[0] !== ENVELOPE_VERSION) throw fail(`Unsupported envelope version ${raw[0]}.`);
  try {
    return new Uint8Array(
      await crypto.subtle.decrypt(
        { name: 'AES-GCM', iv: raw.subarray(1, 1 + NONCE_LEN), additionalData: aad(keys), tagLength: 128 },
        await aesKey(keys),
        raw.subarray(1 + NONCE_LEN),
      ),
    );
  } catch {
    throw fail("The pairing code doesn't match the data on the server.");
  }
}

async function transform(bytes: Uint8Array, stream: CompressionStream | DecompressionStream): Promise<Uint8Array<ArrayBuffer>> {
  const piped = new Blob([new Uint8Array(bytes)]).stream().pipeThrough(stream);
  return new Uint8Array(await new Response(piped).arrayBuffer());
}

export function gzip(bytes: Uint8Array): Promise<Uint8Array<ArrayBuffer>> {
  return transform(bytes, new CompressionStream('gzip'));
}

export function gunzip(bytes: Uint8Array): Promise<Uint8Array<ArrayBuffer>> {
  return transform(bytes, new DecompressionStream('gzip'));
}

/** JSON → gzip → AES-GCM envelope (tests, dev tooling; the desktop primary seals in Rust). */
export async function sealEnvelope(snapshot: unknown, keys: SyncKeys, nonce?: Uint8Array): Promise<string> {
  return sealBytes(await gzip(enc.encode(JSON.stringify(snapshot))), keys, nonce);
}

/** Envelope → snapshot. Throws SyncError('decrypt-failed' | 'bad-snapshot'). */
export async function openEnvelope(payload: string, keys: SyncKeys): Promise<Snapshot> {
  const plain = await openBytes(payload, keys);
  let parsed: unknown;
  try {
    parsed = JSON.parse(new TextDecoder('utf-8', { fatal: true }).decode(await gunzip(plain)));
  } catch {
    throw new SyncError('bad-snapshot', 'The synced data could not be read (not gzip’d JSON).');
  }
  return asSnapshot(parsed);
}

/** Minimal shape check; everything else is optional for forward/backward compat. */
export function asSnapshot(v: unknown): Snapshot {
  const bad = (why: string) => new SyncError('bad-snapshot', `The synced data is not a usage snapshot (${why}).`);
  if (!v || typeof v !== 'object' || Array.isArray(v)) throw bad('not an object');
  const o = v as Record<string, unknown>;
  if (typeof o.schema !== 'number') throw bad('no schema');
  if (o.schema > 1) {
    // A newer primary. Still try: viewers tolerate unknown fields.
  }
  if (!o.usage || typeof o.usage !== 'object') throw bad('no usage');
  if (typeof o.generatedAt !== 'string') throw bad('no generatedAt');
  const device = o.device && typeof o.device === 'object' ? (o.device as Record<string, unknown>) : {};
  return {
    ...(o as unknown as Snapshot),
    device: {
      ...device,
      name: typeof device.name === 'string' && device.name.trim() ? device.name : 'Main Mac',
    },
  };
}

// ---------- server RPCs (§1) ----------

export interface HeadInfo {
  version: number;
  updatedAt: string | null;
  generatedAt: string | null;
  device: string | null;
}

export interface GetInfo extends HeadInfo {
  payload: string;
}

export interface RpcOptions {
  timeoutMs?: number;
  signal?: AbortSignal;
}

async function rpc(keys: SyncKeys, name: string, body: Record<string, unknown>, opts: RpcOptions = {}): Promise<unknown> {
  const ctrl = new AbortController();
  const timer = setTimeout(() => ctrl.abort(), opts.timeoutMs ?? 20_000);
  const onAbort = () => ctrl.abort();
  opts.signal?.addEventListener('abort', onAbort);
  let res: Response;
  try {
    res = await fetch(`${keys.url}/rest/v1/rpc/${name}`, {
      method: 'POST',
      headers: { apikey: keys.apiKey, 'Content-Type': 'application/json' },
      body: JSON.stringify(body),
      signal: ctrl.signal,
    });
  } catch {
    const timedOut = ctrl.signal.aborted && !opts.signal?.aborted;
    throw new SyncError('network', timedOut ? 'The sync server did not answer in time.' : "Can't reach the sync server.");
  } finally {
    clearTimeout(timer);
    opts.signal?.removeEventListener('abort', onAbort);
  }
  let text = '';
  try {
    text = await res.text();
  } catch {
    throw new SyncError('network', 'The connection to the sync server dropped.', res.status);
  }
  let json: unknown = null;
  try {
    json = text ? JSON.parse(text) : null;
  } catch {
    json = null;
  }
  if (res.status === 401 || res.status === 403) {
    const code = json && typeof json === 'object' ? (json as Record<string, unknown>).code : null;
    throw new SyncError(
      'forbidden',
      code === '42501'
        ? "This pairing code isn't registered on the server."
        : 'The sync server rejected this pairing code.',
      res.status,
    );
  }
  if (!res.ok) throw new SyncError('network', `The sync server answered ${res.status}.`, res.status);
  if (json === null && text) throw new SyncError('network', 'The sync server sent an unreadable reply.', res.status);
  return json;
}

function asHead(v: unknown): HeadInfo {
  const o = v && typeof v === 'object' ? (v as Record<string, unknown>) : {};
  const str = (x: unknown) => (typeof x === 'string' ? x : null);
  return {
    version: typeof o.version === 'number' ? o.version : 0,
    updatedAt: str(o.updatedAt),
    generatedAt: str(o.generatedAt),
    device: str(o.device),
  };
}

const auth = (keys: SyncKeys) => ({ p_channel: keys.channel, p_token: keys.token });

/** Current version + timestamps, no payload (cheap; polled). */
export async function head(keys: SyncKeys, opts?: RpcOptions): Promise<HeadInfo> {
  return asHead(await rpc(keys, 'tc_head', auth(keys), opts));
}

/** Version + the envelope. Throws SyncError('empty') when nothing was uploaded yet. */
export async function get(keys: SyncKeys, opts?: RpcOptions): Promise<GetInfo> {
  const r = await rpc(keys, 'tc_get', auth(keys), opts);
  const h = asHead(r);
  const payload = r && typeof r === 'object' ? (r as Record<string, unknown>).payload : null;
  if (typeof payload !== 'string' || !payload) throw new SyncError('empty', 'Nothing has been uploaded yet.');
  return { ...h, payload };
}

/** Upload an envelope; returns the new version. (Tests / dev — the desktop primary uploads from Rust.) */
export async function put(keys: SyncKeys, payload: string, generatedAt: string, opts?: RpcOptions): Promise<number> {
  const r = await rpc(
    keys,
    'tc_put',
    { ...auth(keys), p_payload: payload, p_device: null, p_generated_at: generatedAt },
    opts,
  );
  if (typeof r !== 'number') throw new SyncError('network', 'The sync server sent an unexpected reply to an upload.');
  return r;
}

/** get → open, as one call (selftest and first pairing). */
export async function fetchSnapshot(keys: SyncKeys, opts?: RpcOptions): Promise<{ snapshot: Snapshot; version: number }> {
  const g = await get(keys, opts);
  return { snapshot: await openEnvelope(g.payload, keys), version: g.version };
}

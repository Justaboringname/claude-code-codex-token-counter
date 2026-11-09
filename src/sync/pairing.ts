// Pairing state of the phone web app (and of `vite dev` with ?source=remote).
// The desktop app keeps its code in Rust (sync.json) instead.
//
// The code lives in this origin's localStorage. iOS home-screen apps get
// storage separate from Safari, so a fresh standalone app reads the code from
// the `#pair=<code>` fragment it was added with: Safari keeps the fragment
// (so "Add to Home Screen" captures it); the standalone app stores it and
// replaces it away.
import { useCallback, useEffect, useRef, useState } from 'react';
import type { Snapshot } from '../types';
import { SyncError, deriveKeys, get, head, isSyncError, openEnvelope, parseCode } from './protocol';
import { forgetRemote, seedRemote } from './remote';

export const CODE_STORAGE_KEY = 'sync-code-v1';

export function readStoredCode(): string | null {
  try {
    const c = window.localStorage.getItem(CODE_STORAGE_KEY);
    return c && c.startsWith('tc1.') ? c : null;
  } catch {
    return null;
  }
}

function storeCode(code: string) {
  try {
    window.localStorage.setItem(CODE_STORAGE_KEY, code);
  } catch {}
}

function clearStoredCode() {
  try {
    window.localStorage.removeItem(CODE_STORAGE_KEY);
  } catch {}
}

/** iOS home-screen app (or any installed PWA window). */
export function isStandalone(): boolean {
  const nav = navigator as Navigator & { standalone?: boolean };
  return nav.standalone === true || window.matchMedia?.('(display-mode: standalone)').matches === true;
}

/** The code in `#pair=…`, normalized, or null. */
function fragmentCode(): string | null {
  const m = window.location.hash.match(/[#&]pair=([^&]+)/);
  if (!m) return null;
  let v = m[1];
  try {
    v = decodeURIComponent(v);
  } catch {}
  return v.replace(/\s+/g, '');
}

function dropFragment() {
  if (!window.location.hash) return;
  const url = window.location.pathname + window.location.search;
  window.history.replaceState(window.history.state, '', url);
}

export interface PairResult {
  snapshot: Snapshot | null;
  version: number;
}

/** Validate a code against the server (tc_head, then the first snapshot if
 *  any). Throws SyncError: bad-code / forbidden / network / decrypt-failed. */
export async function checkCode(code: string): Promise<PairResult & { channel: string }> {
  const keys = await deriveKeys(parseCode(code));
  const h = await head(keys);
  if (h.version === 0) return { snapshot: null, version: 0, channel: keys.channel };
  try {
    const g = await get(keys);
    return { snapshot: await openEnvelope(g.payload, keys), version: g.version, channel: keys.channel };
  } catch (e) {
    if (isSyncError(e) && e.kind === 'empty') return { snapshot: null, version: 0, channel: keys.channel };
    throw e;
  }
}

export interface WebPairing {
  code: string | null;
  /** A pairing attempt is running (paste, or a `#pair=` link). */
  busy: boolean;
  /** Last failed attempt. */
  error: SyncError | null;
  pair: (code: string) => Promise<boolean>;
  /** Re-run the last failed attempt (e.g. a `#pair=` link opened offline). */
  retry: (() => void) | null;
  unpair: () => void;
}

/** Pairing for the web app. `active` = this build/page pairs in the browser. */
export function useWebPairing(active: boolean): WebPairing {
  const [code, setCode] = useState<string | null>(() => (active ? readStoredCode() : null));
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<SyncError | null>(null);
  const attempt = useRef(0);
  const [lastFailed, setLastFailed] = useState<string | null>(null);

  const pair = useCallback(async (raw: string): Promise<boolean> => {
    const id = ++attempt.current;
    setBusy(true);
    setError(null);
    try {
      const normalized = raw.replace(/\s+/g, '');
      const r = await checkCode(normalized);
      if (id !== attempt.current) return false;
      const prev = readStoredCode();
      if (prev && prev !== normalized) forgetRemote(prev);
      storeCode(normalized);
      if (r.snapshot) seedRemote(normalized, r.channel, r.snapshot, r.version);
      setCode(normalized);
      setLastFailed(null);
      return true;
    } catch (e) {
      if (id === attempt.current) {
        const err = isSyncError(e) ? e : new SyncError('network', String(e));
        setError(err);
        // Only worth retrying when the code itself was fine.
        setLastFailed(err.kind === 'network' ? raw : null);
      }
      return false;
    } finally {
      if (id === attempt.current) setBusy(false);
    }
  }, []);

  const unpair = useCallback(() => {
    attempt.current++;
    forgetRemote(readStoredCode());
    clearStoredCode();
    // A kept `#pair=` would pair again on the next load.
    dropFragment();
    setCode(null);
    setError(null);
    setLastFailed(null);
    setBusy(false);
  }, []);

  // `#pair=<code>` on load (QR link, or the URL a home-screen app was added with).
  useEffect(() => {
    if (!active) return;
    const fromUrl = fragmentCode();
    if (!fromUrl) return;
    const standalone = isStandalone();
    if (fromUrl === readStoredCode()) {
      if (standalone) dropFragment();
      return;
    }
    void pair(fromUrl).then((ok) => {
      if (ok && standalone) dropFragment();
    });
  }, [active, pair]);

  const retry = lastFailed ? () => void pair(lastFailed) : null;
  return { code, busy, error, pair, retry, unpair };
}

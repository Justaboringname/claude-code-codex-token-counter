// Pairing state of the phone web app (and of `vite dev` with ?source=remote).
// The desktop app keeps its code in Rust (sync.json) instead.
//
// The code lives in this origin's localStorage. iOS home-screen apps get
// storage separate from Safari, so a fresh standalone app reads the code from
// the `#pair=<code>` fragment it was added with: Safari keeps the fragment
// (so "Add to Home Screen" captures it); the standalone app stores it and
// replaces it away.
//
// A `#pair=` link pairs by itself only when nothing is stored. If a different
// code is stored, the link becomes a pending replacement that the user has to
// confirm (naming the link's server); nothing is sent to that server before
// they do. A link must never silently move this phone to another server.
//
// iOS relaunches a home-screen app from the URL it was added with, fragment
// included, on every cold start. So in standalone mode the launch link is
// remembered once it has been applied or declined (a SHA-256 of the code, not
// the code), and a remembered link is ignored on later launches. Otherwise
// "Unpair" or a pasted new code would be undone by the next cold launch.
import { useCallback, useEffect, useRef, useState } from 'react';
import type { Snapshot } from '../types';
import { SyncError, deriveKeys, get, head, isSyncError, openEnvelope, parseCode, toHex } from './protocol';
import { forgetRemote, seedRemote } from './remote';

export const CODE_STORAGE_KEY = 'sync-code-v1';
const LAUNCH_LINK_KEY = 'sync-launch-link-used-v1';

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

async function linkHash(code: string): Promise<string> {
  const d = await crypto.subtle.digest('SHA-256', new TextEncoder().encode(`tc-sync/launch-link/v1:${code}`));
  return toHex(new Uint8Array(d));
}

async function isLinkUsed(code: string): Promise<boolean> {
  try {
    return window.localStorage.getItem(LAUNCH_LINK_KEY) === (await linkHash(code));
  } catch {
    return false;
  }
}

async function markLinkUsed(code: string) {
  try {
    window.localStorage.setItem(LAUNCH_LINK_KEY, await linkHash(code));
  } catch {}
}

/** iOS home-screen app (or any installed PWA window). */
export function isStandalone(): boolean {
  const nav = navigator as Navigator & { standalone?: boolean };
  return nav.standalone === true || window.matchMedia?.('(display-mode: standalone)').matches === true;
}

/** A pasted code: whitespace dropped, and the code taken out of a whole
 *  `…#pair=tc1.…` link (e.g. copied from Safari's address bar). */
export function extractCode(raw: string): string {
  const s = (raw ?? '').replace(/\s+/g, '');
  const m = s.match(/[#&?]pair=([^&#]+)/);
  if (!m) return s;
  try {
    return decodeURIComponent(m[1]);
  } catch {
    return m[1];
  }
}

/** The code in `#pair=…`, normalized, or null. */
function fragmentCode(): string | null {
  const m = window.location.hash.match(/[#&]pair=([^&]+)/);
  if (!m) return null;
  let v = m[1];
  try {
    v = decodeURIComponent(v);
  } catch {}
  return v.replace(/\s+/g, '') || null;
}

function dropFragment() {
  if (!window.location.hash) return;
  const url = window.location.pathname + window.location.search;
  window.history.replaceState(window.history.state, '', url);
}

/** The sync server a code points at ("abcd.supabase.co"), or null. */
export function codeHost(code: string | null): string | null {
  if (!code) return null;
  try {
    return new URL(parseCode(code).url).host;
  } catch {
    return null;
  }
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

/** A `#pair=` link for a different code than the stored one, waiting for the user. */
export interface PendingReplace {
  code: string;
  /** Server of the link's code. */
  host: string;
  /** Server of the code in use. */
  currentHost: string | null;
}

export interface WebPairing {
  code: string | null;
  /** iOS home-screen app (scanning a QR code opens Safari, not this app). */
  standalone: boolean;
  /** A pairing attempt is running (paste, or a `#pair=` link). */
  busy: boolean;
  /** Last failed attempt. */
  error: SyncError | null;
  pair: (code: string) => Promise<boolean>;
  /** Re-run the last failed attempt (e.g. a `#pair=` link opened offline). */
  retry: (() => void) | null;
  unpair: () => void;
  /** A link that would replace the current pairing, until confirmed or cancelled. */
  pendingReplace: PendingReplace | null;
  confirmReplace: () => void;
  cancelReplace: () => void;
  /** Home-screen app, unpaired: pair again with the link it was added with. */
  relink: (() => void) | null;
}

/** Pairing for the web app. `active` = this build/page pairs in the browser. */
export function useWebPairing(active: boolean): WebPairing {
  // Read once: the fragment is replaced away later, and only the URL this
  // page was opened with counts as "the link".
  const [launchLink] = useState<string | null>(() => (active ? fragmentCode() : null));
  const [standalone] = useState(() => active && isStandalone());
  const [code, setCode] = useState<string | null>(() => (active ? readStoredCode() : null));
  // A launch link with nothing stored is about to pair: start busy, so the
  // pairing form doesn't flash first.
  const [busy, setBusy] = useState(() => !!launchLink && !readStoredCode());
  const [error, setError] = useState<SyncError | null>(null);
  const attempt = useRef(0);
  const [lastFailed, setLastFailed] = useState<string | null>(null);
  const [pendingReplace, setPendingReplace] = useState<PendingReplace | null>(null);
  const [linkUsed, setLinkUsed] = useState(false);

  const pair = useCallback(async (raw: string): Promise<boolean> => {
    const id = ++attempt.current;
    setBusy(true);
    setError(null);
    try {
      const normalized = extractCode(raw);
      const r = await checkCode(normalized);
      if (id !== attempt.current) return false;
      const prev = readStoredCode();
      if (prev && prev !== normalized) forgetRemote(prev);
      storeCode(normalized);
      if (r.snapshot) seedRemote(normalized, r.channel, r.snapshot, r.version);
      setCode(normalized);
      setLastFailed(null);
      setPendingReplace(null);
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
    // A kept `#pair=` would pair again on the next load (Safari tab). A
    // home-screen app's launch link was remembered when it was applied.
    dropFragment();
    setCode(null);
    setError(null);
    setLastFailed(null);
    setPendingReplace(null);
    setBusy(false);
  }, []);

  // `#pair=<code>` on load (QR link, or the URL a home-screen app was added with).
  useEffect(() => {
    if (!active || !launchLink) return;
    let cancelled = false;
    void (async () => {
      await null; // StrictMode runs this twice in dev; act on the second run only
      if (cancelled) return;
      const stored = readStoredCode();
      if (launchLink === stored) return; // the effect below remembers it
      if (standalone && (await isLinkUsed(launchLink))) {
        // Applied or declined on an earlier launch: an Unpair or a newer code stays.
        if (cancelled) return;
        dropFragment();
        setLinkUsed(true);
        setBusy(false);
        return;
      }
      if (cancelled) return;
      if (!stored) {
        await pair(launchLink);
        return;
      }
      setBusy(false);
      const host = codeHost(launchLink);
      if (!host) {
        // Not a pairing code: nothing to offer while paired.
        dropFragment();
        return;
      }
      setPendingReplace({ code: launchLink, host, currentHost: codeHost(stored) });
    })();
    return () => {
      cancelled = true;
    };
  }, [active, launchLink, standalone, pair]);

  // Home-screen app: once the launch link is the code in use (first launch,
  // a retry, a confirmed replacement), remember it and drop it from the URL.
  useEffect(() => {
    if (!active || !standalone || !launchLink || code !== launchLink) return;
    void markLinkUsed(launchLink).then(() => setLinkUsed(true));
    dropFragment();
  }, [active, standalone, launchLink, code]);

  const confirmReplace = useCallback(() => {
    if (pendingReplace) void pair(pendingReplace.code);
  }, [pendingReplace, pair]);

  const cancelReplace = useCallback(() => {
    attempt.current++;
    if (pendingReplace && standalone) void markLinkUsed(pendingReplace.code).then(() => setLinkUsed(true));
    dropFragment();
    setPendingReplace(null);
    setError(null);
    setLastFailed(null);
    setBusy(false);
  }, [pendingReplace, standalone]);

  const retry = lastFailed ? () => void pair(lastFailed) : null;
  const relink = standalone && launchLink && linkUsed && !code && !lastFailed ? () => void pair(launchLink) : null;
  return {
    code,
    standalone,
    busy,
    error,
    pair,
    retry,
    unpair,
    pendingReplace,
    confirmReplace,
    cancelReplace,
    relink,
  };
}

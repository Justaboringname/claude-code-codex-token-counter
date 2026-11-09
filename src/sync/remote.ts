// Remote snapshot store (SPEC §7.3): what a viewer Mac or the phone web app
// shows instead of local scans.
//
// - Polls tc_head every 30 s while the page/window is visible, and right away
//   on focus / visibilitychange / coming back online.
// - Downloads (tc_get) only when the version changed, or when forced by the
//   refresh button.
// - Keeps the last good decrypted snapshot in localStorage
//   (`sync-snapshot-v1`, tagged with its channel so a re-pair never flashes
//   another channel's numbers) for instant display and offline use.
//
// - Every snapshot is validated (protocol.ts asSnapshot) before it is shown or
//   cached, the cached one again when it is read back. A version that fails
//   to open is remembered and not downloaded again until the version moves
//   or the user forces a refresh.
//
// One store per pairing code, shared by every hook that asks for it.
import { useCallback, useEffect, useSyncExternalStore } from 'react';
import type { Snapshot } from '../types';
import { SyncError, asSnapshot, deriveKeys, get, head, isSyncError, openEnvelope, type SyncKeys } from './protocol';

export interface RemoteState {
  /** False until the cached snapshot was checked (a few ms after creation). */
  initialized: boolean;
  snapshot: Snapshot | null;
  version: number | null;
  /** When the server last confirmed `version` is current (ms epoch). */
  fetchedAt: number | null;
  /** Last attempt's error; the snapshot (if any) is kept alongside it. */
  error: SyncError | null;
  refreshing: boolean;
}

export interface RemoteView extends RemoteState {
  refresh: (force?: boolean) => Promise<void>;
  /** Drop the snapshot on screen and its cached copy, then download again. */
  clear: () => void;
}

export const SNAPSHOT_STORAGE_KEY = 'sync-snapshot-v1';
const POLL_MS = 30_000;
// Focus/visibility bursts: don't re-check more often than this.
const MIN_RECHECK_MS = 5_000;

interface CachedSnapshot {
  channel: string;
  version: number;
  fetchedAt: number;
  snapshot: Snapshot;
}

// The cached copy goes through the same validation as a download: a cache
// written by an older build (or by hand) must not be able to break the render
// on every launch. An unusable cache is removed.
function readCache(): CachedSnapshot | null {
  try {
    const raw = window.localStorage.getItem(SNAPSHOT_STORAGE_KEY);
    if (!raw) return null;
    const c = JSON.parse(raw) as Record<string, unknown> | null;
    const num = (v: unknown) => typeof v === 'number' && Number.isFinite(v);
    if (!c || typeof c.channel !== 'string' || !num(c.version) || !num(c.fetchedAt)) throw new Error('cache shape');
    return { channel: c.channel, version: c.version as number, fetchedAt: c.fetchedAt as number, snapshot: asSnapshot(c.snapshot) };
  } catch {
    clearSnapshotCache();
    return null;
  }
}

function writeCache(c: CachedSnapshot) {
  try {
    window.localStorage.setItem(SNAPSHOT_STORAGE_KEY, JSON.stringify(c));
  } catch {}
}

export function clearSnapshotCache() {
  try {
    window.localStorage.removeItem(SNAPSHOT_STORAGE_KEY);
  } catch {}
}

const INITIAL: RemoteState = {
  initialized: false,
  snapshot: null,
  version: null,
  fetchedAt: null,
  error: null,
  refreshing: false,
};

class RemoteStore {
  private state: RemoteState = INITIAL;
  private readonly keys: Promise<SyncKeys>;
  private readonly listeners = new Set<() => void>();
  private retainers = 0;
  private timer: ReturnType<typeof setInterval> | undefined;
  private inFlight: Promise<void> | null = null;
  private queued: boolean | null = null; // null = nothing queued, else the queued run's `force`
  private lastAttempt = 0;
  private disposed = false;
  // Server version whose envelope failed to open (bad-snapshot or
  // decrypt-failed), and why: not downloaded again until head moves or a
  // forced refresh. Network errors are not recorded (they keep retrying).
  private failed: { version: number; error: SyncError } | null = null;
  private channel: string | null = null;

  constructor(code: string) {
    this.keys = deriveKeys(code);
    // Swallow here; refresh() reports a bad code as state.error.
    this.keys
      .then((k) => {
        if (this.disposed) return;
        this.channel = k.channel;
        const c = readCache();
        if (c && c.channel === k.channel) {
          this.set({ initialized: true, snapshot: c.snapshot, version: c.version, fetchedAt: c.fetchedAt });
        } else {
          this.set({ initialized: true });
        }
      })
      .catch((e) => this.set({ initialized: true, error: toSyncError(e) }));
  }

  getState = (): RemoteState => this.state;

  subscribe = (fn: () => void): (() => void) => {
    this.listeners.add(fn);
    return () => this.listeners.delete(fn);
  };

  private set(patch: Partial<RemoteState>) {
    this.state = { ...this.state, ...patch };
    for (const fn of this.listeners) fn();
  }

  /** Seed with a snapshot fetched elsewhere (pairing), so it isn't downloaded twice. */
  seed(snapshot: Snapshot, version: number, channel: string) {
    this.set({ initialized: true, snapshot, version, fetchedAt: Date.now(), error: null });
    writeCache({ channel, version, fetchedAt: Date.now(), snapshot });
  }

  /** Start polling while at least one consumer is mounted and enabled. */
  retain(): () => void {
    this.retainers++;
    if (this.retainers === 1) this.start();
    let released = false;
    return () => {
      if (released) return;
      released = true;
      this.retainers--;
      if (this.retainers === 0) this.stop();
    };
  }

  private onWake = () => {
    if (document.visibilityState !== 'visible') return;
    if (Date.now() - this.lastAttempt < MIN_RECHECK_MS) return;
    void this.refresh();
  };

  private start() {
    void this.refresh();
    this.timer = setInterval(() => {
      if (document.visibilityState === 'visible') void this.refresh();
    }, POLL_MS);
    document.addEventListener('visibilitychange', this.onWake);
    window.addEventListener('focus', this.onWake);
    window.addEventListener('online', this.onWake);
    window.addEventListener('pageshow', this.onWake);
  }

  private stop() {
    clearInterval(this.timer);
    this.timer = undefined;
    document.removeEventListener('visibilitychange', this.onWake);
    window.removeEventListener('focus', this.onWake);
    window.removeEventListener('online', this.onWake);
    window.removeEventListener('pageshow', this.onWake);
  }

  dispose() {
    this.disposed = true;
    this.stop();
    this.listeners.clear();
  }

  /** Recovery (the error boundary's "Clear cached copy"): forget the snapshot
   *  on screen and its cache, then fetch it again. */
  clear = () => {
    if (this.disposed) return;
    const c = readCache();
    if (!c || c.channel === this.channel) clearSnapshotCache();
    this.failed = null;
    this.set({ snapshot: null, version: null, fetchedAt: null, error: null });
    void this.refresh(true);
  };

  /** head → (get + open when the version moved, or when forced). Coalesced:
   *  a call while one is running queues exactly one more run. */
  refresh = (force = false): Promise<void> => {
    if (this.inFlight) {
      this.queued = (this.queued ?? false) || force;
      return this.inFlight;
    }
    const run = this.run(force).finally(() => {
      this.inFlight = null;
      const again = this.queued;
      this.queued = null;
      if (again !== null && !this.disposed) void this.refresh(again);
    });
    this.inFlight = run;
    return run;
  };

  private async run(force: boolean): Promise<void> {
    this.lastAttempt = Date.now();
    this.set({ refreshing: true });
    try {
      const keys = await this.keys;
      const h = await head(keys);
      if (h.version === 0) throw new SyncError('empty', 'Nothing has been uploaded yet.');
      if (force || h.version !== this.failed?.version) this.failed = null;
      // The same version already failed to open: show its error again (next to
      // the last good snapshot, which is stale, so fetchedAt stays) and don't
      // download it again.
      if (this.failed) {
        this.set({ error: this.failed.error });
        return;
      }
      const s = this.state;
      if (!force && s.snapshot && s.version === h.version) {
        this.set({ fetchedAt: Date.now(), error: null });
        return;
      }
      const g = await get(keys);
      let snapshot: Snapshot;
      try {
        // Validated here, so nothing unvalidated is shown or cached.
        snapshot = await openEnvelope(g.payload, keys);
      } catch (e) {
        this.failed = { version: g.version, error: toSyncError(e) };
        throw e;
      }
      if (this.disposed) return;
      const now = Date.now();
      this.set({ snapshot, version: g.version, fetchedAt: now, error: null });
      writeCache({ channel: keys.channel, version: g.version, fetchedAt: now, snapshot });
    } catch (e) {
      if (!this.disposed) this.set({ error: toSyncError(e) });
    } finally {
      if (!this.disposed) this.set({ refreshing: false, initialized: true });
    }
  }
}

function toSyncError(e: unknown): SyncError {
  if (isSyncError(e)) return e;
  return new SyncError('network', e instanceof Error ? e.message : String(e));
}

const stores = new Map<string, RemoteStore>();

function storeFor(code: string): RemoteStore {
  let s = stores.get(code);
  if (!s) {
    s = new RemoteStore(code);
    stores.set(code, s);
  }
  return s;
}

/** Hand a freshly paired code's first snapshot to its store. */
export function seedRemote(code: string, channel: string, snapshot: Snapshot, version: number) {
  storeFor(code).seed(snapshot, version, channel);
}

/** Unpair: drop the store and the cached snapshot. */
export function forgetRemote(code: string | null) {
  if (code) {
    stores.get(code)?.dispose();
    stores.delete(code);
  }
  clearSnapshotCache();
}

const NO_STATE: RemoteState = { ...INITIAL, initialized: true };
const noSubscribe = () => () => {};
const noState = () => NO_STATE;

/** The remote snapshot for `code`; polls only while `enabled`. */
export function useRemoteSnapshot(code: string | null, enabled: boolean): RemoteView {
  const store = code ? storeFor(code) : null;
  const state = useSyncExternalStore(store ? store.subscribe : noSubscribe, store ? store.getState : noState);
  useEffect(() => {
    if (!store || !enabled) return;
    return store.retain();
  }, [store, enabled]);
  const refresh = useCallback((force?: boolean) => (store ? store.refresh(force) : Promise.resolve()), [store]);
  const clear = useCallback(() => store?.clear(), [store]);
  return { ...state, refresh, clear };
}

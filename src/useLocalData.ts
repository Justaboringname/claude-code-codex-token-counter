// This Mac's own numbers: the Rust scan commands (or the browser-preview
// samples). Moved out of PopoverContent unchanged apart from `enabled`, which
// is false while a sync viewer shows the main Mac's snapshot instead — then
// nothing here polls, scans, listens or warms up, and nothing remote is ever
// written into this cache.
import { useEffect, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { sampleClaudeQuota, sampleCodexQuota, sampleUsage } from './sampleData';
import type { ClaudeQuota, CodexLiveQuota, PlanId, Range, Usage } from './types';

const USAGE_CACHE_KEY = 'usage-cache-v1';

function readCached(r: Range): Usage | null {
  try {
    const raw = window.localStorage.getItem(`${USAGE_CACHE_KEY}:${r}`);
    return raw ? (JSON.parse(raw) as Usage) : null;
  } catch {
    return null;
  }
}

function writeCached(r: Range, u: Usage) {
  try {
    window.localStorage.setItem(`${USAGE_CACHE_KEY}:${r}`, JSON.stringify(u));
    window.localStorage.setItem(`${USAGE_CACHE_KEY}:${r}:at`, String(Date.now()));
  } catch {}
}

// The desktop window is hidden (WebKit reports an ordered-out window as a
// hidden page). Always false in a browser tab that is on screen.
function isHidden(): boolean {
  return typeof document !== 'undefined' && document.visibilityState === 'hidden';
}

// When a range's cached payload was written (ms epoch). 0 = unknown (caches
// written before this stamp existed).
function readCachedAt(r: Range): number {
  try {
    return Number(window.localStorage.getItem(`${USAGE_CACHE_KEY}:${r}:at`)) || 0;
  } catch {
    return 0;
  }
}

export interface LocalData {
  usage: Usage | null;
  cachedAt: number | null;
  claudeQuota: ClaudeQuota | null;
  codexQuota: CodexLiveQuota | null;
  /** Call in the same handler as setRange (shows the range's cache at once). */
  pickRange: (r: Range) => void;
  refreshAll: () => void;
}

export function useLocalData(range: Range, plan: PlanId, enabled: boolean): LocalData {
  const [usage, setUsage] = useState<Usage | null>(() => (enabled ? readCached(range) : null));
  // Set while the screen shows a cached payload (from an earlier scan, possibly
  // an earlier app run) and no fresh scan of the current range has landed yet:
  // the cache's write time, 0 if unknown. A full rescan can take a while, and
  // an old number shown at full strength looks current (2026-11-07: a Week
  // payload from the night before was read as a miscount).
  const [cachedAt, setCachedAt] = useState<number | null>(() =>
    enabled && readCached(range) ? readCachedAt(range) : null,
  );
  const [claudeQuota, setClaudeQuota] = useState<ClaudeQuota | null>(null);
  const [codexQuota, setCodexQuota] = useState<CodexLiveQuota | null>(null);
  const lastDisplayedJson = useRef<string>('');

  const applyIfChanged = (u: Usage) => {
    const j = JSON.stringify(u);
    if (j === lastDisplayedJson.current) return;
    lastDisplayedJson.current = j;
    setUsage(u);
  };

  // Load + cache usage for a range. Never touches the UI — callers decide
  // whether to render. Resolves to undefined on error.
  const loadUsage = (r: Range): Promise<Usage | undefined> => {
    const isTauri = typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window;
    // Browser preview only: ?usage=slow delays the sample like a long rescan,
    // so the cached "Updating…" state stays visible.
    const slow = !isTauri && new URLSearchParams(window.location.search).get('usage') === 'slow';
    const promise = isTauri
      ? invoke<Usage>('get_usage', { range: r, plan })
      : new Promise<Usage>((res) => setTimeout(() => res(sampleUsage(r, plan)), slow ? 5000 : 0));
    return promise
      .then((u) => {
        writeCached(r, u);
        return u;
      })
      .catch((e) => {
        console.error('get_usage failed', e);
        return undefined;
      });
  };

  const loadClaudeQuota = (force = false): Promise<void> => {
    const isTauri = typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window;
    const promise = isTauri
      ? invoke<ClaudeQuota>('get_claude_quota', { force })
      : Promise.resolve(sampleClaudeQuota());
    return promise
      .then(setClaudeQuota)
      .catch((e) => console.error('get_claude_quota failed', e));
  };

  const loadCodexQuota = (force = false): Promise<void> => {
    const isTauri = typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window;
    const promise = isTauri
      ? invoke<CodexLiveQuota>('get_codex_quota', { force })
      : Promise.resolve(sampleCodexQuota());
    return promise
      .then(setCodexQuota)
      .catch((e) => console.error('get_codex_quota failed', e));
  };

  // The active range is refreshed from several racing sources: the 15s timer,
  // the file-watcher `usage-changed` event (fires repeatedly while Claude Code
  // writes), manual refresh, and range switches. A full rescan reads JSONL that
  // is being appended live, so two overlapping scans see different snapshots and
  // can resolve out of order — an older, smaller scan landing last would make
  // the headline number jump backwards ($99 → $94 → $99). Two guards:
  //   1. coalesce — at most one active fetch in flight; if asked again while
  //      busy, run exactly one more afterwards (against the latest range) so a
  //      burst of events can't pile up dozens of redundant rescans.
  //   2. sequence — defense-in-depth: only the most recently *started* active
  //      fetch may render. With (1) in place this is dormant (no second fetch
  //      ever starts mid-flight), but it stops a late stale scan from winning if
  //      the coalescing in (1) is ever relaxed. The range check below does the
  //      real work today, on range switches.
  const reqSeq = useRef(0);
  const activeInFlight = useRef(false);
  const activePending = useRef(false);
  const rangeRef = useRef<Range>(range);
  rangeRef.current = range;
  const enabledRef = useRef(enabled);
  enabledRef.current = enabled;
  // Hidden window (closed to the background, or launched at login with no
  // window): nothing on screen needs fresh numbers, and sync uploads from Rust
  // on its own clock. Polls and file-change refreshes are skipped while hidden,
  // and one catch-up runs when the window shows again.
  const missedWhileHidden = useRef(false);

  const refreshActive = () => {
    if (isHidden()) {
      missedWhileHidden.current = true;
      return;
    }
    if (activeInFlight.current) {
      activePending.current = true;
      return;
    }
    activeInFlight.current = true;
    const myId = ++reqSeq.current;
    const r = rangeRef.current;
    loadUsage(r)
      .then((u) => {
        if (u !== undefined && myId === reqSeq.current && r === rangeRef.current) {
          applyIfChanged(u);
          setCachedAt(null);
        }
      })
      .finally(() => {
        activeInFlight.current = false;
        if (activePending.current) {
          activePending.current = false;
          // Not after a switch to the remote source.
          if (enabledRef.current) refreshActive();
        }
      });
  };

  const showCachedRange = (r: Range) => {
    const cached = readCached(r);
    if (cached) applyIfChanged(cached);
    // Cached or (no cache) still the previous range's numbers: either way not
    // yet a fresh scan of `r`.
    setCachedAt(cached ? readCachedAt(r) : 0);
  };

  const pickRange = (r: Range) => {
    if (enabledRef.current) showCachedRange(r);
  };

  // Switching back to this Mac (viewer toggle): show its cache for the
  // current range until the scan the effect below starts lands.
  const wasEnabled = useRef(enabled);
  useEffect(() => {
    if (enabled && !wasEnabled.current) showCachedRange(rangeRef.current);
    wasEnabled.current = enabled;
  }, [enabled]);

  useEffect(() => {
    if (!enabled) return;
    refreshActive();
    const timer = setInterval(refreshActive, 15000);
    return () => clearInterval(timer);
  }, [range, plan, enabled]);

  useEffect(() => {
    if (!enabled) return;
    const loadQuotas = () => {
      if (isHidden()) {
        missedWhileHidden.current = true;
        return;
      }
      void loadClaudeQuota();
      void loadCodexQuota();
    };
    loadQuotas();
    const timer = setInterval(loadQuotas, 60000);
    return () => clearInterval(timer);
  }, [enabled]);

  // The catch-up after a hidden spell (see missedWhileHidden).
  useEffect(() => {
    if (!enabled) return;
    const onVisibility = () => {
      if (isHidden() || !missedWhileHidden.current) return;
      missedWhileHidden.current = false;
      refreshActive();
      void loadClaudeQuota();
      void loadCodexQuota();
    };
    document.addEventListener('visibilitychange', onVisibility);
    return () => document.removeEventListener('visibilitychange', onVisibility);
  }, [range, plan, enabled]);

  useEffect(() => {
    const isTauri = typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window;
    if (!isTauri || !enabled) return;
    const unlistenPromise = listen('usage-changed', () => {
      refreshActive();
    });
    return () => {
      unlistenPromise.then((un) => un()).catch(() => {});
    };
  }, [range, plan, enabled]);

  // Warm the other ranges' caches once, shortly after this Mac's data is
  // first shown (at launch, unless a sync viewer starts on the main Mac).
  const warmedUp = useRef(false);
  useEffect(() => {
    if (!enabled || warmedUp.current) return;
    let cancelled = false;
    const queue: Range[] = ['today', 'week', 'month', 'session', 'all'];
    const shown = rangeRef.current;
    (async () => {
      await new Promise((r) => setTimeout(r, 1500));
      for (const r of queue) {
        if (cancelled || r === shown) continue;
        await loadUsage(r);
        await new Promise((res) => setTimeout(res, 300));
      }
      if (!cancelled) warmedUp.current = true;
    })();
    return () => {
      cancelled = true;
    };
  }, [enabled]);

  const refreshAll = () => {
    refreshActive();
    void loadClaudeQuota(true);
    void loadCodexQuota(true);
  };

  return { usage, cachedAt, claudeQuota, codexQuota, pickRange, refreshAll };
}

import React, { useCallback, useEffect, useMemo, useState } from 'react';
import { Icon, iconBySemantic } from './icons';
import { Sparkline, SparkSeries } from './Sparkline';
import { ModelUsage, costForModel, displayToolName, fmtMoney, fmtTokens, liftForDark, modelMeta, toolColor } from './format';
import { Theme, ThemeMode, darkTheme, lightTheme, useDarkMode } from './theme';
import { GlassSurface } from './GlassSurface';
import type { LimitWindow, PlanId, PriceHealth, Range, Snapshot, SyncStatus, Usage } from './types';
import { appEnv, previewWantsRemote } from './env';
import { useLocalData } from './useLocalData';
import { readViewSource, useDesktopSync, writeViewSource, type ViewSource } from './sync/desktop';
import { useRemoteSnapshot, type RemoteView } from './sync/remote';
import { useWebPairing } from './sync/pairing';
import { DesktopSyncSheet, WebSyncSheet } from './sync/SyncSheet';
import { PairingScreen } from './sync/PairingScreen';
import { describeError } from './sync/describe';
import { relativeAge, useNow } from './sync/time';
import { Button, SHEET_TOP, Spinner } from './sync/ui';

const PLAN_STORAGE_KEY = 'plan-id-v1';

function readPlan(): PlanId {
  try {
    const raw = window.localStorage.getItem(PLAN_STORAGE_KEY);
    if (raw === 'pro' || raw === 'max5x' || raw === 'max20') return raw;
  } catch {}
  return 'max5x';
}

const CANONICAL_TOOLS = ['Read', 'Edit', 'Bash', 'Grep', 'WebFetch', 'Glob'];

const RANGES: { id: Range; label: string }[] = [
  { id: 'session', label: 'Session' },
  { id: 'today', label: 'Today' },
  { id: 'week', label: 'Week' },
  { id: 'month', label: 'Month' },
  { id: 'all', label: 'All' },
];

// A snapshot older than this reads as "the main Mac stopped syncing" (the
// primary re-uploads at least every 10 min while it runs).
const STALE_SNAPSHOT_MS = 15 * 60_000;

// Data source of the panel:
//   local  — this Mac's scans (sync off / primary / a viewer's "This Mac")
//   remote — the main Mac's snapshot (a viewer by default; the phone web app)
type Source = 'local' | 'remote';

export function PopoverContent() {
  const env = appEnv();
  const web = env === 'web';
  // Pairing happens in the browser for the web app, and in plain `vite dev`
  // with ?source=remote; the desktop app keeps its code in Rust.
  const pairsInBrowser = web || (env === 'preview' && previewWantsRemote());

  const [range, setRange] = useState<Range>('today');
  const [plan] = useState<PlanId>(() => readPlan());
  const { isDark, mode, cycleMode } = useDarkMode();
  const t: Theme = isDark ? darkTheme : lightTheme;
  const sparkColor = isDark ? '#E29A78' : '#B8532F';
  const claudeLimitColor = isDark ? '#E29A78' : '#B8532F';
  // Codex overlay line — provider blue, lifted for contrast in dark mode.
  const codexSparkColor = isDark ? '#7FB3E3' : '#437FBD';

  const desktop = useDesktopSync(env === 'tauri');
  const pairing = useWebPairing(pairsInBrowser);
  const isViewer = env === 'tauri' && desktop.role === 'viewer';
  const [viewSource, setViewSourceState] = useState<ViewSource>(() => readViewSource());
  const setViewSource = (v: ViewSource) => {
    writeViewSource(v);
    setViewSourceState(v);
  };
  const source: Source = pairsInBrowser ? 'remote' : isViewer ? viewSource : 'local';
  const remoteCode = pairsInBrowser ? pairing.code : isViewer ? (desktop.config?.code ?? null) : null;
  const remote = useRemoteSnapshot(remoteCode, source === 'remote');
  const local = useLocalData(range, plan, source === 'local');

  const snapshot = source === 'remote' ? remote.snapshot : null;
  const usage: Usage | null = source === 'remote' ? (snapshot?.usage?.[range] ?? null) : local.usage;
  const claudeQuota = source === 'remote' ? (snapshot?.claudeQuota ?? null) : local.claudeQuota;
  const codexQuota = source === 'remote' ? (snapshot?.codexQuota ?? null) : local.codexQuota;
  const cachedAt = source === 'remote' ? null : local.cachedAt;
  const remoteName = remote.snapshot?.device.name ?? null;

  const [sheetOpen, setSheetOpen] = useState(false);
  const [sheetHeight, setSheetHeight] = useState(0);
  const closeSheet = useCallback(() => setSheetOpen(false), []);
  // The sheet needs a backend (desktop) or a pairing (browser).
  const canOpenSheet = env === 'tauri' ? desktop.available && desktop.config !== null : pairsInBrowser && !!pairing.code;
  useEffect(() => {
    if (!canOpenSheet) setSheetOpen(false);
  }, [canOpenSheet]);

  const pickRange = (r: Range) => {
    setRange(r);
    // Remote: instant, straight from the snapshot. Local: show the range's
    // cache at once (a no-op while the local source is off).
    local.pickRange(r);
  };

  const refreshAll = () => {
    if (source === 'remote') void remote.refresh(true);
    else local.refreshAll();
  };

  const models = useMemo(() => {
    if (!usage) return [];
    return Object.entries(usage.byModel ?? {})
      .map(([id, u]) => {
        const meta = modelMeta(id);
        return {
          id,
          ...meta,
          color: isDark ? liftForDark(meta.color) : meta.color,
          usage: u,
          cost: costForModel(id, u),
        };
      })
      .filter((m) => m.cost > 0)
      .sort((a, b) => b.cost - a.cost)
      .slice(0, 10);
  }, [usage, isDark]);

  // Claude (orange) + Codex (blue) share one y-scale. Codex renders FIRST so
  // the Claude line stays on top; the overlay is omitted when the range has
  // no Codex spend, keeping the classic single-line look.
  const sparkSeries = useMemo(() => {
    const claude = { data: usage?.sparkline ?? new Array(14).fill(0), color: sparkColor };
    const codex = usage?.sparklineCodex;
    return codex?.some((v) => v > 0)
      ? [{ data: codex, color: codexSparkColor }, claude]
      : [claude];
  }, [usage, sparkColor, codexSparkColor]);

  const toolTiles = useMemo(() => {
    const used = [...(usage?.byTool ?? [])].sort((a, b) => b.calls - a.calls);
    const usedNames = new Set(used.map((t) => t.name));
    const filler = CANONICAL_TOOLS.filter((n) => !usedNames.has(n)).map((n) => ({
      name: n,
      calls: 0,
      tokens: 0,
    }));
    return [...used, ...filler].slice(0, 6);
  }, [usage]);

  const totalToolCalls = toolTiles.reduce((s, tool) => s + tool.calls, 0);

  const codexLive = codexQuota && (codexQuota.fiveHour || codexQuota.weekly) ? codexQuota : null;
  const codexLog = usage?.codexQuota ?? null;

  const cloud = canOpenSheet
    ? cloudState(env === 'tauri' ? desktop.role : 'viewer', desktop.status, source === 'remote' ? remote : null, () =>
        setSheetOpen(true),
      )
    : undefined;
  const refreshing = source === 'remote' && remote.refreshing;

  // Remote, but nothing to show yet (first connect, empty channel, error).
  const remotePlaceholder =
    source === 'remote' && !remote.snapshot ? (
      <RemotePlaceholder t={t} web={web} remote={remote} hasCode={!!remoteCode} onOpenSheet={cloud?.onClick} />
    ) : null;

  const body = (
    <>
      <RangeBar theme={t} range={range} onPick={pickRange} web={web} />
      {remotePlaceholder ?? (
        <>
          <Hero
            theme={t}
            usage={usage}
            cachedAt={cachedAt}
            web={web}
            footer={
              source === 'remote' && !web && snapshot ? (
                <Freshness t={t} snapshot={snapshot} remote={remote} />
              ) : null
            }
          />
          <SparkRow
            theme={t}
            series={sparkSeries}
            leftLabel={usage?.sparkLeft ?? ''}
            rightLabel={usage?.sparkRight ?? ''}
            web={web}
          />
          <ByModel
            theme={t}
            models={models}
            totalCost={usage?.totalCost ?? 0}
            priceHealth={usage?.priceHealth}
            web={web}
          />
          {claudeQuota && (claudeQuota.fiveHour || claudeQuota.weekly) && (
            <LimitsRow
              theme={t}
              web={web}
              title="Claude limits"
              fivePct={liveIsoPct(claudeQuota.fiveHour)}
              weeklyPct={liveIsoPct(claudeQuota.weekly)}
              barColor={claudeLimitColor}
              staleAsOf={claudeQuota.status !== 'ok' ? claudeQuota.fetchedAt : undefined}
              tooltip={`Official Claude limits${claudeQuota.subscription ? ` (${claudeQuota.subscription})` : ''} — as of ${new Date(claudeQuota.fetchedAt).toLocaleString()}${claudeQuota.status !== 'ok' ? ` — stale${claudeQuota.message ? `: ${claudeQuota.message}` : ''}` : ''}`}
            />
          )}
          {codexLive ? (
            <LimitsRow
              theme={t}
              web={web}
              title="Codex limits"
              fivePct={liveIsoPct(codexLive.fiveHour)}
              weeklyPct={liveIsoPct(codexLive.weekly)}
              barColor={codexSparkColor}
              staleAsOf={codexLive.status !== 'ok' ? codexLive.fetchedAt : undefined}
              tooltip={`Official Codex limits${codexLive.planType ? ` (${codexLive.planType} plan)` : ''} — as of ${new Date(codexLive.fetchedAt).toLocaleString()}${codexLive.resetCredits ? ` — ${codexLive.resetCredits} reset credit${codexLive.resetCredits === 1 ? '' : 's'} available` : ''}${codexLive.status !== 'ok' ? ` — stale${codexLive.message ? `: ${codexLive.message}` : ''}` : ''}`}
            />
          ) : (
            codexLog && (
              <LimitsRow
                theme={t}
                web={web}
                title="Codex limits"
                fivePct={codexLog.primaryWindowMinutes ? liveEpochPct(codexLog.primaryResetsAt, codexLog.primaryUsedPct) : null}
                weeklyPct={codexLog.secondaryWindowMinutes ? liveEpochPct(codexLog.secondaryResetsAt, codexLog.secondaryUsedPct) : null}
                barColor={codexSparkColor}
                tooltip={`Codex limits from session logs${codexLog.planType ? ` (${codexLog.planType} plan)` : ''} — as of ${new Date(codexLog.asOf).toLocaleString()}${codexQuota?.message ? ` — live read failed: ${codexQuota.message}` : ''}`}
              />
            )
          )}
          <ToolCalls theme={t} tiles={toolTiles} total={totalToolCalls} web={web} />
        </>
      )}
    </>
  );

  // ---- phone web app ----
  if (web || pairsInBrowser) {
    if (!pairing.code) {
      return (
        <WebShell t={t} web={web}>
          <PairingScreen t={t} isDark={isDark} pairing={pairing} />
        </WebShell>
      );
    }
    if (web) {
      return (
        <WebShell t={t} web>
          <WebHeader
            t={t}
            mode={mode}
            isDark={isDark}
            onCycleTheme={cycleMode}
            onRefresh={refreshAll}
            refreshing={refreshing}
            cloud={cloud}
            freshness={snapshot ? <Freshness t={t} snapshot={snapshot} remote={remote} onWallpaper /> : null}
          />
          <GlassSurface theme={t} layout="web">
            {body}
          </GlassSurface>
          {sheetOpen && <WebSyncSheet t={t} remote={remote} onUnpair={pairing.unpair} onClose={closeSheet} />}
        </WebShell>
      );
    }
  }

  // ---- desktop panel (Tauri, or the browser preview) ----
  return (
    <GlassSurface theme={t} minHeight={sheetOpen && sheetHeight ? sheetHeight + SHEET_TOP + 10 : undefined}>
      <TitleBar
        theme={t}
        mode={mode}
        isDark={isDark}
        onCycleTheme={cycleMode}
        onRefresh={refreshAll}
        refreshing={refreshing}
        cloud={cloud}
        sourceToggle={
          isViewer ? (
            <SourceToggle t={t} value={viewSource} remoteName={remoteName} onChange={setViewSource} />
          ) : undefined
        }
      />
      {body}
      {sheetOpen && env === 'tauri' && desktop.config && (
        <DesktopSyncSheet t={t} sync={desktop} remote={remote} onClose={closeSheet} onMeasure={setSheetHeight} />
      )}
      {sheetOpen && env !== 'tauri' && pairsInBrowser && (
        <WebSyncSheet t={t} remote={remote} onUnpair={pairing.unpair} onClose={closeSheet} />
      )}
    </GlassSurface>
  );
}

// ------------------------------------------------------------ sync chrome

interface CloudProps {
  glyph?: 'up' | 'down';
  /** Spinner-ish pulse while an upload/check runs. */
  busy: boolean;
  /** Amber badge: the last upload or check failed. */
  alert: boolean;
  tooltip: string;
  onClick: () => void;
}

function cloudState(
  role: 'off' | 'primary' | 'viewer',
  status: SyncStatus | null,
  remote: RemoteView | null,
  onClick: () => void,
): CloudProps {
  if (role === 'primary') {
    const up = status?.lastUploadAt ? Date.parse(status.lastUploadAt) : NaN;
    const errAt = status?.lastErrorAt ? Date.parse(status.lastErrorAt) : NaN;
    const failing = !!status?.lastError && (!Number.isFinite(up) || errAt >= up);
    return {
      glyph: 'up',
      busy: !!status?.uploading,
      alert: failing,
      tooltip: failing
        ? `Sync: last upload failed — ${status?.lastError}`
        : Number.isFinite(up)
          ? `Sync: this Mac uploads · last upload ${relativeAge(up)}`
          : 'Sync: this Mac uploads',
      onClick,
    };
  }
  if (role === 'viewer') {
    const err = remote?.error ? describeError(remote.error, remote.snapshot?.device.name) : null;
    return {
      glyph: 'down',
      busy: !!remote?.refreshing,
      alert: !!err,
      tooltip: err ? `Sync: ${err.long}` : 'Sync: showing another Mac',
      onClick,
    };
  }
  return { busy: false, alert: false, tooltip: 'Sync: off', onClick };
}

function CloudButton({ t, cloud, size, chip }: { t: Theme; cloud: CloudProps; size: number; chip?: boolean }) {
  return (
    <button
      onClick={cloud.onClick}
      style={{
        ...iconBtn(t),
        width: size,
        height: size,
        position: 'relative',
        ...(chip ? { backdropFilter: 'blur(20px) saturate(160%)', WebkitBackdropFilter: 'blur(20px) saturate(160%)' } : {}),
      }}
      aria-label="Sync"
      title={cloud.tooltip}
    >
      <span className={cloud.busy ? 'tc-pulse' : undefined} style={{ display: 'flex' }}>
        <Icon.Cloud size={size > 30 ? 18 : 14} glyph={cloud.glyph} />
      </span>
      {cloud.alert && (
        <span
          style={{
            position: 'absolute',
            top: size > 30 ? 5 : 2,
            right: size > 30 ? 5 : 2,
            width: size > 30 ? 9 : 7,
            height: size > 30 ? 9 : 7,
            borderRadius: 999,
            background: t.warn,
            boxShadow: '0 0 0 1.5px rgba(255,255,255,0.7)',
          }}
        />
      )}
    </button>
  );
}

// Viewer's title-bar switch: the main Mac's snapshot | this Mac's own scans.
function SourceToggle({
  t,
  value,
  remoteName,
  onChange,
}: {
  t: Theme;
  value: ViewSource;
  remoteName: string | null;
  onChange: (v: ViewSource) => void;
}) {
  const seg = (v: ViewSource, label: string) => {
    const active = v === value;
    return (
      <button
        key={v}
        onClick={() => onChange(v)}
        aria-pressed={active}
        style={{
          padding: '3px 9px',
          fontSize: 11,
          fontWeight: active ? 600 : 500,
          color: active ? t.ink : t.inkMute,
          background: active ? t.segThumbBg : 'transparent',
          border: 'none',
          borderRadius: 999,
          boxShadow: active ? t.segThumbShadow : 'none',
          cursor: 'pointer',
          fontFamily: 'inherit',
          whiteSpace: 'nowrap',
          maxWidth: 120,
          overflow: 'hidden',
          textOverflow: 'ellipsis',
        }}
      >
        {label}
      </button>
    );
  };
  return (
    <div
      role="group"
      aria-label="Show numbers from"
      style={{ display: 'flex', padding: 2, background: t.segBg, borderRadius: 999, boxShadow: t.segInset, minWidth: 0 }}
    >
      {seg('remote', remoteName ?? 'Main Mac')}
      {seg('local', 'This Mac')}
    </div>
  );
}

// "From Mac Studio · updated 2 min ago" — amber once the main Mac has been
// quiet for 15 min. Ticks on its own.
function Freshness({
  t,
  snapshot,
  remote,
  onWallpaper,
}: {
  t: Theme;
  snapshot: Snapshot;
  remote: RemoteView;
  onWallpaper?: boolean;
}) {
  const now = useNow(15_000);
  const gen = Date.parse(snapshot.generatedAt);
  const name = snapshot.device.name;
  const stale = Number.isFinite(gen) && now - gen > STALE_SNAPSHOT_MS;
  const age = Number.isFinite(gen) ? relativeAge(gen, now) : 'at an unknown time';
  const err = remote.error && remote.error.kind !== 'empty' ? describeError(remote.error, name) : null;
  const soft = onWallpaper ? t.webInkSoft : t.inkSoft;
  const warn = onWallpaper ? (t === darkTheme ? t.warn : '#FFE2B8') : t.warn;
  return (
    <div
      title={err ? err.long : `Snapshot built ${new Date(gen).toLocaleString()}`}
      style={{
        display: 'flex',
        alignItems: 'center',
        gap: 6,
        fontSize: onWallpaper ? 15 : 12,
        color: stale || err ? warn : soft,
        marginTop: onWallpaper ? 6 : 8,
        minWidth: 0,
      }}
    >
      {(stale || err) && <Icon.Warn size={onWallpaper ? 12 : 11} color={warn} />}
      <span style={{ overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>
        {stale ? `${name} last synced ${age}` : `From ${name} · updated ${age}`}
        {err ? ` · ${err.short}` : ''}
      </span>
    </div>
  );
}

function RemotePlaceholder({
  t,
  web,
  remote,
  hasCode,
  onOpenSheet,
}: {
  t: Theme;
  web: boolean;
  remote: RemoteView;
  hasCode: boolean;
  onOpenSheet?: () => void;
}) {
  const err = remote.error ? describeError(remote.error) : null;
  const connecting = hasCode && (!remote.initialized || remote.refreshing || (!err && !remote.fetchedAt));
  // Before the cached snapshot was checked (a few ms): draw nothing rather
  // than flash a "Connecting" state.
  if (hasCode && !remote.initialized) return <div style={{ height: web ? 420 : 360 }} />;
  const title = !hasCode
    ? 'No pairing code'
    : connecting
      ? 'Connecting…'
      : remote.error?.kind === 'empty'
        ? 'Waiting for the main Mac'
        : (err?.short ?? 'No data yet');
  const text = !hasCode
    ? 'Open Sync and paste the pairing code of your main Mac.'
    : connecting
      ? 'Fetching the latest snapshot.'
      : (err?.long ?? 'Nothing has arrived yet.');
  return (
    <div
      style={{
        padding: web ? '64px 28px 72px' : '56px 28px 64px',
        display: 'flex',
        flexDirection: 'column',
        alignItems: 'center',
        textAlign: 'center',
        gap: 8,
        color: t.inkSoft,
      }}
    >
      <span style={{ color: t.inkMute, marginBottom: 4 }}>
        {connecting ? <Spinner size={web ? 26 : 22} /> : <Icon.Cloud size={web ? 30 : 26} glyph="down" />}
      </span>
      <div style={{ fontSize: web ? 18 : 14, fontWeight: 600, color: t.ink }}>{title}</div>
      <div style={{ fontSize: web ? 15 : 12, lineHeight: 1.45, maxWidth: 300 }}>{text}</div>
      {!connecting && hasCode && (
        <Button t={t} layout={web ? 'web' : 'desktop'} onClick={() => void remote.refresh(true)} style={{ marginTop: 10 }}>
          Try again
        </Button>
      )}
      {!hasCode && onOpenSheet && (
        <Button t={t} layout={web ? 'web' : 'desktop'} onClick={onOpenSheet} style={{ marginTop: 10 }}>
          Open Sync
        </Button>
      )}
    </div>
  );
}

// ------------------------------------------------------------ web shell

// Phone web app frame: the wallpaper (fixed, under the status bar too), safe
// areas, a centred column up to 560 px.
function WebShell({ t, web, children }: { t: Theme; web: boolean; children: React.ReactNode }) {
  useEffect(() => {
    if (!web) return;
    // Overscroll bounce and the area behind the translucent status bar.
    document.documentElement.style.background = t.webBase;
    document.body.style.background = t.webBase;
  }, [web, t]);
  return (
    <div style={{ position: 'relative', minHeight: '100dvh' }}>
      {web && <div aria-hidden style={{ position: 'fixed', inset: 0, background: t.webWallpaper, zIndex: 0 }} />}
      <div
        style={{
          position: 'relative',
          zIndex: 1,
          maxWidth: 560,
          margin: '0 auto',
          boxSizing: 'border-box',
          padding: web
            ? 'calc(env(safe-area-inset-top) + 6px) calc(env(safe-area-inset-right) + 12px) calc(env(safe-area-inset-bottom) + 28px) calc(env(safe-area-inset-left) + 12px)'
            : 0,
        }}
      >
        {children}
      </div>
    </div>
  );
}

function WebHeader({
  t,
  mode,
  isDark,
  onCycleTheme,
  onRefresh,
  refreshing,
  cloud,
  freshness,
}: {
  t: Theme;
  mode: ThemeMode;
  isDark: boolean;
  onCycleTheme: () => void;
  onRefresh: () => void;
  refreshing: boolean;
  cloud?: CloudProps;
  freshness: React.ReactNode;
}) {
  const chip: React.CSSProperties = {
    ...iconBtn(t),
    width: 38,
    height: 38,
    background: t.webChipBg,
    boxShadow: t.webChipShadow,
    color: t.webInk,
    backdropFilter: 'blur(20px) saturate(160%)',
    WebkitBackdropFilter: 'blur(20px) saturate(160%)',
  };
  return (
    <header style={{ padding: '10px 6px 14px' }}>
      <div style={{ display: 'flex', alignItems: 'center', gap: 10 }}>
        <h1
          style={{
            margin: 0,
            minWidth: 0,
            fontSize: 30,
            lineHeight: 1.12,
            whiteSpace: 'nowrap',
            fontWeight: 700,
            letterSpacing: '-0.02em',
            color: t.webInk,
            textShadow: isDark ? 'none' : '0 1px 12px rgba(80, 30, 10, 0.18)',
          }}
        >
          Token Counter
        </h1>
      <div style={{ flex: 1 }} />
      <div style={{ display: 'flex', gap: 8, color: t.webInk }}>
        {cloud && <CloudButton t={{ ...t, btnBg: t.webChipBg, btnShadow: t.webChipShadow }} cloud={cloud} size={38} chip />}
        <button onClick={onRefresh} style={chip} aria-label="Refresh" disabled={refreshing}>
          <span className={refreshing ? 'tc-spin' : undefined} style={{ display: 'flex' }}>
            <Icon.Refresh size={16} />
          </span>
        </button>
        <button onClick={onCycleTheme} style={chip} aria-label={`Theme: ${mode}`}>
          {mode === 'auto' ? <Icon.Auto size={17} /> : mode === 'light' ? <Icon.Sun size={17} /> : <Icon.Moon size={17} />}
        </button>
      </div>
      </div>
      {freshness}
    </header>
  );
}

// ------------------------------------------------------------ panel

function TitleBar({
  theme: t,
  mode,
  isDark,
  onCycleTheme,
  onRefresh,
  refreshing,
  cloud,
  sourceToggle,
}: {
  theme: Theme;
  mode: ThemeMode;
  isDark: boolean;
  onCycleTheme: () => void;
  onRefresh: () => void;
  refreshing?: boolean;
  cloud?: CloudProps;
  sourceToggle?: React.ReactNode;
}) {
  return (
    <div
      data-tauri-drag-region
      style={{
        display: 'flex',
        alignItems: 'center',
        padding: '10px 14px 10px 78px',
        borderBottom: `0.5px solid ${t.hairline}`,
        fontSize: 12,
        color: t.inkStrong,
      }}
    >
      <div
        data-tauri-drag-region
        style={{
          display: 'flex',
          alignItems: 'center',
          gap: 6,
          pointerEvents: 'none',
          // Viewer: the source toggle takes the subtitle's place.
          ...(sourceToggle ? { minWidth: 0, overflow: 'hidden', whiteSpace: 'nowrap' } : {}),
        }}
      >
        <span style={{ fontWeight: 600, color: t.ink }}>Token Counter</span>
        {!sourceToggle && <span style={{ color: t.inkFaint }}>· Claude + Codex</span>}
      </div>
      {sourceToggle && <div style={{ marginLeft: 10, display: 'flex', minWidth: 0 }}>{sourceToggle}</div>}
      <div data-tauri-drag-region style={{ flex: 1 }} />
      <div style={{ display: 'flex', alignItems: 'center', gap: 8, color: t.inkSoft }}>
        {cloud && <CloudButton t={t} cloud={cloud} size={26} />}
        <button onClick={onRefresh} style={iconBtn(t)} aria-label="Refresh">
          <span className={refreshing ? 'tc-spin' : undefined} style={{ display: 'flex' }}>
            <Icon.Refresh size={12} />
          </span>
        </button>
        <button
          onClick={onCycleTheme}
          style={iconBtn(t)}
          aria-label={`Theme: ${mode}`}
          title={
            mode === 'auto'
              ? `Auto theme — following macOS (${isDark ? 'dark' : 'light'} now, click for Light)`
              : mode === 'light'
                ? 'Light theme (click for Dark)'
                : 'Dark theme (click for Auto)'
          }
        >
          {mode === 'auto' ? (
            <Icon.Auto size={13} />
          ) : mode === 'light' ? (
            <Icon.Sun size={13} />
          ) : (
            <Icon.Moon size={13} />
          )}
        </button>
      </div>
    </div>
  );
}

function RangeBar({
  theme: t,
  range,
  onPick,
  web,
}: {
  theme: Theme;
  range: Range;
  onPick: (r: Range) => void;
  web?: boolean;
}) {
  // Segmented control, Liquid Glass style: a shallow capsule trough recessed
  // into the panel, with the active segment as a floating glass thumb.
  return (
    <div style={{ padding: web ? '14px 14px 0' : '12px 14px 0' }}>
      <div
        style={{
          display: 'flex',
          padding: 3,
          background: t.segBg,
          borderRadius: 999,
          boxShadow: t.segInset,
        }}
      >
        {RANGES.map((r) => {
          const active = r.id === range;
          return (
            <button
              key={r.id}
              onClick={() => onPick(r.id)}
              style={{
                flex: 1,
                padding: web ? '0' : '4px 0',
                height: web ? 34 : undefined,
                fontSize: web ? 13.5 : 11,
                fontWeight: active ? 600 : 500,
                color: active ? t.ink : t.inkMute,
                background: active ? t.segThumbBg : 'transparent',
                border: 'none',
                borderRadius: 999,
                boxShadow: active ? t.segThumbShadow : 'none',
                cursor: 'pointer',
                fontFamily: 'inherit',
              }}
            >
              {r.label}
            </button>
          );
        })}
      </div>
    </div>
  );
}

function Hero({
  theme: t,
  usage,
  cachedAt,
  web,
  footer,
}: {
  theme: Theme;
  usage: Usage | null;
  cachedAt: number | null;
  web?: boolean;
  footer?: React.ReactNode;
}) {
  // Shown only once a scan has kept the cached numbers up for 400 ms: with the
  // per-file parse cache most scans land in ~0.1 s, and dimming for that long
  // reads as a flicker on every range switch.
  const [updating, setUpdating] = useState(false);
  useEffect(() => {
    if (cachedAt === null) {
      setUpdating(false);
      return;
    }
    const id = setTimeout(() => setUpdating(true), 400);
    return () => clearTimeout(id);
  }, [cachedAt]);
  return (
    <div style={{ padding: web ? '18px 20px 14px' : '14px 22px 14px' }}>
      <div
        style={{
          fontSize: web ? 12 : 11,
          fontWeight: 600,
          letterSpacing: '0.08em',
          textTransform: 'uppercase',
          color: t.inkSoft,
        }}
      >
        {usage?.rangeLabel ?? '—'} spend
      </div>
      <div
        style={{
          display: 'flex',
          alignItems: 'baseline',
          flexWrap: web ? 'wrap' : undefined,
          gap: web ? '2px 10px' : 10,
          marginTop: 6,
          opacity: updating ? 0.45 : 1,
          transition: 'opacity 0.2s',
        }}
      >
        <div
          style={{
            // -apple-system resolves to SF Pro Display at this size.
            fontFamily: '-apple-system, "SF Pro Display", "Helvetica Neue", sans-serif',
            fontSize: web ? 50 : 48,
            fontWeight: 600,
            lineHeight: 1.04,
            color: t.ink,
            letterSpacing: '-0.03em',
            fontVariantNumeric: 'tabular-nums',
          }}
        >
          {fmtMoney(usage?.totalCost ?? 0)}
        </div>
        <div style={{ fontSize: web ? 14 : 12, color: t.inkSoft, marginLeft: 4 }}>
          {fmtTokens(usage?.totalTokens ?? 0)} tokens
        </div>
      </div>
      {updating ? (
        <div style={{ marginTop: 6, fontSize: 12, color: t.inkSoft }}>
          Updating…{cachedAt ? ` · cached ${formatAsOf(new Date(cachedAt).toISOString())}` : ''}
        </div>
      ) : (
        usage?.deltaPct !== null &&
        usage?.deltaPct !== undefined && <DeltaRow theme={t} pct={usage.deltaPct} web={web} />
      )}
      {footer}
    </div>
  );
}

function DeltaRow({ theme: t, pct, web }: { theme: Theme; pct: number; web?: boolean }) {
  const up = pct >= 0;
  // Apple system green/orange; same up=green semantic as before.
  const c = up ? '#34C759' : '#FF9F0A';
  return (
    <div
      style={{
        marginTop: 6,
        display: 'flex',
        alignItems: 'center',
        gap: 6,
        fontSize: web ? 14 : 12,
        color: t.inkSoft,
      }}
    >
      {up ? <Icon.ArrowUp color={c} size={web ? 12 : 10} /> : <Icon.ArrowDown color={c} size={web ? 12 : 10} />}
      <span style={{ color: c, fontWeight: 600 }}>
        {up ? '+' : ''}
        {pct.toFixed(0)}%
      </span>
      <span>vs yesterday</span>
    </div>
  );
}

function SparkRow({
  theme: t,
  series,
  leftLabel,
  rightLabel,
  web,
}: {
  theme: Theme;
  series: SparkSeries[];
  leftLabel: string;
  rightLabel: string;
  web?: boolean;
}) {
  return (
    <div style={{ padding: web ? '0 16px 16px' : '0 18px 14px' }}>
      <Sparkline series={series} height={web ? 72 : 60} />
      <div
        style={{
          display: 'flex',
          justifyContent: 'space-between',
          fontSize: web ? 12 : 10,
          color: t.inkMute,
          marginTop: 4,
          fontVariantNumeric: 'tabular-nums',
        }}
      >
        <span>{leftLabel}</span>
        <span>{rightLabel}</span>
      </div>
    </div>
  );
}

interface ModelRow {
  id: string;
  name: string;
  color: string;
  usage: ModelUsage;
  cost: number;
}


function ByModel({
  theme: t,
  models,
  totalCost,
  priceHealth,
  web,
}: {
  theme: Theme;
  models: ModelRow[];
  totalCost: number;
  priceHealth: PriceHealth | undefined;
  web?: boolean;
}) {
  return (
    <div style={{ padding: web ? '16px 20px 10px' : '14px 22px 10px', borderTop: `0.5px solid ${t.hairline}` }}>
      <SectionLabel theme={t} web={web} right={<PriceNote theme={t} health={priceHealth} />}>
        By model
      </SectionLabel>
      {models.length === 0 && (
        <div style={{ fontSize: web ? 14 : 12, color: t.inkFaint, padding: '6px 0' }}>
          No usage in this range.
        </div>
      )}
      {models.map((m) => {
        const pct = totalCost > 0 ? (m.cost / totalCost) * 100 : 0;
        return (
          <div key={m.id} style={{ marginBottom: web ? 14 : 11 }}>
            <div style={{ display: 'flex', alignItems: 'center', gap: 8, fontSize: web ? 15 : 13 }}>
              <Icon.Dot color={m.color} size={web ? 9 : 8} />
              <span style={{ fontWeight: 500, color: t.ink }}>{m.name}</span>
              <div style={{ flex: 1 }} />
              <span style={{ fontVariantNumeric: 'tabular-nums', fontWeight: 600, color: t.ink }}>
                {fmtMoney(m.cost)}
              </span>
              <span
                style={{
                  fontSize: web ? 12.5 : 11,
                  color: t.inkMute,
                  fontVariantNumeric: 'tabular-nums',
                  width: web ? 40 : 38,
                  textAlign: 'right',
                }}
              >
                {pct.toFixed(0)}%
              </span>
            </div>
            <div
              style={{
                height: web ? 6 : 5,
                marginTop: web ? 6 : 5,
                background: t.trackBg,
                borderRadius: 999,
                overflow: 'hidden',
              }}
            >
              <div
                style={{
                  height: '100%',
                  width: `${pct}%`,
                  background: m.color,
                  borderRadius: 999,
                  boxShadow: 'inset 0 0.5px 0 rgba(255,255,255,0.5)',
                }}
              />
            </div>
            <div
              style={{
                marginTop: web ? 5 : 4,
                display: 'flex',
                gap: 10,
                fontSize: web ? 12 : 10.5,
                color: t.inkSoft,
                fontVariantNumeric: 'tabular-nums',
              }}
            >
              {/* Chat, Duet and Cowork have a cost but no tokens. */}
              {m.usage.in + m.usage.out > 0 && (
                <>
                  <span>in {fmtTokens(m.usage.in)}</span>
                  <span>out {fmtTokens(m.usage.out)}</span>
                </>
              )}
              {m.usage.cacheRead > 0 && <span>cache {fmtTokens(m.usage.cacheRead)}</span>}
            </div>
          </div>
        );
      })}
    </div>
  );
}

// "3h" / "2d" since an ISO time; null when missing, unparseable or in the future.
function fmtAge(iso: string | null): string | null {
  const ms = iso ? Date.now() - Date.parse(iso) : Number.NaN;
  if (!Number.isFinite(ms) || ms < 0) return null;
  const h = ms / 3_600_000;
  if (h < 1) return `${Math.max(1, Math.round(ms / 60_000))}m`;
  if (h < 48) return `${Math.floor(h)}h`;
  return `${Math.floor(h / 24)}d`;
}

// Header note for the LiteLLM price table: nothing while it's fresh, a muted
// age at 2 days, a warning past 3 days or after a failed refresh.
function priceNote(h: PriceHealth | undefined): { text: string; tip: string; warn: boolean } | null {
  if (!h || h.status === 'fresh') return null;
  const age = fmtAge(h.fetchedAt);
  const checked = h.fetchedAt
    ? `${age ? `${age} ago` : 'at an unknown time'} (${new Date(h.fetchedAt).toLocaleString()})`
    : '';
  const failure = h.lastError
    ? `Last update failed ${fmtAge(h.lastErrorAt) ?? '?'} ago: ${h.lastError}`
    : null;
  switch (h.status) {
    case 'builtin':
      return {
        text: 'Built-in prices',
        warn: failure !== null,
        tip: ['LiteLLM price table not loaded yet — costs use the built-in rates.', failure]
          .filter(Boolean)
          .join('\n'),
      };
    case 'error':
      return {
        text: 'Price update failed',
        warn: true,
        tip: `${failure ?? 'Last update failed.'}\nStill using the table checked ${checked}. Retries automatically.`,
      };
    case 'aging':
    case 'stale':
      return {
        text: age ? `Prices ${age} old` : 'Prices out of date',
        warn: h.status === 'stale',
        tip: `LiteLLM price table last checked ${checked}. It normally refreshes daily.`,
      };
  }
}

function PriceNote({ theme: t, health }: { theme: Theme; health: PriceHealth | undefined }) {
  const note = priceNote(health);
  if (!note) return null;
  const c = note.warn ? t.warn : t.inkMute;
  return (
    <span
      title={note.tip}
      style={{
        display: 'flex',
        alignItems: 'center',
        gap: 4,
        color: c,
        letterSpacing: 0,
        textTransform: 'none',
        fontVariantNumeric: 'tabular-nums',
        cursor: 'default',
      }}
    >
      {note.warn && <Icon.Warn color={c} size={11} />}
      {note.text}
    </span>
  );
}

const clampPct = (pct: number) => Math.min(100, Math.max(0, pct));

function liveEpochPct(resetsAt: number | null, pct: number): number {
  return resetsAt !== null && resetsAt * 1000 <= Date.now() ? 0 : clampPct(pct);
}

function liveIsoPct(window: LimitWindow | null): number | null {
  if (!window) return null;
  const resetMs = window.resetsAt ? Date.parse(window.resetsAt) : Number.NaN;
  return Number.isFinite(resetMs) && resetMs <= Date.now() ? 0 : clampPct(window.usedPct);
}

// Shared official rolling-window meters. Claude uses Anthropic's private
// /api/oauth/usage endpoint; Codex uses OpenAI's /wham/usage, falling back to
// the main bucket's rate_limits in the rollouts.
function LimitsRow({
  theme: t,
  title,
  fivePct,
  weeklyPct,
  barColor,
  tooltip,
  staleAsOf,
  web,
}: {
  theme: Theme;
  title: string;
  fivePct: number | null;
  weeklyPct: number | null;
  barColor: string;
  tooltip: string;
  // When the live read failed: the time of the reading on show.
  staleAsOf?: string;
  web?: boolean;
}) {
  const meter = (label: string, pct: number) => (
    <div
      style={{
        display: 'flex',
        alignItems: 'center',
        gap: web ? 8 : 6,
        flex: 1,
        minWidth: 0,
        opacity: staleAsOf ? 0.55 : 1,
      }}
    >
      <span style={{ color: t.inkMute, width: web ? 22 : 18 }}>{label}</span>
      <div
        style={{
          flex: 1,
          height: web ? 6 : 5,
          background: t.trackBg,
          borderRadius: 999,
          overflow: 'hidden',
        }}
      >
        <div
          style={{
            height: '100%',
            width: `${pct}%`,
            background: barColor,
            borderRadius: 999,
            boxShadow: 'inset 0 0.5px 0 rgba(255,255,255,0.5)',
          }}
        />
      </div>
      <span
        style={{
          color: t.inkSoft,
          fontVariantNumeric: 'tabular-nums',
          width: web ? 38 : 34,
          textAlign: 'right',
        }}
      >
        {pct.toFixed(0)}%
      </span>
    </div>
  );

  const label = (
    <span
      style={{
        fontWeight: 600,
        letterSpacing: '0.08em',
        textTransform: 'uppercase',
        color: t.inkSoft,
        whiteSpace: 'nowrap',
      }}
    >
      {title}
    </span>
  );

  // Phone: title on its own line (with the stale time, which is a tooltip-free
  // fact here), the two meters full width below — a 330 px row can't hold all.
  if (web) {
    return (
      <div style={{ padding: '12px 20px 14px', borderTop: `0.5px solid ${t.hairline}`, fontSize: 12.5 }}>
        <div style={{ display: 'flex', alignItems: 'center', marginBottom: 9 }}>
          {label}
          <div style={{ flex: 1 }} />
          {staleAsOf && <span style={{ color: t.inkMute }}>as of {formatAsOf(staleAsOf)}</span>}
        </div>
        <div style={{ display: 'flex', gap: 18 }}>
          {fivePct !== null && meter('5h', fivePct)}
          {weeklyPct !== null && meter('wk', weeklyPct)}
        </div>
      </div>
    );
  }

  return (
    <div
      title={tooltip}
      style={{
        padding: '10px 22px 12px',
        borderTop: `0.5px solid ${t.hairline}`,
        display: 'flex',
        alignItems: 'center',
        gap: 14,
        fontSize: 11,
      }}
    >
      {label}
      {fivePct !== null && meter('5h', fivePct)}
      {weeklyPct !== null && meter('wk', weeklyPct)}
      {staleAsOf && (
        <span style={{ color: t.inkMute, whiteSpace: 'nowrap' }}>{formatAsOf(staleAsOf)}</span>
      )}
    </div>
  );
}

function formatAsOf(iso: string): string {
  const d = new Date(iso);
  return d.toDateString() === new Date().toDateString()
    ? d.toLocaleTimeString([], { hour: 'numeric', minute: '2-digit' })
    : d.toLocaleDateString([], { month: 'short', day: 'numeric' });
}

function ToolCalls({
  theme: t,
  tiles,
  total,
  web,
}: {
  theme: Theme;
  tiles: { name: string; calls: number; tokens: number }[];
  total: number;
  web?: boolean;
}) {
  return (
    <div
      style={{
        padding: web ? '14px 16px 18px' : '12px 22px 18px',
        borderTop: `0.5px solid ${t.hairline}`,
      }}
    >
      <div
        style={{
          fontSize: web ? 12 : 11,
          fontWeight: 600,
          letterSpacing: '0.08em',
          textTransform: 'uppercase',
          color: t.inkSoft,
          marginBottom: 10,
          display: 'flex',
          alignItems: 'center',
          padding: web ? '0 4px' : 0,
        }}
      >
        <span>Tool calls</span>
        <div style={{ flex: 1 }} />
        <span
          style={{
            color: t.inkMute,
            fontVariantNumeric: 'tabular-nums',
            letterSpacing: 0,
            textTransform: 'none',
          }}
        >
          {total.toLocaleString()} total
        </span>
      </div>
      <div style={{ display: 'grid', gridTemplateColumns: 'repeat(3, minmax(0, 1fr))', gap: 8 }}>
        {tiles.map((tool) => {
          const iconName = iconBySemantic(tool.name);
          const IconC = Icon[iconName] as (p: { color?: string; size?: number }) => React.ReactElement;
          const c = toolColor(tool.name);
          return (
            <div
              key={tool.name}
              title={tool.name}
              style={{
                background: t.tileBg,
                border: t.tileBorder,
                // Concentric with the panel: 20 outer − ~6 visual inset.
                borderRadius: 14,
                padding: web ? '10px 12px' : '9px 11px',
                boxShadow: t.tileInset,
                minWidth: 0,
              }}
            >
              <div
                style={{
                  display: 'flex',
                  alignItems: 'center',
                  gap: 6,
                  fontSize: web ? 12.5 : 11,
                  fontWeight: 500,
                  color: c,
                }}
              >
                <IconC color={c} size={web ? 12 : 11} />
                <span
                  style={{
                    overflow: 'hidden',
                    textOverflow: 'ellipsis',
                    whiteSpace: 'nowrap',
                  }}
                >
                  {displayToolName(tool.name)}
                </span>
              </div>
              <div
                style={{
                  fontSize: web ? 18 : 15,
                  fontWeight: 600,
                  marginTop: 2,
                  fontVariantNumeric: 'tabular-nums',
                  color: t.ink,
                }}
              >
                {tool.calls.toLocaleString()}
              </div>
            </div>
          );
        })}
      </div>
    </div>
  );
}

function SectionLabel({
  theme: t,
  children,
  right,
  web,
}: {
  theme: Theme;
  children: React.ReactNode;
  right?: React.ReactNode;
  web?: boolean;
}) {
  return (
    <div
      style={{
        fontSize: web ? 12 : 11,
        fontWeight: 600,
        letterSpacing: '0.08em',
        textTransform: 'uppercase',
        color: t.inkSoft,
        marginBottom: web ? 12 : 10,
        display: 'flex',
        alignItems: 'center',
      }}
    >
      <span>{children}</span>
      <div style={{ flex: 1 }} />
      {right}
    </div>
  );
}

// Circular floating glass button — a small lozenge of glass sitting ON the
// panel slab, with its own specular top lip (the Liquid Glass layering cue).
function iconBtn(t: Theme): React.CSSProperties {
  return {
    width: 26,
    height: 26,
    borderRadius: 999,
    background: t.btnBg,
    boxShadow: t.btnShadow,
    border: 'none',
    padding: 0,
    color: 'inherit',
    cursor: 'pointer',
    display: 'flex',
    alignItems: 'center',
    justifyContent: 'center',
  };
}

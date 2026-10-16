import React, { useEffect, useMemo, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { Icon, iconBySemantic } from './icons';
import { Sparkline, SparkSeries } from './Sparkline';
import { ModelUsage, costForModel, displayToolName, fmtMoney, fmtTokens, liftForDark, modelMeta, toolColor } from './format';
import { Theme, ThemeMode, darkTheme, lightTheme, useDarkMode } from './theme';
import { GlassSurface } from './GlassSurface';

type Range = 'session' | 'today' | 'week' | 'month' | 'all';

// Official Codex rolling-window quota, straight from the newest token_count
// event's rate_limits. `resets_at` are unix seconds; a reset in the past
// means the window rolled over with no Codex activity since → treat as 0%.
interface CodexQuota {
  asOf: string;
  primaryUsedPct: number;
  primaryWindowMinutes: number;
  primaryResetsAt: number | null;
  secondaryUsedPct: number;
  secondaryWindowMinutes: number;
  secondaryResetsAt: number | null;
  planType: string | null;
}

interface LimitWindow {
  usedPct: number;
  resetsAt: string | null;
}

interface ClaudeQuota {
  fiveHour: LimitWindow | null;
  weekly: LimitWindow | null;
  status: 'ok' | 'stale' | 'no_token' | 'error';
  message: string | null;
  fetchedAt: string;
  subscription: string | null;
}

interface Usage {
  byModel: Record<string, ModelUsage>;
  byTool: { name: string; calls: number; tokens: number }[];
  sparkline: number[];
  // Optional: absent in pre-Codex cached payloads.
  sparklineCodex?: number[];
  codexQuota?: CodexQuota | null;
  sparkLeft: string;
  sparkRight: string;
  rangeLabel: string;
  totalCost: number;
  totalTokens: number;
  deltaPct: number | null;
  creditUsed5h: number;
  creditUsedWeek: number;
  creditLimit5h: number;
  creditLimitWeek: number;
  planLabel: string;
  planId: string;
  sessionStart: string | null;
  tokens5h: number;
  tokensWeek: number;
  priorityEvents5h: number;
  activeBlockStart: string | null;
  tokenLimit5h: number;
}

type PlanId = 'pro' | 'max5x' | 'max20';
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
  } catch {}
}

const PLAN_SAMPLE: Record<PlanId, { label: string; l5: number; lw: number; t5: number }> = {
  pro: { label: 'Pro', l5: 550_000, lw: 5_000_000, t5: 9_800_000 },
  max5x: { label: 'Max 5x', l5: 3_300_000, lw: 41_670_000, t5: 58_600_000 },
  max20: { label: 'Max 20x', l5: 11_000_000, lw: 83_330_000, t5: 195_300_000 },
};

function sampleUsage(r: Range, plan: PlanId): Usage {
  const byModel: Record<string, ModelUsage> = {
    'claude-opus-4': { in: 18, out: 12, cacheWrite: 84, cacheRead: 340 },
    'claude-sonnet-4-5': { in: 142, out: 38, cacheWrite: 612, cacheRead: 8420 },
    'claude-haiku-4-5': { in: 220, out: 9, cacheWrite: 0, cacheRead: 0 },
    'claude-sonnet-3-7': { in: 0, out: 0, cacheWrite: 0, cacheRead: 0 },
    'gpt-5.5': { in: 84, out: 21, cacheWrite: 0, cacheRead: 1840 },
    'gpt-5.1-codex-max': { in: 46, out: 8, cacheWrite: 0, cacheRead: 620 },
    'codex-auto-review': { in: 24, out: 0.2, cacheWrite: 0, cacheRead: 6, priceSource: 'estimate' },
  };
  const totalCost = Object.entries(byModel).reduce((s, [id, u]) => s + costForModel(id, u), 0);
  const totalTokens = Object.values(byModel).reduce(
    (s, u) => s + u.in + u.out + u.cacheWrite + u.cacheRead,
    0,
  );
  const ps = PLAN_SAMPLE[plan];
  return {
    byModel,
    byTool: [
      { name: 'Read', calls: 1842, tokens: 4820 },
      { name: 'Edit', calls: 924, tokens: 1280 },
      { name: 'Bash', calls: 682, tokens: 3420 },
      { name: 'Grep', calls: 421, tokens: 890 },
      { name: 'WebFetch', calls: 84, tokens: 2140 },
      { name: 'Glob', calls: 312, tokens: 180 },
    ],
    sparkline: [
      2.1, 1.8, 4.2, 3.6, 5.8, 0.4, 0.2, 6.2, 8.4, 7.1, 9.8, 4.2, 1.1, 0.8, 11.2, 14.8, 9.4, 8.8,
      12.1, 3.2, 0.9, 13.4, 18.2, 22.1, 16.4, 19.8, 6.1, 1.4, 24.8, 17.2,
    ],
    sparklineCodex: [
      0.4, 0.9, 0.2, 1.8, 2.4, 0.1, 0.0, 1.2, 3.1, 2.2, 1.4, 0.8, 0.2, 0.1, 2.8, 4.2, 3.1, 1.9,
      2.4, 0.8, 0.2, 3.8, 5.1, 6.4, 4.2, 5.8, 1.4, 0.4, 7.2, 4.8,
    ],
    codexQuota: {
      asOf: new Date(Date.now() - 20 * 60 * 1000).toISOString(),
      primaryUsedPct: 65,
      primaryWindowMinutes: 300,
      primaryResetsAt: Math.floor(Date.now() / 1000) + 2.5 * 3600,
      secondaryUsedPct: 19,
      secondaryWindowMinutes: 10080,
      secondaryResetsAt: Math.floor(Date.now() / 1000) + 4.2 * 86400,
      planType: 'plus',
    },
    sparkLeft: '14 days ago',
    sparkRight: 'Today',
    rangeLabel: {
      session: 'Current session',
      today: 'Today',
      week: 'This week',
      month: 'This month',
      all: 'All-time',
    }[r],
    totalCost,
    totalTokens,
    deltaPct: 18,
    creditUsed5h: ps.l5 * 0.6,
    creditUsedWeek: ps.lw * 0.45,
    creditLimit5h: ps.l5,
    creditLimitWeek: ps.lw,
    planLabel: ps.label,
    planId: plan,
    sessionStart: new Date(Date.now() - 2.3 * 3600 * 1000).toISOString(),
    tokens5h: ps.t5 * 0.62,
    tokensWeek: ps.t5 * 5.2,
    priorityEvents5h: 0,
    activeBlockStart: new Date(Date.now() - 2.3 * 3600 * 1000).toISOString(),
    tokenLimit5h: ps.t5,
  };
}

function sampleClaudeQuota(): ClaudeQuota {
  return {
    fiveHour: {
      usedPct: 74,
      resetsAt: new Date(Date.now() + 2.5 * 3600 * 1000).toISOString(),
    },
    weekly: {
      usedPct: 31,
      resetsAt: new Date(Date.now() + 4.2 * 86400 * 1000).toISOString(),
    },
    status: 'ok',
    message: null,
    fetchedAt: new Date().toISOString(),
    subscription: 'max',
  };
}

export function PopoverContent() {
  const [range, setRange] = useState<Range>('today');
  const [plan] = useState<PlanId>(() => readPlan());
  const [usage, setUsage] = useState<Usage | null>(() => readCached('today'));
  const [claudeQuota, setClaudeQuota] = useState<ClaudeQuota | null>(null);
  const { isDark, mode, cycleMode } = useDarkMode();
  const t: Theme = isDark ? darkTheme : lightTheme;
  const sparkColor = isDark ? '#E29A78' : '#B8532F';
  const claudeLimitColor = isDark ? '#E29A78' : '#B8532F';
  // Codex overlay line — provider blue, lifted for contrast in dark mode.
  const codexSparkColor = isDark ? '#7FB3E3' : '#437FBD';
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
    const promise = isTauri
      ? invoke<Usage>('get_usage', { range: r, plan })
      : Promise.resolve(sampleUsage(r, plan));
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

  const refreshActive = () => {
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
        }
      })
      .finally(() => {
        activeInFlight.current = false;
        if (activePending.current) {
          activePending.current = false;
          refreshActive();
        }
      });
  };

  const pickRange = (r: Range) => {
    setRange(r);
    const cached = readCached(r);
    if (cached) applyIfChanged(cached);
  };

  useEffect(() => {
    refreshActive();
    const timer = setInterval(refreshActive, 15000);
    return () => clearInterval(timer);
  }, [range, plan]);

  useEffect(() => {
    void loadClaudeQuota();
    const timer = setInterval(() => void loadClaudeQuota(), 60000);
    return () => clearInterval(timer);
  }, []);

  useEffect(() => {
    const isTauri = typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window;
    if (!isTauri) return;
    const unlistenPromise = listen('usage-changed', () => {
      refreshActive();
    });
    return () => {
      unlistenPromise.then((un) => un()).catch(() => {});
    };
  }, [range, plan]);

  useEffect(() => {
    let cancelled = false;
    const queue: Range[] = ['today', 'week', 'month', 'session', 'all'];
    (async () => {
      await new Promise((r) => setTimeout(r, 1500));
      for (const r of queue) {
        if (cancelled || r === range) continue;
        await loadUsage(r);
        await new Promise((res) => setTimeout(res, 300));
      }
    })();
    return () => {
      cancelled = true;
    };
  }, []);

  const models = useMemo(() => {
    if (!usage) return [];
    return Object.entries(usage.byModel)
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
      // Top 10, but never hide an estimated row: its "est." pill is the only
      // sign that the headline includes unconfirmed spend.
      .filter((m, i) => i < 10 || m.usage.priceSource === 'estimate');
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
  const refreshAll = () => {
    refreshActive();
    void loadClaudeQuota(true);
  };

  return (
    <GlassSurface theme={t}>
      <TitleBar theme={t} mode={mode} isDark={isDark} onCycleTheme={cycleMode} onRefresh={refreshAll} />
      <RangeBar theme={t} range={range} onPick={pickRange} />
      <Hero theme={t} usage={usage} />
      <SparkRow
        theme={t}
        series={sparkSeries}
        leftLabel={usage?.sparkLeft ?? ''}
        rightLabel={usage?.sparkRight ?? ''}
      />
      <ByModel theme={t} models={models} totalCost={usage?.totalCost ?? 0} />
      {claudeQuota && (claudeQuota.fiveHour || claudeQuota.weekly) && (
        <LimitsRow
          theme={t}
          title="Claude limits"
          fivePct={liveIsoPct(claudeQuota.fiveHour)}
          weeklyPct={liveIsoPct(claudeQuota.weekly)}
          barColor={claudeLimitColor}
          tooltip={`Official Claude limits${claudeQuota.subscription ? ` (${claudeQuota.subscription})` : ''} — as of ${new Date(claudeQuota.fetchedAt).toLocaleString()}${claudeQuota.status !== 'ok' ? ` — stale${claudeQuota.message ? `: ${claudeQuota.message}` : ''}` : ''}`}
        />
      )}
      {usage?.codexQuota && (
        <LimitsRow
          theme={t}
          title="Codex limits"
          fivePct={liveEpochPct(usage.codexQuota.primaryResetsAt, usage.codexQuota.primaryUsedPct)}
          weeklyPct={liveEpochPct(usage.codexQuota.secondaryResetsAt, usage.codexQuota.secondaryUsedPct)}
          barColor={codexSparkColor}
          tooltip={`Official Codex limits${usage.codexQuota.planType ? ` (${usage.codexQuota.planType} plan)` : ''} — as of ${new Date(usage.codexQuota.asOf).toLocaleString()}`}
        />
      )}
      <ToolCalls theme={t} tiles={toolTiles} total={totalToolCalls} />
    </GlassSurface>
  );
}

function TitleBar({
  theme: t,
  mode,
  isDark,
  onCycleTheme,
  onRefresh,
}: {
  theme: Theme;
  mode: ThemeMode;
  isDark: boolean;
  onCycleTheme: () => void;
  onRefresh: () => void;
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
        style={{ display: 'flex', alignItems: 'center', gap: 6, pointerEvents: 'none' }}
      >
        <span style={{ fontWeight: 600, color: t.ink }}>Token Counter</span>
        <span style={{ color: t.inkFaint }}>· Claude + Codex</span>
      </div>
      <div data-tauri-drag-region style={{ flex: 1 }} />
      <div style={{ display: 'flex', alignItems: 'center', gap: 8, color: t.inkSoft }}>
        <button onClick={onRefresh} style={iconBtn(t)} aria-label="Refresh">
          <Icon.Refresh size={12} />
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
}: {
  theme: Theme;
  range: Range;
  onPick: (r: Range) => void;
}) {
  // Segmented control, Liquid Glass style: a shallow capsule trough recessed
  // into the panel, with the active segment as a floating glass thumb.
  return (
    <div style={{ padding: '12px 14px 0' }}>
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
                padding: '4px 0',
                fontSize: 11,
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

function Hero({ theme: t, usage }: { theme: Theme; usage: Usage | null }) {
  return (
    <div style={{ padding: '14px 22px 14px' }}>
      <div
        style={{
          fontSize: 11,
          fontWeight: 600,
          letterSpacing: '0.08em',
          textTransform: 'uppercase',
          color: t.inkSoft,
        }}
      >
        {usage?.rangeLabel ?? '—'} spend
      </div>
      <div style={{ display: 'flex', alignItems: 'baseline', gap: 10, marginTop: 6 }}>
        <div
          style={{
            // -apple-system resolves to SF Pro Display at this size.
            fontFamily: '-apple-system, "SF Pro Display", "Helvetica Neue", sans-serif',
            fontSize: 48,
            fontWeight: 600,
            lineHeight: 1.04,
            color: t.ink,
            letterSpacing: '-0.03em',
            fontVariantNumeric: 'tabular-nums',
          }}
        >
          {fmtMoney(usage?.totalCost ?? 0)}
        </div>
        <div style={{ fontSize: 12, color: t.inkSoft, marginLeft: 4 }}>
          {fmtTokens(usage?.totalTokens ?? 0)} tokens
        </div>
      </div>
      {usage?.deltaPct !== null && usage?.deltaPct !== undefined && <DeltaRow theme={t} pct={usage.deltaPct} />}
    </div>
  );
}

function DeltaRow({ theme: t, pct }: { theme: Theme; pct: number }) {
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
        fontSize: 12,
        color: t.inkSoft,
      }}
    >
      {up ? <Icon.ArrowUp color={c} /> : <Icon.ArrowDown color={c} />}
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
}: {
  theme: Theme;
  series: SparkSeries[];
  leftLabel: string;
  rightLabel: string;
}) {
  return (
    <div style={{ padding: '0 18px 14px' }}>
      <Sparkline series={series} height={60} />
      <div
        style={{
          display: 'flex',
          justifyContent: 'space-between',
          fontSize: 10,
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
}: {
  theme: Theme;
  models: ModelRow[];
  totalCost: number;
}) {
  return (
    <div style={{ padding: '14px 22px 10px', borderTop: `0.5px solid ${t.hairline}` }}>
      <SectionLabel theme={t}>By model</SectionLabel>
      {models.length === 0 && (
        <div style={{ fontSize: 12, color: t.inkFaint, padding: '6px 0' }}>
          No usage in this range.
        </div>
      )}
      {models.map((m) => {
        const pct = totalCost > 0 ? (m.cost / totalCost) * 100 : 0;
        return (
          <div key={m.id} style={{ marginBottom: 11 }}>
            <div style={{ display: 'flex', alignItems: 'center', gap: 8, fontSize: 13 }}>
              <Icon.Dot color={m.color} size={8} />
              <span style={{ fontWeight: 500, color: t.ink }}>{m.name}</span>
              {m.usage.priceSource === 'estimate' && (
                <span
                  title="Price not confirmed for this model id — cost is estimated"
                  style={{
                    fontSize: 9.5,
                    fontWeight: 600,
                    letterSpacing: 0.3,
                    color: t.inkMute,
                    background: t.trackBg,
                    borderRadius: 999,
                    padding: '1px 6px',
                  }}
                >
                  est.
                </span>
              )}
              <div style={{ flex: 1 }} />
              <span style={{ fontVariantNumeric: 'tabular-nums', fontWeight: 600, color: t.ink }}>
                {fmtMoney(m.cost)}
              </span>
              <span
                style={{
                  fontSize: 11,
                  color: t.inkMute,
                  fontVariantNumeric: 'tabular-nums',
                  width: 38,
                  textAlign: 'right',
                }}
              >
                {pct.toFixed(0)}%
              </span>
            </div>
            <div
              style={{
                height: 5,
                marginTop: 5,
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
                marginTop: 4,
                display: 'flex',
                gap: 10,
                fontSize: 10.5,
                color: t.inkSoft,
                fontVariantNumeric: 'tabular-nums',
              }}
            >
              <span>in {fmtTokens(m.usage.in)}</span>
              <span>out {fmtTokens(m.usage.out)}</span>
              {m.usage.cacheRead > 0 && <span>cache {fmtTokens(m.usage.cacheRead)}</span>}
            </div>
          </div>
        );
      })}
    </div>
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
// /api/oauth/usage endpoint; Codex uses rate_limits embedded in rollouts.
function LimitsRow({
  theme: t,
  title,
  fivePct,
  weeklyPct,
  barColor,
  tooltip,
}: {
  theme: Theme;
  title: string;
  fivePct: number | null;
  weeklyPct: number | null;
  barColor: string;
  tooltip: string;
}) {
  const meter = (label: string, pct: number) => (
    <div style={{ display: 'flex', alignItems: 'center', gap: 6, flex: 1, minWidth: 0 }}>
      <span style={{ color: t.inkMute, width: 18 }}>{label}</span>
      <div
        style={{
          flex: 1,
          height: 5,
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
          width: 34,
          textAlign: 'right',
        }}
      >
        {pct.toFixed(0)}%
      </span>
    </div>
  );

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
      {fivePct !== null && meter('5h', fivePct)}
      {weeklyPct !== null && meter('wk', weeklyPct)}
    </div>
  );
}

function ToolCalls({
  theme: t,
  tiles,
  total,
}: {
  theme: Theme;
  tiles: { name: string; calls: number; tokens: number }[];
  total: number;
}) {
  return (
    <div
      style={{
        padding: '12px 22px 18px',
        borderTop: `0.5px solid ${t.hairline}`,
      }}
    >
      <div
        style={{
          fontSize: 11,
          fontWeight: 600,
          letterSpacing: '0.08em',
          textTransform: 'uppercase',
          color: t.inkSoft,
          marginBottom: 10,
          display: 'flex',
          alignItems: 'center',
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
                padding: '9px 11px',
                boxShadow: t.tileInset,
                minWidth: 0,
              }}
            >
              <div
                style={{
                  display: 'flex',
                  alignItems: 'center',
                  gap: 6,
                  fontSize: 11,
                  fontWeight: 500,
                  color: c,
                }}
              >
                <IconC color={c} size={11} />
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
                  fontSize: 15,
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

function SectionLabel({ theme: t, children }: { theme: Theme; children: React.ReactNode }) {
  return (
    <div
      style={{
        fontSize: 11,
        fontWeight: 600,
        letterSpacing: '0.08em',
        textTransform: 'uppercase',
        color: t.inkSoft,
        marginBottom: 10,
      }}
    >
      {children}
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

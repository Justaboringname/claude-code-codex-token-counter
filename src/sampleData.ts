// Browser-preview sample data (plain `vite dev`, no Tauri). The real app
// gets these payloads from the Rust commands; the web app from a snapshot.
import { ModelUsage, costForModel } from './format';
import type { ClaudeQuota, CodexLiveQuota, PlanId, PriceHealth, Range, Usage } from './types';

const PLAN_SAMPLE: Record<PlanId, { label: string; l5: number; lw: number; t5: number }> = {
  pro: { label: 'Pro', l5: 550_000, lw: 5_000_000, t5: 9_800_000 },
  max5x: { label: 'Max 5x', l5: 3_300_000, lw: 41_670_000, t5: 58_600_000 },
  max20: { label: 'Max 20x', l5: 11_000_000, lw: 83_330_000, t5: 195_300_000 },
};

export function sampleUsage(r: Range, plan: PlanId): Usage {
  const byModel: Record<string, ModelUsage> = {
    'claude-opus-4': { in: 18, out: 12, cacheWrite: 84, cacheRead: 340 },
    'claude-sonnet-4-5': { in: 142, out: 38, cacheWrite: 612, cacheRead: 8420 },
    'claude-haiku-4-5': { in: 220, out: 9, cacheWrite: 0, cacheRead: 0 },
    'claude-sonnet-3-7': { in: 0, out: 0, cacheWrite: 0, cacheRead: 0 },
    'gpt-5.5': { in: 84, out: 21, cacheWrite: 0, cacheRead: 1840 },
    'gpt-5.1-codex-max': { in: 46, out: 8, cacheWrite: 0, cacheRead: 620 },
    'codex-auto-review': { in: 24, out: 0.2, cacheWrite: 0, cacheRead: 6, priceSource: 'estimate' },
    'claude-duet': { in: 0, out: 0, cacheWrite: 0, cacheRead: 0, cost: 0.9 },
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
    priceHealth: samplePriceHealth(),
  };
}

export function sampleCodexQuota(): CodexLiveQuota {
  return {
    fiveHour: {
      usedPct: 65,
      resetsAt: new Date(Date.now() + 2.5 * 3600 * 1000).toISOString(),
    },
    weekly: {
      usedPct: 19,
      resetsAt: new Date(Date.now() + 4.2 * 86400 * 1000).toISOString(),
    },
    status: 'ok',
    message: null,
    fetchedAt: new Date().toISOString(),
    planType: 'plus',
    resetCredits: 1,
  };
}

// Browser preview only: ?prices=aging|stale|error|builtin|builtin-error pins a
// price-table state for screenshots. Default is fresh (no indicator).
function samplePriceHealth(): PriceHealth {
  const hoursAgo = (h: number) => new Date(Date.now() - h * 3600 * 1000).toISOString();
  const failed = {
    lastError:
      'fetch: error sending request for url (https://raw.githubusercontent.com/BerriAI/litellm/main/model_prices_and_context_window.json)',
    lastErrorAt: hoursAgo(0.4),
  };
  const ok = { lastError: null, lastErrorAt: null };
  switch (new URLSearchParams(window.location.search).get('prices')) {
    case 'aging':
      return { status: 'aging', fetchedAt: hoursAgo(60), ...ok };
    case 'stale':
      return { status: 'stale', fetchedAt: hoursAgo(100), ...ok };
    case 'error':
      return { status: 'error', fetchedAt: hoursAgo(26), ...failed };
    case 'builtin':
      return { status: 'builtin', fetchedAt: null, ...ok };
    case 'builtin-error':
      return { status: 'builtin', fetchedAt: null, ...failed };
    default:
      return { status: 'fresh', fetchedAt: hoursAgo(3), ...ok };
  }
}

export function sampleClaudeQuota(): ClaudeQuota {
  // Browser preview only: ?quota=stale shows the failed-live-read state.
  const stale = new URLSearchParams(window.location.search).get('quota') === 'stale';
  return {
    fiveHour: {
      usedPct: 74,
      resetsAt: new Date(Date.now() + 2.5 * 3600 * 1000).toISOString(),
    },
    weekly: {
      usedPct: 31,
      resetsAt: new Date(Date.now() + 4.2 * 86400 * 1000).toISOString(),
    },
    status: stale ? 'stale' : 'ok',
    message: stale ? 'Claude OAuth token expired' : null,
    fetchedAt: stale ? new Date(Date.now() - 86400 * 1000).toISOString() : new Date().toISOString(),
    subscription: 'max',
  };
}

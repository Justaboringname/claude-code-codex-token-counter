// A realistic-looking schema-1 snapshot (SPEC §4) for the sync selftest's live
// upload (scripts/sync-selftest.mjs imports this file through Node's type
// stripping) and the dev-only Tauri mock. Made-up numbers; timestamps are
// relative to `now` so freshness and reset times read naturally.
import type { ModelUsage } from '../format.ts';
import type { Snapshot, Usage } from '../types.ts';

const iso = (ms: number) => new Date(ms).toISOString();

function model(inK: number, outK: number, cacheWriteK: number, cacheReadK: number, cost: number): ModelUsage & { cost: number } {
  return { in: inK, out: outK, cacheWrite: cacheWriteK, cacheRead: cacheReadK, cost };
}

// Chat, Cowork and Claude Duet: priced from the weekly share, no tokens.
const costOnly = (cost: number) => ({ in: 0, out: 0, cacheWrite: 0, cacheRead: 0, cost });

type Models = Record<string, ModelUsage & { cost: number }>;

function totals(byModel: Models) {
  let cost = 0;
  let tokens = 0;
  for (const m of Object.values(byModel)) {
    cost += m.cost;
    tokens += m.in + m.out + m.cacheWrite + m.cacheRead;
  }
  return { totalCost: Math.round(cost * 100) / 100, totalTokens: Math.round(tokens * 10) / 10 };
}

interface RangeSpec {
  rangeLabel: string;
  byModel: Models;
  byTool: Usage['byTool'];
  sparkline: number[];
  sparklineCodex: number[];
  sparkLeft: string;
  sparkRight: string;
  deltaPct: number | null;
}

function usage(now: number, { rangeLabel, byModel, byTool, sparkline, sparklineCodex, sparkLeft, sparkRight, deltaPct }: RangeSpec): Usage {
  const t = totals(byModel);
  return {
    byModel,
    byTool,
    sparkline,
    sparklineCodex,
    sparkLeft,
    sparkRight,
    rangeLabel,
    totalCost: t.totalCost,
    totalCostEst: Math.round(t.totalCost * 0.012 * 100) / 100,
    totalTokens: t.totalTokens,
    deltaPct,
    creditUsed5h: 1_310_000,
    creditUsedWeek: 25_400_000,
    creditLimit5h: 3_300_000,
    creditLimitWeek: 41_670_000,
    planLabel: 'Max 5x',
    planId: 'max5x',
    sessionStart: iso(now - 104 * 60_000),
    tokens5h: 21_800_000,
    tokensWeek: 389_000_000,
    priorityEvents5h: 0,
    activeBlockStart: iso(now - 2.4 * 3_600_000),
    tokenLimit5h: 58_600_000,
    priceHealth: { status: 'fresh' as const, fetchedAt: iso(now - 5.2 * 3_600_000), lastError: null, lastErrorAt: null },
  };
}

const tools = (scale: number): Usage['byTool'] =>
  (
    [
    ['Read', 4210, 9120],
    ['Bash', 3880, 7340],
    ['Edit', 2214, 2410],
    ['Grep', 1462, 1290],
    ['shell', 961, 2880],
    ['WebFetch', 402, 3910],
    ['apply_patch', 410, 820],
    ['Glob', 380, 220],
  ] as const
  ).map(([name, calls, tokens]) => ({
    name,
    calls: Math.max(0, Math.round(calls * scale)),
    tokens: Math.round(tokens * scale),
  }));

export function demoSnapshot(nowDate = new Date(), deviceName = 'Mac Studio'): Snapshot {
  const now = nowDate.getTime();
  const session = usage(now, {
    rangeLabel: 'Current session',
    byModel: {
      'claude-opus-5-5': model(388, 97.4, 1180, 39_600, 17.42),
      'claude-haiku-5-5': model(41, 3.2, 0, 0, 0.06),
    },
    byTool: tools(0.031),
    sparkline: [0.4, 1.1, 0.9, 1.6, 2.2, 1.4, 0.8, 1.9, 2.6, 1.7, 1.2, 1.5],
    sparklineCodex: new Array(12).fill(0),
    sparkLeft: new Date(now - 104 * 60_000).toLocaleTimeString('en-US', { hour: 'numeric', minute: '2-digit' }).toLowerCase().replace(' ', ''),
    sparkRight: 'now',
    deltaPct: null,
  });
  const today = usage(now, {
    rangeLabel: 'Today',
    byModel: {
      'claude-opus-5-5': model(1_420, 312, 4_810, 148_200, 64.18),
      'claude-fable-5-1': model(212, 61.5, 690, 18_400, 19.73),
      'gpt-6-sol-fast': model(3_120, 41.2, 0, 21_900, 8.36),
      'claude-haiku-5-5': model(96, 7.1, 0, 0, 0.14),
    },
    byTool: tools(0.12),
    sparkline: [2.4, 1.1, 0.3, 0, 0, 0, 0, 0, 0, 3.8, 9.6, 12.1, 8.4, 6.2, 10.8, 14.6, 11.2, 7.9, 4.4, 9.1, 12.8, 8.2, 5.1, 3.6],
    sparklineCodex: [0.2, 0, 0, 0, 0, 0, 0, 0, 0, 0.4, 1.2, 0.8, 0.3, 0.9, 1.6, 0.7, 0.2, 0.6, 0.4, 0.3, 0.2, 0.1, 0, 0],
    sparkLeft: '12am',
    sparkRight: 'now',
    deltaPct: 14,
  });
  const week = usage(now, {
    rangeLabel: 'This week',
    byModel: {
      'claude-opus-5-5': model(31_400, 7_820, 96_300, 2_412_000, 1_486.2),
      'claude-fable-5-1': model(4_120, 1_092, 14_800, 361_000, 402.15),
      'claude-duet': costOnly(214.3),
      'gpt-6-sol-fast': model(48_200, 690, 0, 318_000, 133.48),
      'gpt-6-astra': model(6_830, 212, 0, 51_200, 96.12),
      'claude-cowork': costOnly(61.2),
      'claude-chat': costOnly(30.61),
      'claude-sonnet-5-5': model(1_840, 402, 2_100, 61_000, 18.4),
      'claude-haiku-5-5': model(2_210, 96, 0, 0, 2.1),
    },
    byTool: tools(1),
    sparkline: [41, 38, 22, 6, 52, 61, 44, 19, 66, 72, 58, 21, 49, 70, 81, 33, 58, 64, 47, 12, 39, 55],
    sparklineCodex: [4, 6, 2, 0, 8, 12, 5, 1, 9, 14, 6, 2, 3, 11, 16, 4, 7, 9, 5, 0, 4, 6],
    sparkLeft: 'Wed 1am',
    sparkRight: 'now',
    deltaPct: null,
  });
  const month = usage(now, {
    rangeLabel: 'This month',
    byModel: {
      'claude-opus-5-5': model(42_900, 10_410, 128_000, 3_310_000, 2_031.44),
      'claude-fable-5-1': model(5_960, 1_588, 21_400, 512_000, 581.9),
      'claude-duet': costOnly(286.1),
      'gpt-6-sol-fast': model(66_100, 951, 0, 432_000, 182.65),
      'gpt-6-astra': model(9_870, 301, 0, 72_400, 137.83),
      'claude-cowork': costOnly(81.6),
      'claude-chat': costOnly(40.82),
      'claude-sonnet-5-5': model(2_610, 571, 2_980, 86_400, 26.12),
      'claude-haiku-5-5': model(3_100, 131, 0, 0, 2.95),
    },
    byTool: tools(1.38),
    sparkline: [262, 341, 188, 402, 377, 296, 431, 358, 79],
    sparklineCodex: [21, 44, 12, 38, 51, 29, 47, 36, 8],
    sparkLeft: 'Nov 1',
    sparkRight: 'Today',
    deltaPct: null,
  });
  const all = usage(now, {
    rangeLabel: 'All-time',
    byModel: {
      'claude-opus-5-5': model(268_000, 64_900, 802_000, 20_700_000, 12_684.3),
      'claude-fable-5-1': model(21_300, 5_610, 76_200, 1_830_000, 2_082.4),
      'claude-fable-5': model(14_800, 3_920, 51_800, 1_270_000, 1_441.75),
      'claude-cowork': costOnly(906.2),
      'claude-opus-4-8': model(9_640, 2_710, 33_100, 1_010_000, 1_073.6),
      'gpt-6-sol-fast': model(201_000, 2_880, 0, 1_310_000, 556.22),
      'gpt-6-astra': model(38_400, 1_170, 0, 281_000, 534.1),
      'claude-duet': costOnly(512.4),
      'gpt-5.6-sol': model(61_200, 980, 0, 402_000, 249.8),
      'claude-sonnet-5-5': model(14_100, 3_080, 16_100, 466_000, 141.2),
      'claude-chat': costOnly(96.3),
      'claude-haiku-5-5': model(16_800, 712, 0, 0, 16.9),
    },
    byTool: tools(9.6),
    sparkline: [
      310, 420, 380, 512, 640, 590, 702, 655, 810, 760, 690, 905, 1020, 940, 860, 1110, 1260, 1180, 1340, 1490, 1310, 1620, 1715,
      1580, 1840, 2010, 1930, 2290,
    ],
    sparklineCodex: [
      0, 0, 0, 12, 40, 55, 31, 62, 80, 74, 66, 92, 101, 88, 120, 140, 132, 160, 151, 170, 188, 176, 204, 191, 230, 250, 238,
      262,
    ],
    sparkLeft: 'May 31',
    sparkRight: 'Today',
    deltaPct: null,
  });

  const nextWed1am = (() => {
    const d = new Date(now);
    d.setHours(1, 0, 0, 0);
    const add = (3 - d.getDay() + 7) % 7 || (d.getTime() <= now ? 7 : 0);
    d.setDate(d.getDate() + add);
    return d.getTime();
  })();

  return {
    schema: 1,
    generatedAt: iso(now),
    device: { name: deviceName, appVersion: '0.3.0' },
    plan: 'max5x',
    usage: { session, today, week, month, all },
    claudeQuota: {
      fiveHour: { usedPct: 38, resetsAt: iso(now + 2.2 * 3_600_000) },
      weekly: { usedPct: 61, resetsAt: iso(nextWed1am) },
      status: 'ok' as const,
      message: null,
      fetchedAt: iso(now - 20_000),
      subscription: 'max',
    },
    codexQuota: {
      fiveHour: { usedPct: 22, resetsAt: iso(now + 3.1 * 3_600_000) },
      weekly: { usedPct: 47, resetsAt: iso(now + 4.6 * 86_400_000) },
      status: 'ok' as const,
      message: null,
      fetchedAt: iso(now - 20_000),
      planType: 'pro',
      resetCredits: 1,
    },
  };
}

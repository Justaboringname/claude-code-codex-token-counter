// Shared payload types: the Tauri commands' serde output (usage.rs,
// claude_limits.rs, codex_limits.rs, prices.rs, sync.rs) and the cloud-sync
// snapshot built from them. Type-only module: nothing here exists at runtime,
// so Node can import it (type-stripped) from scripts/sync-selftest.mjs.
import type { ModelUsage } from './format.ts';

export type Range = 'session' | 'today' | 'week' | 'month' | 'all';

export type PlanId = 'pro' | 'max5x' | 'max20';

// Codex's main quota bucket as last recorded in the session logs (token_count
// rate_limits), primary = 5h and secondary = weekly by window length. Only a
// fallback for the live CodexLiveQuota read. `resets_at` are unix seconds; a
// reset in the past means the window rolled over with no Codex activity since
// → treat as 0%.
export interface CodexQuota {
  asOf: string;
  primaryUsedPct: number;
  primaryWindowMinutes: number;
  primaryResetsAt: number | null;
  secondaryUsedPct: number;
  secondaryWindowMinutes: number;
  secondaryResetsAt: number | null;
  planType: string | null;
}

export interface LimitWindow {
  usedPct: number;
  resetsAt: string | null;
}

export interface ClaudeQuota {
  fiveHour: LimitWindow | null;
  weekly: LimitWindow | null;
  status: 'ok' | 'stale' | 'no_token' | 'error';
  message: string | null;
  fetchedAt: string;
  subscription: string | null;
}

// Official Codex limits from OpenAI's usage endpoint (what Codex /status shows).
export interface CodexLiveQuota {
  fiveHour: LimitWindow | null;
  weekly: LimitWindow | null;
  status: 'ok' | 'stale' | 'no_token' | 'error';
  message: string | null;
  fetchedAt: string;
  planType: string | null;
  resetCredits: number | null;
}

// State of the LiteLLM price table that priced this scan (prices.rs health_at).
export interface PriceHealth {
  status: 'fresh' | 'aging' | 'stale' | 'error' | 'builtin';
  fetchedAt: string | null;
  lastError: string | null;
  lastErrorAt: string | null;
}

export interface Usage {
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
  // The workflow-output estimate already inside totalCost (not displayed).
  totalCostEst?: number;
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
  // Optional: absent in payloads cached before the price-health indicator.
  priceHealth?: PriceHealth;
}

// ---- Cloud sync (SPEC §4/§5) ----

// What the primary Mac publishes (schema 1). Viewers must tolerate missing
// optional fields and missing ranges.
export interface Snapshot {
  schema: number;
  generatedAt: string;
  device: { name: string; appVersion?: string };
  plan?: string;
  usage: Partial<Record<Range, Usage>>;
  claudeQuota?: ClaudeQuota | null;
  codexQuota?: CodexLiveQuota | null;
}

export type SyncRole = 'off' | 'primary' | 'viewer';

// `sync_status` / the `sync-status` event.
export interface SyncStatus {
  role: SyncRole;
  paired: boolean;
  // A viewer set to upload while the main Mac is offline, and doing so now.
  // Optional: absent from older backends.
  standby?: boolean;
  standbyActive?: boolean;
  deviceName: string;
  lastUploadAt: string | null;
  lastVersion: number | null;
  lastError: string | null;
  lastErrorAt: string | null;
  uploading: boolean;
}

// `sync_get_config`.
export interface SyncConfigView {
  role: SyncRole;
  code: string | null;
  deviceName: string;
  plan: string;
  webUrl: string | null;
  standby?: boolean;
}

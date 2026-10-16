export function fmtMoney(n: number, decimals = 2): string {
  if (n >= 1000) return '$' + Math.round(n).toLocaleString('en-US');
  return '$' + n.toFixed(decimals);
}

export function fmtTokens(k: number): string {
  if (k >= 1000) return (k / 1000).toFixed(1) + 'M';
  if (k >= 1) return k.toFixed(0) + 'K';
  return (k * 1000).toFixed(0);
}

// Credits are the unit Anthropic's Max / Pro plans meter against.
// Rendered compactly for progress meters: "3.3M", "412K", "42".
export function fmtCredits(n: number): string {
  if (n >= 1_000_000) return (n / 1_000_000).toFixed(2) + 'M';
  if (n >= 10_000) return (n / 1_000).toFixed(0) + 'K';
  if (n >= 1_000) return (n / 1_000).toFixed(1) + 'K';
  return Math.round(n).toLocaleString();
}

export function pctOf(used: number, limit: number): number {
  if (!(limit > 0)) return 0;
  return Math.min(100, (used / limit) * 100);
}

export interface ModelMeta {
  name: string;
  color: string;
  family: 'fable' | 'opus' | 'sonnet' | 'haiku' | 'codex' | 'other';
}

// Dark-appearance accent lift (the Apple pattern: #007AFF → #0A84FF). Deep
// ramp colors (e.g. the GPT-5.5 navy) sink into the dark glass track without
// this. Expects #RRGGBB.
export function liftForDark(hex: string, amt = 0.28): string {
  const n = parseInt(hex.slice(1), 16);
  if (Number.isNaN(n) || hex.length !== 7) return hex;
  const lift = (c: number) => Math.round(c + (255 - c) * amt);
  const [r, g, b] = [lift(n >> 16), lift((n >> 8) & 0xff), lift(n & 0xff)];
  return `#${((r << 16) | (g << 8) | b).toString(16).padStart(6, '0')}`;
}

// Claude models render in the warm orange ramp, Codex/GPT models in a cool
// blue ramp — the two providers must be tellable apart at a glance.
const FAMILY_COLORS: Record<string, string> = {
  fable: '#B8532F',
  opus: '#C86F49',
  sonnet: '#D88C62',
  haiku: '#E8A87C',
  other: '#8B6F47',
};

// Blue ramp indexed by GPT-5 minor version — newer (pricier) models darker,
// mirroring how the Claude ramp darkens toward Fable.
const CODEX_BLUES = ['#85B5E0', '#6FA6D9', '#5893CD', '#437FBD', '#336CAA', '#265A94'];

// Mirrors the backend's is_claude_model: anything that isn't Claude is an
// OpenAI/Codex model (gpt-*, codex-*, o3, …).
export const isCodexModel = (id: string): boolean => {
  const c = id.toLowerCase();
  return !(c.startsWith('claude') || /opus|sonnet|haiku|fable/.test(c));
};

function codexMeta(c: string): ModelMeta {
  if (c === 'codex-auto-review') {
    return { name: 'Codex Review', color: '#8CA8C4', family: 'codex' };
  }
  const m = c.match(/gpt-(\d+)(?:\.(\d+))?/);
  const minor = m?.[2] ? parseInt(m[2], 10) : 0;
  const version = m ? (m[2] ? `${m[1]}.${m[2]}` : m[1]) : '';
  let name = m ? `GPT-${version}` : c === 'codex' ? 'Codex' : c;
  if (m && c.includes('codex')) name += ' Codex';
  if (c.includes('-max')) name += ' Max';
  if (c.includes('-fast')) name += ' Fast';
  const color = CODEX_BLUES[Math.min(Math.max(minor, 0), CODEX_BLUES.length - 1)];
  return { name, color, family: 'codex' };
}

export function modelMeta(canonical: string): ModelMeta {
  const c = canonical.toLowerCase();
  if (isCodexModel(c)) return codexMeta(c);
  const m = c.match(/(fable|opus|sonnet|haiku)-(\d+)(?:-(\d+))?/);
  // Unknown families (e.g. claude-mythos-5-1) keep their own name so two of
  // them don't both render as plain "Claude".
  const u = m ? null : c.match(/^claude-([a-z]+)(?:-(\d+))?(?:-(\d+))?/);
  const family: ModelMeta['family'] = m ? (m[1] as ModelMeta['family']) : 'other';
  const word = m ? m[1] : u ? u[1] : '';
  const major = m ? m[2] : u?.[2];
  const minor = m ? m[3] : u?.[3];
  const version = major ? (minor ? `${major}.${minor}` : major) : '';
  const familyLabel = word ? word[0].toUpperCase() + word.slice(1) : 'Claude';
  const name = version ? `${familyLabel} ${version}` : familyLabel;
  return { name, color: FAMILY_COLORS[family], family };
}

export const TOOL_COLOR: Record<string, string> = {
  Read: '#D97757',
  Edit: '#B8532F',
  Write: '#B8532F',
  MultiEdit: '#B8532F',
  Bash: '#8B6F47',
  BashOutput: '#8B6F47',
  Grep: '#E8A87C',
  WebFetch: '#6B8E4E',
  WebSearch: '#6B8E4E',
  Glob: '#A68C6C',
  // Codex tools arrive lowercase from the backend and render blue.
  shell: '#437FBD',
  apply_patch: '#5893CD',
  update_plan: '#6FA6D9',
  web_search: '#336CAA',
};

export const toolColor = (name: string) => TOOL_COLOR[name] ?? '#8B6F47';

// Codex tool ids → tile labels.
const CODEX_TOOL_NAMES: Record<string, string> = {
  shell: 'Shell',
  apply_patch: 'Apply Patch',
  update_plan: 'Plan',
  web_search: 'Web Search',
};

// MCP tool ids ("mcp__<Server>__<tool>") are too long for the tool-grid
// tiles; show just the tool segment and keep the full id for tooltips.
export function displayToolName(name: string): string {
  if (CODEX_TOOL_NAMES[name]) return CODEX_TOOL_NAMES[name];
  if (!name.startsWith('mcp__')) return name;
  const rest = name.slice(5);
  const i = rest.indexOf('__');
  return i >= 0 ? rest.slice(i + 2) : rest;
}

function pricing(id: string) {
  const c = id.toLowerCase();
  // GPT/Codex rates (browser-preview fallback; the Rust table is canonical).
  // Cached input maps onto cacheRead; Codex has no cache writes.
  if (isCodexModel(c)) {
    if (c.startsWith('gpt-6-astra')) return { input: 10, output: 50, cacheWrite: 0, cacheRead: 1 };
    if (c.startsWith('gpt-6.1-sol')) return { input: 2, output: 10, cacheWrite: 0, cacheRead: 0.1 };
    if (c.startsWith('gpt-6-sol')) return { input: 2, output: 10, cacheWrite: 0, cacheRead: 0.2 };
    if (c.startsWith('gpt-6-luna')) return { input: 0.1, output: 0.5, cacheWrite: 0, cacheRead: 0.01 };
    if (c.includes('-cyber')) return { input: 12.5, output: 75, cacheWrite: 0, cacheRead: 1.25 };
    if (c.includes('5.6-terra')) return { input: 2, output: 12, cacheWrite: 0, cacheRead: 0.2 };
    if (c.includes('5.6-luna')) return { input: 0.2, output: 1.2, cacheWrite: 0, cacheRead: 0.02 };
    if (c === 'gpt-5.6' || c.includes('5.6-sol'))
      return { input: 4, output: 20, cacheWrite: 0, cacheRead: 0.4 };
    if (c.includes('5.5-pro') || c.includes('5.4-pro'))
      return { input: 30, output: 180, cacheWrite: 0, cacheRead: 0 };
    if (c.includes('5.5')) return { input: 5, output: 30, cacheWrite: 0, cacheRead: 0.5 };
    if (c.includes('5.4-mini')) return { input: 0.75, output: 4.5, cacheWrite: 0, cacheRead: 0.075 };
    if (c.includes('5.4-nano')) return { input: 0.2, output: 1.25, cacheWrite: 0, cacheRead: 0.02 };
    if (c.includes('5.4')) return { input: 2.5, output: 15, cacheWrite: 0, cacheRead: 0.25 };
    if (c.includes('5.2-pro')) return { input: 21, output: 168, cacheWrite: 0, cacheRead: 0 };
    if (c.includes('5.2') || c.includes('5.3'))
      return { input: 1.75, output: 14, cacheWrite: 0, cacheRead: 0.175 };
    if (c.startsWith('gpt-5.1-codex-mini')) return { input: 0.25, output: 2, cacheWrite: 0, cacheRead: 0.025 };
    if (c === 'codex-mini-latest') return { input: 1.5, output: 6, cacheWrite: 0, cacheRead: 0.375 };
    if (c.startsWith('gpt-5-pro')) return { input: 15, output: 120, cacheWrite: 0, cacheRead: 0 };
    if (c.startsWith('gpt-5-mini')) return { input: 0.25, output: 2, cacheWrite: 0, cacheRead: 0.025 };
    if (c.startsWith('gpt-5-nano')) return { input: 0.05, output: 0.4, cacheWrite: 0, cacheRead: 0.005 };
    if (
      c === 'gpt-5' ||
      c.startsWith('gpt-5-') ||
      c === 'gpt-5.1' ||
      c.startsWith('gpt-5.1-') ||
      c === 'codex' ||
      c === 'codex-auto-review'
    )
      return { input: 1.25, output: 10, cacheWrite: 0, cacheRead: 0.125 };
    // Conservative estimate for unknown future GPT/Codex model ids (GPT-6 Astra).
    return { input: 10, output: 50, cacheWrite: 0, cacheRead: 1 };
  }
  // Claude: [input, output, cacheRead]; versioned rows before family fallbacks.
  const [input, output, cacheRead] = c.includes('fable-5-1')
    ? [10, 50, 0.25]
    : c.includes('fable')
    ? [10, 50, 1]
    : c.includes('opus-5-5')
    ? [4, 20, 0.2]
    : c.includes('opus')
    ? [5, 25, 0.5]
    : c.includes('haiku-5-5')
    ? [0.1, 0.5, 0.01]
    : c.includes('haiku')
    ? [1, 5, 0.1]
    : c.includes('sonnet-5-5')
    ? [2, 10, 0.1]
    : c.includes('sonnet-5')
    ? [2, 10, 0.2]
    : [3, 15, 0.3];
  // Fast mode: 2× on Opus 4.8/5/5.5, legacy 6× otherwise.
  const f = !c.includes('-fast') ? 1 : c.includes('opus-5') || c.includes('opus-4-8') ? 2 : 6;
  return { input: input * f, output: output * f, cacheWrite: input * 1.25 * f, cacheRead: cacheRead * f };
}

export interface ModelUsage {
  in: number;
  out: number;
  cacheWrite: number;
  cacheRead: number;
  // Populated by the Rust backend. The local `costForModel` below is
  // kept only as a fallback for the browser-preview sample path.
  cost?: number;
  // Where the backend found this model's rates. 'estimate' = no published
  // price for this exact id (family fallback); the UI flags those rows.
  priceSource?: 'litellm' | 'builtin' | 'estimate';
}

export function costForModel(id: string, u: ModelUsage): number {
  if (typeof u.cost === 'number') return u.cost;
  const p = pricing(id);
  return (u.in * p.input + u.out * p.output + u.cacheWrite * p.cacheWrite + u.cacheRead * p.cacheRead) / 1000;
}

export function rawTokensOf(u: ModelUsage): number {
  return u.in + u.out + u.cacheWrite + u.cacheRead;
}

import type { CSSProperties } from 'react';

type ColorSize = { color?: string; size?: number };

export const Icon = {
  Dot: ({ color = '#D97757', size = 8 }: ColorSize) => (
    <span
      style={{
        display: 'inline-block',
        width: size,
        height: size,
        borderRadius: '50%',
        background: color,
        flexShrink: 0,
      }}
    />
  ),
  ArrowUp: ({ size = 10, color = '#2C7A4B' }: ColorSize) => (
    <svg width={size} height={size} viewBox="0 0 10 10" fill="none">
      <path d="M5 2 L8 6 H6 V8 H4 V6 H2 Z" fill={color} />
    </svg>
  ),
  ArrowDown: ({ size = 10, color = '#C94F3C' }: ColorSize) => (
    <svg width={size} height={size} viewBox="0 0 10 10" fill="none">
      <path d="M5 8 L2 4 H4 V2 H6 V4 H8 Z" fill={color} />
    </svg>
  ),
  Spark: ({ size = 12, color = '#D97757' }: ColorSize) => (
    <svg width={size} height={size} viewBox="0 0 12 12" fill="none">
      <path d="M6 0 L7 5 L12 6 L7 7 L6 12 L5 7 L0 6 L5 5 Z" fill={color} />
    </svg>
  ),
  Settings: ({ size = 14, color = 'currentColor' }: ColorSize) => (
    <svg width={size} height={size} viewBox="0 0 14 14" fill="none">
      <circle cx="7" cy="7" r="2" stroke={color} strokeWidth="1.3" />
      <path
        d="M7 1v2M7 11v2M1 7h2M11 7h2M2.8 2.8l1.4 1.4M9.8 9.8l1.4 1.4M2.8 11.2l1.4-1.4M9.8 4.2l1.4-1.4"
        stroke={color}
        strokeWidth="1.3"
        strokeLinecap="round"
      />
    </svg>
  ),
  Close: ({ size = 10, color = 'currentColor' }: ColorSize) => (
    <svg width={size} height={size} viewBox="0 0 10 10" fill="none">
      <path d="M2 2l6 6M8 2l-6 6" stroke={color} strokeWidth="1.4" strokeLinecap="round" />
    </svg>
  ),
  // exclamationmark.triangle, for status that needs attention.
  Warn: ({ size = 11, color = 'currentColor' }: ColorSize) => (
    <svg width={size} height={size} viewBox="0 0 12 12" fill="none">
      <path
        d="M6 1.5 L11 10.5 H1 Z"
        stroke={color}
        strokeWidth="1.2"
        strokeLinejoin="round"
        fill="none"
      />
      <path d="M6 5v2.4" stroke={color} strokeWidth="1.2" strokeLinecap="round" />
      <circle cx="6" cy="8.9" r="0.65" fill={color} />
    </svg>
  ),
  Refresh: ({ size = 12, color = 'currentColor' }: ColorSize) => (
    <svg width={size} height={size} viewBox="0 0 12 12" fill="none">
      <path
        d="M10 6a4 4 0 1 1-1.2-2.8M10 2v2H8"
        stroke={color}
        strokeWidth="1.3"
        strokeLinecap="round"
        strokeLinejoin="round"
        fill="none"
      />
    </svg>
  ),
  Bash: ({ size = 12, color = 'currentColor' }: ColorSize) => (
    <svg width={size} height={size} viewBox="0 0 12 12" fill="none">
      <path
        d="M2 3l2.5 2L2 7M5.5 8H10"
        stroke={color}
        strokeWidth="1.3"
        strokeLinecap="round"
        strokeLinejoin="round"
        fill="none"
      />
    </svg>
  ),
  Edit: ({ size = 12, color = 'currentColor' }: ColorSize) => (
    <svg width={size} height={size} viewBox="0 0 12 12" fill="none">
      <path
        d="M8.5 1.5l2 2L4 10l-2.5.5L2 8z"
        stroke={color}
        strokeWidth="1.2"
        strokeLinejoin="round"
        fill="none"
      />
    </svg>
  ),
  Read: ({ size = 12, color = 'currentColor' }: ColorSize) => (
    <svg width={size} height={size} viewBox="0 0 12 12" fill="none">
      <path
        d="M2 2h4l1 1h3v7H2z"
        stroke={color}
        strokeWidth="1.2"
        strokeLinejoin="round"
        fill="none"
      />
    </svg>
  ),
  Search: ({ size = 12, color = 'currentColor' }: ColorSize) => (
    <svg width={size} height={size} viewBox="0 0 12 12" fill="none">
      <circle cx="5" cy="5" r="3" stroke={color} strokeWidth="1.3" fill="none" />
      <path d="M7.5 7.5L10 10" stroke={color} strokeWidth="1.3" strokeLinecap="round" />
    </svg>
  ),
  Web: ({ size = 12, color = 'currentColor' }: ColorSize) => (
    <svg width={size} height={size} viewBox="0 0 12 12" fill="none">
      <circle cx="6" cy="6" r="4" stroke={color} strokeWidth="1.2" fill="none" />
      <path
        d="M2 6h8M6 2c1.5 2 1.5 6 0 8M6 2c-1.5 2-1.5 6 0 8"
        stroke={color}
        strokeWidth="1.2"
        fill="none"
      />
    </svg>
  ),
  Moon: ({ size = 12, color = 'currentColor' }: ColorSize) => (
    <svg width={size} height={size} viewBox="0 0 12 12" fill="none">
      <path
        d="M10 7.2A4 4 0 1 1 4.8 2a3.2 3.2 0 0 0 5.2 5.2z"
        stroke={color}
        strokeWidth="1.3"
        strokeLinejoin="round"
        fill="none"
      />
    </svg>
  ),
  Sun: ({ size = 12, color = 'currentColor' }: ColorSize) => (
    <svg width={size} height={size} viewBox="0 0 12 12" fill="none">
      <circle cx="6" cy="6" r="2.2" stroke={color} strokeWidth="1.3" fill="none" />
      <path
        d="M6 1v1.4M6 9.6V11M1 6h1.4M9.6 6H11M2.6 2.6l1 1M8.4 8.4l1 1M2.6 9.4l1-1M8.4 3.6l1-1"
        stroke={color}
        strokeWidth="1.3"
        strokeLinecap="round"
      />
    </svg>
  ),
  // Auto appearance — half-filled circle (the macOS "automatic" glyph):
  // outlined ring with the left semicircle filled.
  Auto: ({ size = 12, color = 'currentColor' }: ColorSize) => (
    <svg width={size} height={size} viewBox="0 0 12 12" fill="none">
      <circle cx="6" cy="6" r="4.2" stroke={color} strokeWidth="1.3" fill="none" />
      <path d="M6 1.8 A4.2 4.2 0 0 0 6 10.2 Z" fill={color} />
    </svg>
  ),
  MenuBar: ({ size = 16 }: { size?: number }) => (
    <svg width={size} height={size} viewBox="0 0 16 16" fill="none">
      <path d="M8 2C8 2 4 4 4 8c0 2 1.5 4 4 4s4-2 4-4c0-4-4-6-4-6z" fill="#D97757" />
    </svg>
  ),
};

export type IconName = keyof typeof Icon;

export const iconBySemantic = (name: string): IconName => {
  const n = name.toLowerCase();
  if (n === 'read') return 'Read';
  if (n === 'edit' || n === 'write' || n === 'multiedit') return 'Edit';
  if (n === 'bash' || n === 'bashoutput' || n === 'killshell') return 'Bash';
  if (n === 'grep' || n === 'glob') return 'Search';
  if (n === 'webfetch' || n === 'websearch') return 'Web';
  return 'Search';
};

export const SurfaceStyle: CSSProperties = { position: 'relative' };

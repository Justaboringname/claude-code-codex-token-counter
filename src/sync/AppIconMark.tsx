// The app icon ("quota rings") drawn inline from the Icon Composer layers
// (src-tauri/icons/AppIcon.icon): Claude outer arc, Codex inner arc, hub dot,
// faint tracks, on the orange tile (charcoal in dark mode, as macOS draws it).
export function AppIconMark({ size = 72, dark = false }: { size?: number; dark?: boolean }) {
  const id = dark ? 'tc-icon-dark' : 'tc-icon-light';
  const tile = dark ? ['#26272B', '#111113'] : ['#EC8F66', '#D46D47'];
  const claude = dark ? '#EE8A5C' : '#FFFFFF';
  const codex = dark ? '#9ACBF3' : '#2462C8';
  const trackOpacity = dark ? 0.16 : 0.24;
  return (
    <svg width={size} height={size} viewBox="0 0 1024 1024" role="img" aria-label="Token Counter" style={{ display: 'block' }}>
      <defs>
        <linearGradient id={id} x1="0" y1="0" x2="0" y2="1">
          <stop offset="0" stopColor={tile[0]} />
          <stop offset="1" stopColor={tile[1]} />
        </linearGradient>
      </defs>
      <rect width="1024" height="1024" rx="230" fill={`url(#${id})`} />
      <rect
        x="1"
        y="1"
        width="1022"
        height="1022"
        rx="229"
        fill="none"
        stroke="#FFFFFF"
        strokeOpacity={dark ? 0.12 : 0.28}
        strokeWidth="2"
      />
      <g fill="none" strokeLinecap="round" stroke="#FFFFFF" strokeOpacity={trackOpacity}>
        <path d="M 255.66 724.00 A 296.00 296.00 0 1 1 768.34 724.00" strokeWidth="122" />
        <path d="M 376.90 654.00 A 156.00 156.00 0 1 1 647.10 654.00" strokeWidth="96" />
      </g>
      <path d="M 255.66 724.00 A 296.00 296.00 0 0 1 747.87 397.16" fill="none" stroke={claude} strokeWidth="122" strokeLinecap="round" />
      <path d="M 479.57 423.41 A 156.00 156.00 0 0 1 647.10 654.00" fill="none" stroke={codex} strokeWidth="96" strokeLinecap="round" />
      <circle cx="512" cy="576" r="58" fill={claude} />
    </svg>
  );
}

import { useId } from 'react';

export interface SparkSeries {
  data: number[];
  color: string;
}

interface Props {
  /** Single-series shorthand — kept for existing call sites. */
  data?: number[];
  color?: string;
  /** Multi-series overlay (e.g. Claude orange + Codex blue). Takes
   *  precedence over `data`/`color` when provided. All series share one
   *  y-scale so their relative heights are comparable. */
  series?: SparkSeries[];
  height?: number;
}

export function Sparkline({ data, color = '#B8532F', series, height = 60 }: Props) {
  // Unique gradient-id prefix per instance — two sparklines on one page (or
  // one with several series) must not collide on SVG defs ids.
  const uid = useId().replace(/[^a-zA-Z0-9]/g, '');
  const all: SparkSeries[] = series?.length ? series : [{ data: data ?? [], color }];

  const w = 360;
  const h = height;
  const pad = 2;
  const max = Math.max(...all.flatMap((s) => s.data), 0.0001);
  const min = 0;

  const shapes = all.map((s) => {
    const step = s.data.length > 1 ? (w - pad * 2) / (s.data.length - 1) : 0;
    const points = s.data.map((v, i) => {
      const x = pad + i * step;
      const y = h - pad - ((v - min) / (max - min)) * (h - pad * 2);
      return [x, y] as const;
    });
    const linePath = points
      .map(([x, y], i) => (i === 0 ? `M ${x.toFixed(1)} ${y.toFixed(1)}` : `L ${x.toFixed(1)} ${y.toFixed(1)}`))
      .join(' ');
    const areaPath =
      linePath +
      ` L ${(pad + (s.data.length - 1) * step).toFixed(1)} ${h - pad}` +
      ` L ${pad} ${h - pad} Z`;
    return { color: s.color, linePath, areaPath, last: points[points.length - 1] };
  });

  return (
    <svg width="100%" height={h} viewBox={`0 0 ${w} ${h}`} preserveAspectRatio="none">
      <defs>
        {shapes.map((s, i) => (
          <linearGradient key={i} id={`sparkfill-${uid}-${i}`} x1="0" y1="0" x2="0" y2="1">
            <stop offset="0%" stopColor={s.color} stopOpacity="0.25" />
            <stop offset="100%" stopColor={s.color} stopOpacity="0" />
          </linearGradient>
        ))}
      </defs>
      {shapes.map((s, i) => (
        <g key={i}>
          <path d={s.areaPath} fill={`url(#sparkfill-${uid}-${i})`} />
          <path
            d={s.linePath}
            stroke={s.color}
            strokeWidth="1.6"
            fill="none"
            strokeLinecap="round"
            strokeLinejoin="round"
          />
          {s.last && (
            <>
              <circle cx={s.last[0]} cy={s.last[1]} r="5" fill={s.color} opacity="0.2" />
              <circle cx={s.last[0]} cy={s.last[1]} r="3" fill={s.color} />
            </>
          )}
        </g>
      ))}
    </svg>
  );
}

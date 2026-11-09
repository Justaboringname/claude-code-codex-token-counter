import { useMemo } from 'react';
import qrcode from 'qrcode-generator';

// Always dark-on-white with a 4-module quiet zone, in both themes: phone
// cameras read inverted or tinted codes badly.
export function QrCode({ text, size = 176 }: { text: string; size?: number }) {
  const { n, path } = useMemo(() => {
    const qr = qrcode(0, 'L');
    qr.addData(text, 'Byte');
    qr.make();
    const count = qr.getModuleCount();
    let d = '';
    for (let r = 0; r < count; r++) {
      let c = 0;
      while (c < count) {
        if (!qr.isDark(r, c)) {
          c++;
          continue;
        }
        let e = c;
        while (e < count && qr.isDark(r, e)) e++;
        d += `M${c + 4} ${r + 4}h${e - c}v1h-${e - c}z`;
        c = e;
      }
    }
    return { n: count, path: d };
  }, [text]);
  const total = n + 8;
  return (
    <svg
      width={size}
      height={size}
      viewBox={`0 0 ${total} ${total}`}
      shapeRendering="crispEdges"
      role="img"
      aria-label="QR code for the phone web app"
      style={{ display: 'block', borderRadius: 10 }}
    >
      <rect width={total} height={total} fill="#FFFFFF" />
      <path d={path} fill="#000000" />
    </svg>
  );
}

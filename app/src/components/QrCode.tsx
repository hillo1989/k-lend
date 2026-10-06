import qrcode from "qrcode-generator";
import { useMemo } from "react";

/**
 * QR-Code als SVG. Immer dunkel auf weiß, auch im dunklen Design, damit
 * Handy-Kameras ihn sicher lesen. Die Adresse wird unverändert kodiert (nicht
 * großgeschrieben), weil nicht jede Wallet großgeschriebene Adressen annimmt.
 */
export function QrCode({ text, label, size = 184 }: { text: string; label: string; size?: number }) {
  const cells = useMemo(() => {
    const q = qrcode(0, "M");
    q.addData(text, "Byte");
    q.make();
    const n = q.getModuleCount();
    const dark: string[] = [];
    for (let r = 0; r < n; r++) for (let c = 0; c < n; c++) if (q.isDark(r, c)) dark.push(`M${c + 4} ${r + 4}h1v1h-1z`);
    return { n: n + 8, path: dark.join("") };
  }, [text]);

  return (
    <svg className="qr" role="img" aria-label={label} width={size} height={size} viewBox={`0 0 ${cells.n} ${cells.n}`} shapeRendering="crispEdges">
      <rect width={cells.n} height={cells.n} fill="#fff" />
      <path d={cells.path} fill="#000" />
    </svg>
  );
}

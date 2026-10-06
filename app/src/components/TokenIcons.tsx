import { useId } from "react";
import { NATIVE, STABLE_SYMBOL } from "../config";

/**
 * Token-Symbol von GHOST: ein „G“, durch das der senkrechte Strich des
 * Dollarzeichens läuft; an dessen Enden zwei Knoten wie im BlockDAG.
 * Bewusst kein Geist (Abstand zu Aave/GHO). Dieselbe Grafik liegt als
 * public/ghost-token.svg für Wallets und Explorer bereit.
 */
export function GhostIcon({ size = 20, title }: { size?: number; title?: string }) {
  const g = useId().replace(/:/g, "");
  return (
    <svg
      className="ghost-icon"
      width={size}
      height={size}
      viewBox="0 0 64 64"
      role={title ? "img" : undefined}
      aria-hidden={title ? undefined : true}
      aria-label={title}
      focusable="false"
    >
      <defs>
        <linearGradient id={`${g}-r`} x1="0" y1="0" x2="1" y2="1">
          <stop offset="0" stopColor="#70C7BA" />
          <stop offset="1" stopColor="#49EACB" />
        </linearGradient>
      </defs>
      <circle cx="32" cy="32" r="29.5" fill="#0f1416" stroke={`url(#${g}-r)`} strokeWidth="3" />
      {/* G: offener Bogen mit Querbalken */}
      <path
        d="M44.5 22.5 A15.5 15.5 0 1 0 47.5 34 H34"
        fill="none"
        stroke={`url(#${g}-r)`}
        strokeWidth="5"
        strokeLinecap="round"
        strokeLinejoin="round"
      />
      {/* Dollar-Strich mit zwei Knoten */}
      <path d="M32 12 V52" stroke="#e9fffb" strokeWidth="3.2" strokeLinecap="round" />
      <circle cx="32" cy="11" r="3.4" fill="#49EACB" />
      <circle cx="32" cy="53" r="3.4" fill="#49EACB" />
    </svg>
  );
}

// Kaspa-Zeichen (gespiegeltes „K“ auf Türkis), Form aus der Vorlage
// cryptologos.cc/logos/kaspa-kas-logo.svg, Farben wie das offizielle Symbol
// (weißes Zeichen). Gleiche Form in public/kas-token.svg.
const KAS_VIEWBOX = "57.99 28.8 76.44 76.33";
const KAS_CIRCLE =
  "m 134.43,66.58 c 0,5.11 -2.11,10.05 -3.96,14.5 -1.85,4.45 -4.71,8.85 -8.18,12.32 -3.47,3.47 -7.64,6.46 -12.24,8.37 -4.44,1.84 -9.46,3.36 -14.57,3.36 -5.11,0 -10.24,-1.26 -14.68,-3.1 -4.61,-1.91 -7.76,-6.06 -11.23,-9.54 -3.47,-3.47 -7.36,-6.73 -9.27,-11.34 -1.91,-4.61 -2.22,-9.46 -2.22,-14.57 0,-5.11 -0.6,-10.53 1.24,-14.98 1.91,-4.61 5.94,-8.29 9.42,-11.76 3.47,-3.47 7.32,-7.1 11.93,-9.01 4.44,-1.84 9.7,-2.03 14.81,-2.03 5.11,0 10.06,0.93 14.5,2.77 4.61,1.91 9.05,4.51 12.52,7.99 3.47,3.47 6.48,7.75 8.39,12.35 1.84,4.44 3.54,9.56 3.54,14.67 z";
const KAS_MARK =
  "98.08,87.16 106.18,88.36 109.4,66.58 106.18,44.79 98.08,45.99 100.39,61.66 83.44,48.61 78.45,55.12 93.32,66.58 78.45,78.03 83.44,84.55 100.39,71.49";

/** Token-Symbol für KAS: das Kaspa-Zeichen */
export function KasIcon({ size = 20, title }: { size?: number; title?: string }) {
  return (
    <svg
      className="ghost-icon"
      width={size}
      height={size}
      viewBox={KAS_VIEWBOX}
      role={title ? "img" : undefined}
      aria-hidden={title ? undefined : true}
      aria-label={title}
      focusable="false"
    >
      <path d={KAS_CIRCLE} fill="#70C7BA" />
      <polygon points={KAS_MARK} fill="#FFFFFF" />
    </svg>
  );
}

/** Token-Name mit Symbol davor, z. B. in Tabellen und Kennzahlen */
export function TokenLabel({ token, size = 18 }: { token: "KAS" | "GHOST"; size?: number }) {
  return (
    <span className="ghost-label">
      {token === "KAS" ? <KasIcon size={size} /> : <GhostIcon size={size} />}
      {token === "KAS" ? NATIVE : STABLE_SYMBOL}
    </span>
  );
}

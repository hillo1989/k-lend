// Schema: Vault ↔ Orakel ↔ GHOST ↔ Keeper. Reines SVG, Farben aus CSS-Variablen.
import { tr } from "../lib/i18n";

export function ProtocolDiagram() {
  return (
    <figure className="diagram">
      <p className="diagram-swipe small muted" aria-hidden="true">
        {tr("← Schema seitlich wischen →", "← Swipe diagram sideways →")}
      </p>
      <div className="diagram-scroll">
        <svg viewBox="0 0 720 350" role="img" aria-labelledby="dia-title dia-desc">
          <title id="dia-title">{tr("Zusammenspiel der Bausteine", "How the pieces interact")}</title>
          <desc id="dia-desc">
            {tr(
              "Die Unterzeichner signieren Preis und Zinssatz, das Unterzeichner-Register prüft die Signaturen, und das Orakel wird aktualisiert. Der Vault liest das Orakel in derselben Transaktion, führt Schuld und Zins getrennt und prägt oder verbrennt GHOST über seinen eigenen Minter-Zweig. Der GHOST-Agent hält das Orakel frisch, passt den Zinssatz an den GHOST-Kurs an und löst Vaults unter 150 % ab.",
              "The signers sign price and interest rate, the signer register checks the signatures, and the oracle is updated. The vault reads the oracle within the same transaction, keeps debt and interest separately and mints or burns GHOST via its own minter branch. The GHOST agent keeps the oracle fresh, adjusts the interest rate to the GHOST price and liquidates vaults below 150 %.",
            )}
          </desc>
          <defs>
            <marker id="arr" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto-start-reverse">
              <path d="M0 0L10 5L0 10z" className="dia-arrowhead" />
            </marker>
          </defs>

          <rect x="250" y="12" width="220" height="56" rx="12" className="dia-box dia-muted" />
          <text x="360" y="36" className="dia-title">
            {tr("Unterzeichner-Register", "Signer register")}
          </text>
          <text x="360" y="54" className="dia-sub">
            {tr("Mehrheit signiert Preis + Zins", "Majority signs price + rate")}
          </text>
          <line x1="360" y1="68" x2="360" y2="116" className="dia-line" markerEnd="url(#arr)" />
          <text x="372" y="96" className="dia-label" textAnchor="start">
            update()
          </text>

          <rect x="250" y="118" width="220" height="64" rx="12" className="dia-box dia-accent" />
          <text x="360" y="144" className="dia-title">
            RiskOracle
          </text>
          <text x="360" y="163" className="dia-sub">
            {tr("KAS-Preis · Zinssatz · Index", "KAS price · interest rate · index")}
          </text>

          <rect x="20" y="240" width="200" height="90" rx="12" className="dia-box" />
          <text x="120" y="270" className="dia-title">
            {tr("Vault (je Nutzer)", "Vault (per user)")}
          </text>
          <text x="120" y="290" className="dia-sub">
            {tr("KAS-Sicherheit", "KAS collateral")}
          </text>
          <text x="120" y="308" className="dia-sub">
            {tr("Schuld + Zins", "debt + interest")}
          </text>
          <path d="M250 165 C 170 175, 130 195, 120 238" className="dia-line" markerEnd="url(#arr)" />
          <text x="112" y="196" className="dia-label">
            {tr("liest Preis + Index", "reads price + index")}
          </text>

          <rect x="500" y="240" width="200" height="90" rx="12" className="dia-box" />
          <text x="600" y="270" className="dia-title">
            GHOST
          </text>
          <text x="600" y="290" className="dia-sub">
            {tr("eigener Minter-Zweig", "own minter branch")}
          </text>
          <text x="600" y="308" className="dia-sub">
            {tr("je Vault", "per vault")}
          </text>
          <line x1="222" y1="272" x2="498" y2="272" className="dia-line" markerEnd="url(#arr)" />
          <text x="360" y="264" className="dia-label">
            {tr("prägen", "mint")}
          </text>
          <line x1="498" y1="300" x2="222" y2="300" className="dia-line dia-dashed" markerEnd="url(#arr)" />
          <text x="360" y="320" className="dia-label">
            {tr("tilgen = verbrennen", "repay = burn")}
          </text>

          <rect x="520" y="118" width="180" height="64" rx="12" className="dia-box dia-muted" />
          <text x="610" y="144" className="dia-title">
            {tr("GHOST-Agent", "GHOST agent")}
          </text>
          <text x="610" y="163" className="dia-sub">
            {tr("Orakel · Zins · Keeper", "oracle · rate · keeper")}
          </text>
          <line x1="518" y1="150" x2="472" y2="150" className="dia-line" markerEnd="url(#arr)" />
          <path d="M610 184 C 600 205, 560 215, 470 222" className="dia-line" markerEnd="url(#arr)" />
          <text x="560" y="228" className="dia-label">
            {tr("liquidiert unter 150 %", "liquidates below 150 %")}
          </text>
        </svg>
      </div>
      <figcaption className="muted small">
        {tr(
          "Alles läuft in einer Transaktion: Wer einen Preis braucht, gibt die aktuelle Orakel-UTXO aus und erzeugt sie unverändert neu. Der GHOST-Agent erledigt Orakel-Updates, die Zinsanpassung (höchstens stündlich) und Liquidationen; jeder kann ihn als Liquidator starten. Zum Start gibt es einen Unterzeichner, den Betreiber.",
          "Everything happens in one transaction: whoever needs a price spends the current oracle UTXO and recreates it unchanged. The GHOST agent handles oracle updates, the rate adjustment (at most hourly) and liquidations; anyone can run it as a liquidator. At launch there is one signer, the operator.",
        )}
      </figcaption>
    </figure>
  );
}

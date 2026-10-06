# K.Lend – GHOST-Stablecoin auf Kaspa L1

Quellcode von [k-lend.com](https://k-lend.com): ein überbesicherter USD-Stablecoin (**GHOST**)
mit Vaults, Preisorakel und Tauschpool, direkt als Covenants auf Kaspa L1 (SilverScript 1.0).

| Ordner | Inhalt |
|---|---|
| `contracts/` | Verträge in SilverScript (Vault, Token, Orakel, Unterzeichner-Register, Pool, Zinskasse) |
| `protocol/` | Rust: Transaktionsbau, Simulator, Kettenstand, `ghostctl` (Kommandozeile und Orakel-Agent), Tests |
| `app/` | Website (React/Vite) und Server; signiert wird ausschließlich in der Wallet des Nutzers (KasWare, Kastle) |
| `audit/`, `AUDIT.md` | Prüfberichte und ihr Stand |
| `docs/`, `ARCHITEKTUR.md`, `MAINNET.md` | Entwurf und Betriebsnotizen (deutsch) |
| `deploy/hetzner/` | Server-Einrichtung |

## Bauen und testen

```bash
git submodule update --init
cd protocol && cargo test --release
cd ../app && npm ci && npx vitest run
```

## Fehler melden

Sicherheitslücken bitte **nicht** als öffentliches Issue, sondern vertraulich an
info@k-lend.com. Alles andere gern als Issue.

## Rechte

Der Code ist zum Ansehen und Prüfen veröffentlicht. Es gibt **keine Open-Source-Lizenz**:
alle Rechte vorbehalten, Kopieren, Weiterverbreiten und Betreiben eigener Instanzen sind
ohne schriftliche Zustimmung nicht erlaubt. Ausgenommen sind die Teile unter
`vendor/` mit ihrer eigenen Lizenz.

Keine Anlageberatung. Nutzung auf eigenes Risiko, siehe [k-lend.com/#/rechtliches](https://k-lend.com/#/rechtliches).

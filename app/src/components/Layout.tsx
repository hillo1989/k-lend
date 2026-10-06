import { useEffect, useState } from "react";
import { PROTOCOL_NAME, PROTOCOL_SHORT } from "../config";
import { NetworkBar } from "./Network";
import { WalletButton } from "./WalletButton";
import { InstallHint } from "./InstallHint";
import { FOOTER_ROUTES, NAV_ROUTES, href, routeLabel, type RoutePath } from "../router";
import { tr } from "../lib/i18n";

/** Eigenes Zeichen (kein Kaspa-Logo): drei Blöcke, die auf einen vierten zeigen. */
export function Mark({ size = 28 }: { size?: number }) {
  return (
    <svg width={size} height={size} viewBox="0 0 32 32" aria-hidden="true" focusable="false">
      <rect width="32" height="32" rx="8" fill="var(--surface-2)" />
      <g fill="none" stroke="var(--teal)" strokeWidth="2">
        <path d="M8 10l8 6-8 6M16 16h8" />
      </g>
      <circle cx="8" cy="10" r="2.5" fill="var(--accent)" />
      <circle cx="8" cy="22" r="2.5" fill="var(--accent)" />
      <circle cx="16" cy="16" r="2.5" fill="var(--teal)" />
      <circle cx="24" cy="16" r="2.5" fill="var(--teal)" />
    </svg>
  );
}

export function Header({ route }: { route: RoutePath }) {
  const [open, setOpen] = useState(false);
  useEffect(() => setOpen(false), [route]);

  return (
    <header className="site-header">
      <div className="container header-row">
        <a className="brand" href={href("")} aria-label={`${PROTOCOL_NAME} – ${tr("Startseite", "home")}`}>
          <Mark />
          <span className="brand-name">
            <span className="brand-long">{PROTOCOL_NAME}</span>
            <span className="brand-short">{PROTOCOL_SHORT}</span>
          </span>
        </a>
        <button
          className="menu-btn"
          aria-expanded={open}
          aria-controls="hauptnavigation"
          onClick={() => setOpen((o) => !o)}
        >
          <span className="sr-only">{tr("Menü", "Menu")}</span>
          <svg width="22" height="22" viewBox="0 0 24 24" aria-hidden="true">
            <path d={open ? "M6 6l12 12M18 6L6 18" : "M4 7h16M4 12h16M4 17h16"} stroke="currentColor" strokeWidth="2" fill="none" />
          </svg>
        </button>
        <nav id="hauptnavigation" className={open ? "main-nav open" : "main-nav"} aria-label={tr("Hauptnavigation", "Main navigation")}>
          <ul>
            {NAV_ROUTES.map((r) => (
              <li key={r.path}>
                <a href={href(r.path)} aria-current={route === r.path ? "page" : undefined}>
                  {routeLabel(r)}
                </a>
              </li>
            ))}
          </ul>
        </nav>
        <WalletButton />
      </div>
      <NetworkBar />
      <InstallHint />
    </header>
  );
}

export function Footer() {
  return (
    <footer className="site-footer">
      <div className="container footer-grid">
        <div>
          <div className="brand footer-brand">
            <Mark size={24} />
            <span>{PROTOCOL_NAME}</span>
          </div>
          <p className="muted small">{tr("Unabhängiges Community-Projekt, nicht mit Kaspa verbunden.", "Independent community project, not affiliated with Kaspa.")}</p>
          <nav aria-label={tr("Rechtliches", "Legal")}>
            <ul className="footer-links">
              {FOOTER_ROUTES.map((r) => (
                <li key={r.path}>
                  <a href={href(r.path)}>{routeLabel(r)}</a>
                </li>
              ))}
            </ul>
          </nav>
        </div>
      </div>
    </footer>
  );
}

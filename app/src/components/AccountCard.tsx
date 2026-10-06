import { useId, useState } from "react";
import { NETWORKS, STABLE_SYMBOL } from "../config";
import { keygen } from "../lib/api";
import { useAccount } from "../lib/AccountContext";
import { useStatus } from "../lib/StatusContext";
import { de } from "../lib/status";
import { CopyButton } from "./CopyCode";
import { Callout } from "./ui";
import { TokenLabel } from "./TokenIcons";
import { tr } from "../lib/i18n";
import { Usd } from "./Usd";

/** Konto = eine Schlüsseldatei in keys/ (von ghostctl verwaltet, Geheimnisse bleiben dort). */
export function AccountCard() {
  const acc = useAccount();
  const { network } = useStatus();
  const selId = useId();
  const nameId = useId();
  const [showNew, setShowNew] = useState(false);
  const [name, setName] = useState("");
  const [busy, setBusy] = useState(false);
  const [msg, setMsg] = useState<{ kind: "info" | "danger"; text: string } | null>(null);
  const k = acc.selected;
  const nameOk = /^[a-z0-9-]{3,40}$/.test(name);
  const suggested = network === "mainnet" ? "mainnet-" : "tn10-";

  const create = async () => {
    setBusy(true);
    setMsg(null);
    try {
      const r = await keygen(network, name);
      if (!r.ok || !r.file) throw new Error(r.error ?? tr("Schlüssel konnte nicht angelegt werden.", "Key could not be created."));
      acc.select(r.file);
      acc.refresh();
      setMsg({ kind: "info", text: tr(`${r.file} angelegt. Bitte jetzt sichern (siehe Hinweis) und dann mit KAS aufladen.`, `${r.file} created. Please back it up now (see note) and then fund it with KAS.`) });
      setName("");
      setShowNew(false);
    } catch (e) {
      setMsg({ kind: "danger", text: (e as Error).message });
    } finally {
      setBusy(false);
    }
  };

  return (
    <section className="card" aria-labelledby="konto-title">
      <div className="card-head">
        <h2 id="konto-title">{tr("Konto", "Account")}</h2>
        <span className="tag">{NETWORKS[network].label}</span>
      </div>

      {acc.error && <Callout kind="warn" title={tr("Konto gerade nicht abrufbar", "Account currently unavailable")}>{acc.error}</Callout>}

      <div className="field">
        <label htmlFor={selId}>{tr("Schlüsseldatei (keys/)", "Key file (keys/)")}</label>
        <div className="input-wrap">
          <select id={selId} value={k?.file ?? ""} onChange={(e) => acc.select(e.target.value)} disabled={acc.keys.length === 0}>
            {acc.keys.length === 0 && <option value="">{acc.loading ? tr("lädt …", "loading …") : tr("keine Schlüssel gefunden", "no keys found")}</option>}
            {acc.keys.map((x) => (
              <option key={x.file} value={x.file}>
                {x.file.replace(/^keys\//, "")}
                {acc.isNetworkKey(x.file) ? "" : tr(" (Name passt nicht zum Netz)", " (name does not match network)")}
              </option>
            ))}
          </select>
        </div>
        <div className="field-hint">{tr("Die Seite sieht nur öffentliche Daten. Signiert wird lokal von ghostctl mit dieser Datei.", "The page only sees public data. Signing happens locally in ghostctl with this file.")}</div>
      </div>

      {k && (
        <>
          <dl className="kv">
            <div>
              <dt>{tr("Adresse", "Address")}</dt>
              <dd className="addr">
                <code title={k.address}>{k.address}</code>
                <CopyButton text={k.address} label={tr("Adresse", "Address")} />
              </dd>
            </div>
            <div>
              <dt>
                <TokenLabel token="KAS" />
              </dt>
              <dd>
                {k.kas === null ? tr("unbekannt (kein Node erreichbar)", "unknown (no node reachable)") : `${de(k.kas, 8)} KAS`}
                {k.kas !== null && <Usd amount={k.kas} unit="KAS" />}
              </dd>
            </div>
            <div>
              <dt>
                <TokenLabel token="GHOST" />
              </dt>
              <dd>
                {de(k.ghost, 8)} {STABLE_SYMBOL}
                <Usd amount={k.ghost} unit="GHOST" />
              </dd>
            </div>
            <div>
              <dt>{tr("Eigene Vaults", "Own vaults")}</dt>
              <dd>{k.vaults.length ? k.vaults.map((v) => `Vault ${v}`).join(", ") : tr("keine", "none")}</dd>
            </div>
          </dl>
          {k.kas !== null && k.kas < 1 && (
            <p className="small">
              <strong>{tr("Adresse mit KAS aufladen:", "Fund the address with KAS:")}</strong>{" "}
              {tr("Sende aus deiner Wallet KAS an die Adresse oben. Danach „Neu laden“.", "Send KAS from your wallet to the address above. Then “Reload”.")}
            </p>
          )}
        </>
      )}

      <div className="btn-row">
        <button type="button" className="btn btn-ghost btn-sm" onClick={acc.refresh} disabled={acc.loading}>
          {acc.loading ? tr("lädt …", "loading …") : tr("Guthaben neu laden", "Reload balance")}
        </button>
        <button type="button" className="btn btn-ghost btn-sm" aria-expanded={showNew} onClick={() => setShowNew((x) => !x)}>
          {tr("Neuen Schlüssel anlegen", "Create new key")}
        </button>
      </div>

      {showNew && (
        <div className="subpanel">
          <Callout kind="warn" title={tr("Nur eine Kopie", "Only one copy")}>
            {tr("Der neue private Schlüssel liegt danach nur in ", "The new private key will only exist in ")}
            <code>keys/{name || "<name>"}.json</code>
            {tr(
              " auf diesem Rechner. Geht die Datei verloren, sind alle KAS, GHOST und Vaults dieses Schlüssels verloren. Sichere die Datei (verschlüsselt) an einem zweiten Ort, bevor du Geld darauf überweist.",
              " on this computer. If the file is lost, all KAS, GHOST and vaults of this key are lost. Back up the file (encrypted) in a second place before sending money to it.",
            )}
          </Callout>
          <div className="field">
            <label htmlFor={nameId}>Name</label>
            <div className={name && !nameOk ? "input-wrap invalid" : "input-wrap"}>
              <span className="suffix">keys/</span>
              <input
                id={nameId}
                value={name}
                placeholder={`${suggested}zweit`}
                onChange={(e) => setName(e.target.value.toLowerCase())}
                spellCheck={false}
                autoComplete="off"
                aria-invalid={(name !== "" && !nameOk) || undefined}
              />
              <span className="suffix">.json</span>
            </div>
            <div className="field-hint">{tr(`3–40 Zeichen: a–z, 0–9, Bindestrich. Tipp: mit „${suggested}“ beginnen.`, `3–40 characters: a–z, 0–9, hyphen. Tip: start with “${suggested}”.`)}</div>
          </div>
          <button type="button" className="btn btn-primary" disabled={!nameOk || busy} aria-busy={busy} onClick={() => void create()}>
            {busy ? tr("Lege an …", "Creating …") : tr("Schlüssel anlegen", "Create key")}
          </button>
        </div>
      )}
      <div aria-live="polite">{msg && <Callout kind={msg.kind}>{msg.text}</Callout>}</div>
    </section>
  );
}

import { useState } from "react";
import { tr } from "../lib/i18n";

function useCopy() {
  const [state, setState] = useState<"idle" | "ok" | "err">("idle");
  const copy = async (text: string) => {
    try {
      await navigator.clipboard.writeText(text);
      setState("ok");
    } catch {
      setState("err");
    }
    window.setTimeout(() => setState("idle"), 1800);
  };
  return { state, copy };
}

/** Kleiner Kopier-Knopf für Adressen, TXIDs usw. */
export function CopyButton({ text, label }: { text: string; label: string }) {
  const { state, copy } = useCopy();
  return (
    <>
      <button type="button" className="btn btn-ghost btn-xs" onClick={() => void copy(text)}>
        {state === "ok" ? tr("Kopiert", "Copied") : state === "err" ? tr("Bitte markieren", "Please select") : tr("Kopieren", "Copy")}
        <span className="sr-only"> {label}</span>
      </button>
      <span className="sr-only" aria-live="polite">
        {state === "ok" ? `${label} kopiert` : ""}
      </span>
    </>
  );
}

/** Befehl als Code mit Kopier-Knopf. */
export function CopyCode({ code, label = "Befehl" }: { code: string; label?: string }) {
  return (
    <div className="copycode">
      <pre tabIndex={0} aria-label={label}>
        <code>{code}</code>
      </pre>
      <CopyButton text={code} label={label} />
    </div>
  );
}

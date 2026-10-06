// Läuft gerade eine sendende Aktion? Solange ja, darf nichts die App neu
// aufbauen (Sprachwechsel), sonst gehen Ergebnis und TXIDs verloren und
// der Nutzer sendet womöglich doppelt (Audit 10, A10-W-4).
import { useSyncExternalStore } from "react";

let count = 0;
const subs = new Set<() => void>();

function emit() {
  subs.forEach((f) => f());
}

/** Zählt laufende Aktionen; gibt die Freigabe-Funktion zurück */
export function markBusy(): () => void {
  count++;
  emit();
  let done = false;
  return () => {
    if (done) return;
    done = true;
    count--;
    emit();
  };
}

export function isBusy(): boolean {
  return count > 0;
}

export function useBusy(): boolean {
  return useSyncExternalStore(
    (cb) => {
      subs.add(cb);
      return () => subs.delete(cb);
    },
    () => count > 0,
    () => false,
  );
}

// Seite schließen oder neu laden, während gesendet wird: nachfragen
if (typeof window !== "undefined") {
  window.addEventListener("beforeunload", (e) => {
    if (count > 0) e.preventDefault();
  });
}

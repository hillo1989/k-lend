// Audit 12, Nachprüfung (Gruppe a): Die FAQ stellte Regeln von ghostctl als
// Vertragsgarantie dar („Nach jeder Zahlung bleibt mindestens 1 KAS im
// Tresor“, A12-8) und versprach den Abgleich der Tresor-Nachricht auch ohne
// übernommenen Tresor-Code (A12-1). Zweite Nachprüfung: „fragen vorher nach“
// galt auch für den GHOST-Agenten, der die Gebühr ohne Rückfrage zahlt.
import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { beforeEach, describe, expect, it } from "vitest";
import { setLangGlobal } from "../lib/i18n";
import { Faq } from "./Faq";

beforeEach(() => setLangGlobal("de"));
const faq = () => renderToStaticMarkup(createElement(Faq)).replaceAll("&quot;", '"');

describe("Nachprüfung A12-8: Regeln von ghostctl nicht als Vertragsgarantie", () => {
  it("1 KAS Rest ist die Regel von ghostctl und dieser Seite, nicht des Vertrags", () => {
    const h = faq();
    expect(h).not.toContain("Nach jeder Zahlung bleibt mindestens 1 KAS im Tresor.");
    expect(h).not.toContain("kann nur noch auslösen, wer sie selbst zahlt");
    expect(h).toContain("Nach Betrag und Höchstgebühr muss laut Vertrag nur etwas übrig bleiben.");
    expect(h).toContain("ghostctl und diese Seite zahlen nur, wenn danach mindestens 1 KAS im Tresor bleibt");
    expect(h).toContain("Ein fremder Auslöser kann dagegen zahlen, solange Betrag und Höchstgebühr gedeckt sind, und weniger Rest lassen.");
    setLangGlobal("en");
    expect(faq()).not.toContain("At least 1 KAS stays in the vault after every payment.");
  });
});

describe("A12-1 im Vertrag: Tresor-Nachricht fest gebunden, Besitzer weiter frei eintragbar", () => {
  it("Nachricht vom Vertrag erzwungen, aber keine Absender-Echtheit", () => {
    const h = faq();
    expect(h).toContain("Bei einem Tresor ist die Nachricht im Vertrag fest gebunden: Sie wurde beim Anlegen hinterlegt, jede Zahlung trägt genau sie, und wer die Zahlung auslöst, kann sie weder weglassen noch ändern.");
    expect(h).toContain("Den Besitzer eines Tresors kann beim Anlegen jeder frei eintragen, ohne dessen Signatur.");
    expect(h).not.toContain("bestimmt, wer die Zahlung auslöst, auch deren Nachricht");
    expect(h).not.toContain("„nicht prüfbar“");
    expect(h).not.toContain("beim Auslösen eine andere eingefügt");
  });
  it("die Nachricht lässt sich nachträglich nicht ändern – nur mit einem neuen Tresor", () => {
    const h = faq();
    expect(h).toContain("Kann ich die Nachricht eines Tresors später ändern?");
    expect(h).toContain("Nein. Die Nachricht steht als Hash fest im Vertrag des Tresors");
    expect(h).toContain("und auch für dich als Absender");
    expect(h).toContain("kündigst du den Tresor (der Rest kommt zu dir zurück) und legst einen neuen an");
    setLangGlobal("en");
    const e = faq().replaceAll("&#x27;", "'");
    expect(e).toContain("Can I change a vault's message later?");
    expect(e).toContain("whoever triggers a payment can neither leave it out nor change it");
  });
});

describe("Zweite Nachprüfung: der GHOST-Agent fragt nicht nach", () => {
  it("Rückfrage nur auf der Seite und in ghostctl, der Agent zahlt die Gebühr ohne", () => {
    const h = faq();
    expect(h).not.toContain("zahlen sie nur noch mit Gebühr vom eigenen Schlüssel und fragen vorher nach");
    expect(h).toContain("Diese Seite fragt dann vorher nach, ghostctl im Mainnet ebenso.");
    expect(h).toContain("Ein laufender GHOST-Agent zahlt die Gebühr dagegen ohne Rückfrage vom Schlüssel seines Betreibers, sobald der Tresor sie nicht trägt");
    setLangGlobal("en");
    const e = faq().replaceAll("&#x27;", "'");
    expect(e).not.toContain("from your own key and ask first");
    expect(e).toContain("A running GHOST agent, however, pays the fee without asking from its operator's key");
  });
});

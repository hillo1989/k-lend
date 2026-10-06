// K.Lend als App vom Startbildschirm (PWA). Bewusst OHNE Zwischenspeicher:
// Kurse, Orakel und Vaults müssen immer frisch sein, und eine neue Version der
// Seite soll sofort gelten. Ohne Netz zeigt die App eine kurze Meldung.
self.addEventListener("install", () => self.skipWaiting());
self.addEventListener("activate", (e) => e.waitUntil(self.clients.claim()));
self.addEventListener("fetch", (e) => {
  if (e.request.mode !== "navigate") return; // alles andere geht normal ans Netz
  e.respondWith(
    fetch(e.request).catch(
      () =>
        new Response(
          '<!doctype html><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1"><body style="background:#0f1416;color:#e8f3f1;font-family:system-ui;padding:24px"><h1>K.Lend</h1><p>Keine Internetverbindung. Bitte später erneut öffnen.</p></body>',
          { headers: { "Content-Type": "text/html; charset=utf-8" } },
        ),
    ),
  );
});

// @impl SPEC_UI_PWA
// Service Worker ohne Offline-Cache: Er macht die App installierbar, behandelt
// keine Anfragen (kein `fetch`-Handler) und speichert nichts. Alle Anfragen
// gehen unverändert an den Server.
"use strict";

self.addEventListener("install", () => {
  self.skipWaiting();
});

self.addEventListener("activate", (event) => {
  event.waitUntil(self.clients.claim());
});

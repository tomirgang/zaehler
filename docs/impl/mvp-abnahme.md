# MVP: Qualitätssicherung und Abnahme

Ergebnisse von {need}`WP_MVP_15` mit Stand 16.10.2026.

## Leistungstest

{need}`SPEC_PERF_BUDGET`: ein Haushalt, 20 Messstellen mit je 2 Registern,
10 Jahre monatliche Ablesungen (4 800), Release-Build, je 200 Requests nach
20 zum Aufwärmen, serverseitig gemessen über den vollständigen Router
(`crates/zaehler-server/src/perf_tests.rs`). Die Auswertungsseite enthält
die Prognosen und Diagrammdaten je Register ({need}`SPEC_ANA_REGISTER_CHARTS`).
Messung auf einem Entwicklungsrechner (WSL2), PostgreSQL 18 im Container.

| Seite | SQLite p95 | PostgreSQL p95 | Budget |
|---|---|---|---|
| Dashboard `/` | 36,8 ms | 50,2 ms | 300 ms |
| Auswertung `/meters/{id}` | 7,0 ms | 8,9 ms | 300 ms |
| `/meters/{id}/chart.json` | 6,7 ms | 6,4 ms | 300 ms |

## Sicherheitsreview

Geprüft wurden CSRF, Mandantentrennung, Uploads und Header sowie die neuen
Endpunkte (Manifest, Service Worker) und die Telemetrie.

| Bereich | Prüfung | Ergebnis |
|---|---|---|
| CSRF | Alle zustandsändernden Routen sind `POST`; Token im Header oder Formularfeld, bei Multipart im Handler; `Origin`-Prüfung; Sitzungscookie `HttpOnly`, `SameSite=Lax`, `Secure` bei HTTPS | ohne Befund; HTTP-Tests `csrf_and_origin_are_enforced` |
| Mandanten | Jeder Anwendungsfall erhält einen geprüften `Tenant`; fremde IDs ergeben 404 (Messstellen, Fotos, Entwürfe, `chart.json`, Import-Datei) | ohne Befund; Port-Testsuite und HTTP-Tests |
| Uploads | Body-Grenzen je Route; Magic Bytes; Neukodierung als JPEG ohne Metadaten; Dekodiergrenzen (Kantenlänge, Speicher); CSV nur UTF-8 ohne NUL; Archiv-Import mit Grenzen je Eintrag (512 MiB) und Anzahl | ohne Befund hoher Schwere; siehe B3 |
| Header | CSP ohne `unsafe-inline`, `nosniff`, `Referrer-Policy`, `Permissions-Policy`, `frame-ancestors 'none'` | siehe B1 |
| Manifest, Service Worker | ohne Sitzung, statisch; Service Worker ohne `fetch`-Handler und ohne Cache | ohne Befund |
| Telemetrie | keine Zählerwerte, Fotos, Tokens oder E-Mail-Adressen in Logs und Spans; AI-Span ohne Bildinhalt; `otel.endpoint` in `zaehler config` maskiert | siehe B2 |
| Konfiguration | Geheimnisse nicht serialisierbar; Entwicklungs-Login nur lokal und nicht zusammen mit OIDC | ohne Befund |

| Nr. | Befund | Schwere | Status |
|---|---|---|---|
| B1 | HTML-Antworten ohne `Cache-Control` | niedrig | behoben: `private, no-cache` für HTML ohne eigene Angabe |
| B2 | `x-request-id` eines Clients wurde ungeprüft in Logs und Antwort übernommen | niedrig | behoben: nur bis 64 sichtbare ASCII-Zeichen, sonst neue ID |
| B3 | Haushaltsimport liest `data.json` bis 512 MiB in den Speicher | niedrig | offen; betrifft angemeldete Nutzer, Grenze `upload.max_backup_bytes` |
| B4 | HSTS setzt die App nicht | niedrig | akzeptiert; Aufgabe des Ingress ({need}`SPEC_SEC_HEADERS`) |

Offene Befunde der Schwere hoch: keine.

## Barrierefreiheit (Grundprüfung)

| Prüfung | Ergebnis |
|---|---|
| Kontrast der Farbtokens (WCAG 2.1 AA, 4,5:1) | hell: kleinster Wert 5,58 (Akzent auf Hintergrund); dunkel: Gefahr-Button hatte 2,28, behoben mit Token `--error-fg` (jetzt 8,15); übrige Paare 6,5 bis 16,9 |
| Tastatur | Sprunglink „Zum Inhalt springen“ in beiden Layouts, sichtbarer Fokusrahmen (`:focus-visible`), alle Bedienelemente sind native Links, Buttons und Formularfelder |
| Beschriftungen | Formularfelder mit `label`; Diagramme mit `role="img"` und Überschrift, Tabellenalternative je Diagramm; Haushaltsauswahl mit unsichtbarem Label |
| Mobile Darstellung | alle Seiten bei 360 px ohne horizontales Scrollen (Tabellen scrollen in sich), Bedienelemente mindestens 44 px; behoben: Faktenliste der Detailseite lief bei langen Begriffen über den Rand |
| Bewegung | Diagramme ohne Animation und langsamerer Spinner bei `prefers-reduced-motion` |

Geprüft mit Chromium (DevTools-Protokoll) auf allen Seiten in 360 px; ohne
Screenreader-Test.

## Nachverfolgung

Alle Spezifikationen der MVP-Phase haben Implementierung und Test; alle
MVP-Anforderungen haben Spezifikation und Arbeitspaket. Prüfungen, die die
CI ausführt (Clippy-Verbot für SQL aus Zeichenketten, Doku-Build mit `-W`,
`helm lint`, Smoke-Test mit Image), tragen ihre Trace-Tags am jeweiligen Job
in `.forgejo/workflows/ci.yaml`.

## Betrieb

| Prüfung | Ergebnis |
|---|---|
| `helm lint` (CNPG, SQLite, S3 mit 2 Replicas, OTel und CNPG-Sicherung) | grün; Verweigerung ohne OIDC und bei Sicherungsplan ohne Ziel geprüft |
| Smoke-Test SQLite | grün (lokal) |
| Smoke-Test PostgreSQL, S3, OTel | grün (lokal), Traces im Collector |
| Keycloak-Override | Anmeldung in Chromium erfolgreich |
| Installation mit CNPG 1.27.1 in kind | grün; ein Neustart der App, bis die Datenbank bereit war |

## Offene manuelle Abnahmen

- {need}`WP_MVP_07`: Probe mit 20 Fotos verschiedener Zählertypen und
  konfiguriertem Modell; benötigt Fotos mit bekannten Werten.
- {need}`WP_MVP_12`: Installation auf Android (Chrome) und iOS (Safari).
- CI-Läufe der neuen Jobs `perf` und `smoke` auf dem Forgejo-Runner.

# v1: Qualitätssicherung und Abnahme

Ergebnisse von {need}`WP_V1_25` mit Stand 17.10.2026.

## Leistungstest

{need}`SPEC_PERF_BUDGET`: ein Haushalt, 20 Messstellen mit je 2 Registern,
10 Jahre monatliche Ablesungen und je ein Vertrag mit zwei Preiswechseln je
Jahr; dazu ein Plug-in-Hybrid mit 10 Jahren Kostenbuch und Ladevorgängen,
ein Hausanschluss mit PV-Anlage, Erzeugungszähler und Speicher (10 Jahre
monatliche Sammelerfassung, Investition im Kostenbuch) und ein API-Token.
Release-Build, je 200 Requests nach 20 zum Aufwärmen, serverseitig über den
vollständigen Router gemessen (`crates/zaehler-server/src/perf_tests.rs`),
Entwicklungsrechner, PostgreSQL 18 im Container.

| Seite | SQLite p95 | PostgreSQL p95 | Budget |
|---|---|---|---|
| Dashboard `/` | 100,5 ms | 133,7 ms | 300 ms |
| Auswertung `/meters/{id}` | 19,7 ms | 32,0 ms | 300 ms |
| `/meters/{id}/chart.json` | 9,8 ms | 14,1 ms | 300 ms |
| Kostenansicht `/meters/{id}/costs` | 13,5 ms | 19,9 ms | 300 ms |
| Haushaltskosten `/costs` | 94,9 ms | 102,3 ms | 300 ms |
| Fahrzeug `/vehicles/{id}` | 20,6 ms | 30,8 ms | 300 ms |
| Kostenbuch `/vehicles/{id}/costs` | 9,3 ms | 16,5 ms | 300 ms |
| Hausanschluss `/pv/grid/{id}` | 13,1 ms | 18,9 ms | 300 ms |
| PV-Anlage `/pv/{id}` | 34,1 ms | 43,2 ms | 300 ms |
| `/pv/{id}/balance.json` | 28,8 ms | 37,5 ms | 300 ms |
| Sammelerfassung `/pv/grid/{id}/readings` | 11,4 ms | 21,3 ms | 300 ms |
| `GET /api/v1/meters` | 41,0 ms | 59,1 ms | 300 ms |
| `GET /api/v1/meters/{id}/readings?limit=1000` | 1,4 ms | 3,4 ms | 300 ms |
| `GET /api/v1/meters/{id}/analysis` | 5,0 ms | 7,5 ms | 300 ms |
| `GET /api/v1/meters/{id}/costs` | 7,2 ms | 8,3 ms | 300 ms |

Das Dashboard ist gegenüber dem MVP (36,8 ms bzw. 50,2 ms) langsamer, da es
je Messstelle Verträge und den Gesamtverbrauch bei PV liest; es bleibt unter
der Hälfte des Budgets.

## Sicherheitsreview

Geprüft wurden die Bereiche aus {need}`WP_V1_25` sowie die neuen Teile aus
M8 und M9.

| Bereich | Prüfung | Ergebnis |
|---|---|---|
| Rollen | Jeder Anwendungsfall prüft die Mindestrolle vor dem Lesen; Rollenmatrix-Test um API-Tokens, `IngestReadings`, `ExportHouseholdCsv` und die PV-Anwendungsfälle erweitert | ohne Befund |
| API | Nur `Authorization: Bearer`, Cookies und Sitzungen werden nicht ausgewertet (kein CSRF nötig); Token mit 256 Bit Zufall, gespeichert als SHA-256; Formprüfung vor dem Nachschlagen; wirksame Rolle = Minimum aus Token- und Mitgliedsrolle bei jeder Anfrage; Body höchstens 1 MiB, höchstens 1000 Stände; Token nie im Log (Test mit Log-Mitschnitt) | siehe V2 |
| Einladungen | Einlösen nur einmal (bedingtes Update), Ablauf und Widerruf; Rücksprung nach der Anmeldung über Keycloak geprüft | ohne Befund |
| Dokumente | Download nur mit `attachment` und `Content-Security-Policy: sandbox`; Fotos der Sammelerfassung werden je Ablesung kopiert und liegen unter dem Präfix des Haushalts; Entwürfe nur des eigenen Haushalts (`get_open` mit Tenant) | ohne Befund |
| Outbox | Einträge in der Unit of Work der Änderung; Versandprotokoll mit eindeutigem Schlüssel; zwei Instanzen versenden eine Einladung genau einmal (Smoke-Test mit Mailpit) | siehe V3 |
| Export | CSV nur für Owner; ohne Nutzerkonten, Einladungen, Einstellungen und Tokens | siehe V1 |
| E-Mail | `security = none` nur für `localhost` oder mit `allow_insecure` ohne Zugangsdaten | ohne Befund |

| Nr. | Befund | Schwere | Status |
|---|---|---|---|
| V1 | CSV-Export übernahm Texte, die mit `=`, `+`, `-` oder `@` beginnen, unverändert; eine Tabellenkalkulation führt sie als Formel aus (CSV-Injection, z.B. über einen Messstellennamen eines Mitglieds) | mittel | behoben: führendes `'` bei solchen Texten, Zahlen unverändert |
| V2 | API-Antworten ohne `Cache-Control` | niedrig | behoben: `no-store` unter `/api/` |
| V3 | Die Sammelerfassung schrieb keinen Outbox-Eintrag; Abweichungs- und PV-Alarme blieben für so erfasste Stände aus | mittel (Funktion) | behoben: `readings_recorded` je Messstelle |
| V4 | `readings.api_token_id` ohne Fremdschlüssel | niedrig | akzeptiert; Tokens werden nur widerrufen, mit dem Haushalt entfallen auch die Ablesungen |
| V5 | Ratenbegrenzung der API | niedrig | akzeptiert; Aufgabe des Ingress ({need}`ADR_023`) |

Offene Befunde der Schwere hoch: keine.

## Barrierefreiheit

{need}`SPEC_A11Y`: `tests/browser/a11y.mjs` mit axe-core 4.13 über Playwright
1.63 und Chromium; Regelsätze WCAG 2.0 und 2.1 A/AA sowie 2.2 AA; Seiten in
`de` mit Beispieldaten, je Routenmuster einmal.

| Lauf | Seiten | serious/critical | Hinweise |
|---|---|---|---|
| erster Lauf | 33 | 1 (`scrollable-region-focusable`, Tabellen der Hausanschluss-Seite) | 0 |
| nach Behebung | 33 | 0 | 0 |

Behebung: Tabellen, die waagerecht scrollen, erhalten per `app.js`
`tabindex="0"` und sind damit per Tastatur erreichbar und mit den
Pfeiltasten verschiebbar.

Derselbe Lauf prüft je Seite CSP-Verstöße, Skriptfehler und ob jedes
sichtbare Diagramm gezeichnet ist: keine Verstöße, gezeichnet sind
`monthly` und `cumulative` (Messstelle), `costs` (Kosten), `pvGeneration`
und `pvConsumption` (Hausanschluss), `pvYield` und `pvBalance` (Anlage),
`vehicle` (Fahrzeug).

Nicht durchgeführt: manuelle Bedienung aller Abläufe nur mit Tastatur und
Probe mit Bildschirmleser (Orca).

## Abnahmen mit Umgebung

| Abnahme | Ergebnis |
|---|---|
| Zwei Nutzer über Keycloak ({need}`WP_V1_01`) | `tests/browser/keycloak.mjs`: Einladung, Anmeldung über Keycloak mit Rücksprung zur Einladung, Annahme, Rollen; bestanden |
| Zwei Instanzen, PostgreSQL, S3, Mailpit ({need}`WP_V1_23`) | `deploy/smoke-test.sh compose.postgres.yaml compose.s3.yaml compose.mail.yaml compose.replicas.yaml`: REST-API, eine E-Mail, genau ein Leader; bestanden |
| SQLite-Wiederherstellung ({need}`WP_V1_23`) | `VACUUM INTO` im laufenden Betrieb, Export von Original und Kopie byte-identisch |
| CNPG im kind-Cluster, Barman-Wiederherstellung ({need}`WP_V1_23`) | nicht durchgeführt; `kind` war in der Arbeitsumgebung nicht ausführbar |
| CSV in LibreOffice ({need}`WP_V1_22`) | nicht durchgeführt; `soffice` nicht ausführbar. Python `csv` liest alle Dateien |
| Reale Abrechnungen und Belege ({need}`WP_V1_08`, {need}`WP_V1_10`, {need}`WP_V1_14`) | nicht durchgeführt; keine realen Dokumente vorhanden |

## Nachverfolgung

Die Lückenlisten unter {doc}`../nachverfolgung/index` sind leer
(Anforderungen ohne Spezifikation oder Arbeitspaket, Spezifikationen ohne
Code oder Test, `implemented` ohne Code); der Docs-Build läuft mit `-W` ohne
Warnung. Spezifikation im Status `draft` nach v1: {need}`SPEC_OPS_BACKUP_CNPG`
(Wiederherstellung im Cluster nicht durchgespielt).

# MVP: Umsetzungsplan

## Ziel und Umfang

Der MVP umfasst alle Anforderungen mit `release: MVP`:
{need_count}`type == 'req' and release == 'MVP'` Anforderungen, davon
{need_count}`type == 'req' and release == 'MVP' and priority == 'must'` mit
Priorität `must`. Architektur und Detail-Design: {doc}`mvp-architektur`,
{doc}`mvp-detail-design`.

Ergebnis des MVP: Haushalte mit OIDC-Login, Messstellen mit Registern und
Zählerwechsel, Erfassung per Formular, Foto mit AI-Vorschlag und CSV-Import,
Monatsauswertung mit Vergleichen und Diagrammen, Jahresprognose, Dashboard,
Deutsch und Englisch, Betrieb über Helm und docker compose mit SQLite oder
PostgreSQL.

### Nicht im MVP

| Thema | Anforderungen | Meilenstein |
|---|---|---|
| Tarife, Kosten, Abschläge, Kostenprognose | REQ_TAR_*, {need}`REQ_FC_002` | v1 |
| Fahrzeuge inkl. Elektrofahrzeuge und Laden | REQ_CAR_* | v1 |
| Photovoltaik | REQ_PV_* | v1, PVGIS und interner Zinsfuß später |
| Erwartungsbereich und Ampel, gelerntes Saisonprofil | {need}`REQ_ANA_006`, {need}`REQ_ANA_002` | v1 |
| Benachrichtigungen | REQ_NTF_* | v1 |
| Rollen mit mehreren Mitgliedern, Einladungen | {need}`REQ_TEN_003`, {need}`REQ_TEN_004` | v1 |
| Änderungshistorie von Ablesungen | {need}`REQ_RDG_003` | v1 |
| Rechnungs- und Belegerkennung | {need}`REQ_AI_004`, {need}`REQ_AI_005` | v1 |
| REST-API, Export | REQ_API_*, {need}`REQ_IMP_002` | v1 |

### Einschränkungen im MVP

- Jeder Haushalt hat genau einen Nutzer (Owner); gemeinsame Nutzung eines
  Haushalts durch mehrere Personen erfordert {need}`REQ_TEN_004` (v1).
- Prognosen verwenden die Gleichverteilung über die Tage
  ({need}`SPEC_FC_CURRENT_YEAR`); für Gas und Wärme weicht die Prognose im
  Jahresverlauf stärker ab als mit Saisonprofil.
- Das Abrechnungsjahr wird je Messstelle über Monat und Tag des Beginns
  festgelegt, ohne Vertragsmodell.
- Ablesungen lassen sich löschen, aber nicht bearbeiten; es gibt keine
  Änderungshistorie ({need}`SPEC_RDG_DELETE`).
- Fotos im HEIC-Format werden abgelehnt ({need}`ADR_017`).
- Kein Offline-Betrieb der PWA.

## Meilensteine

| Meilenstein | Inhalt | Arbeitspakete |
|---|---|---|
| M1 | Grundgerüst, Login, Messstellen, Erfassung per Formular | WP_MVP_01 bis WP_MVP_05 |
| M2 | Objektspeicher, Foto-Erfassung mit AI | WP_MVP_06, WP_MVP_07 |
| M3 | Auswertung, Dashboard, CSV-Import | WP_MVP_08 bis WP_MVP_10 |
| M4 | Sprache Englisch, PWA, Observability, Betrieb, Qualitätssicherung | WP_MVP_11 bis WP_MVP_15 |

## Arbeitspakete

### M1: Grundgerüst, Login, Messstellen, Formular

```{wp} Hexagonale Struktur und Web-Grundgerüst
:id: WP_MVP_01
:status: done
:milestone: M1
:covers: REQ_NFR_003, SPEC_ARCH_LAYERS, SPEC_ERR_HANDLING, SPEC_SEC_HEADERS, SPEC_I18N

Umbau des Grundgerüsts auf die Crates gemäß {need}`ADR_022`: `zaehler-app`
(Ids, `AppError`, Port-Traits als Gerüst), `zaehler-web` (Router, Handler,
Templates, Assets aus `zaehler-server` übernommen), `zaehler-server` als
Composition Root (`wiring.rs`). CI-Schritt für die Abhängigkeitsregeln.
Layout-Template mit Navigation, Sicherheitsheader, versionierte statische
Dateien, `app.js`, i18n-Infrastruktur mit deutschen Texten und
Formatierungsfunktionen, Extraktor `Lang`.

**Abnahme:** Abhängigkeitsprüfung in der CI grün und bei einem absichtlichen
Verstoß rot; Fehlerseiten 404/422/500 in beiden Formaten (HTML, HTMX);
CSP-Header auf allen HTML-Antworten; Schlüsseltest `de`/`en` vorhanden;
vorhandene Tests weiterhin grün.
```

```{wp} Ports, Unit of Work, DB-Adapter und Tenant
:id: WP_MVP_02
:status: done
:milestone: M1
:depends: WP_MVP_01
:covers: REQ_OPS_002, REQ_OPS_009, REQ_NFR_005, SPEC_SEC_TENANT_SCOPE, SPEC_ARCH_UOW, SPEC_TZ, SPEC_SEC_SQL_PARAMS

Repository- und Unit-of-Work-Ports in `zaehler-app`, `Tenant` und
`AuthorizeTenant`; In-Memory-Adapter und gemeinsame Port-Testsuite (Feature
`testing`); Implementierung in `zaehler-db` inkl. Migration
`m20261015_000002_mvp` und `SessionStore`; `zaehler_domain::calendar`.

**Abnahme:** Port-Testsuite (Tenant-Isolation, Rollback ohne `commit`) grün
für In-Memory, SQLite und PostgreSQL; Migration idempotent; Tests der
Zeitumstellung für `Calendar`.
```

```{wp} Anmeldung und Haushalte
:id: WP_MVP_03
:status: done
:milestone: M1
:depends: WP_MVP_01, WP_MVP_02
:covers: REQ_TEN_001, REQ_TEN_002, REQ_NFR_004, SPEC_TEN_OIDC, SPEC_SEC_SESSION_CSRF, SPEC_TEN_HOUSEHOLDS, SPEC_TEN_DEV_LOGIN, SPEC_PRIV_HOUSEHOLD_DELETE

Sitzung und CSRF-Prüfung in `zaehler-web`, OIDC-Login und Logout, Dev-Login,
Anwendungsfälle `SignIn`, `CreateHousehold`, `SwitchHousehold`,
`UpdateHousehold`, `DeleteHousehold`, Einstellungsseite.

**Abnahme:** Login gegen Keycloak (lokal per compose-Override) und Dev-Login;
CSRF-Tests (fehlend, falsch, korrekt); Löschen eines Haushalts entfernt DB-Zeilen
und Objekte.
```

```{wp} Messstellen verwalten
:id: WP_MVP_04
:status: done
:milestone: M1
:depends: WP_MVP_03
:covers: REQ_MTR_001, REQ_MTR_003, REQ_MTR_005, REQ_MTR_006, SPEC_MTR_MANAGE, SPEC_MTR_SWAP_FLOW

Anwendungsfälle `CreateMeter`, `UpdateMeter`, `ListMeters`, `GetMeter`,
`SwapDevice`, Seiten Liste, Formular, Detail, Zählerwechsel; Validierung
gemäß {need}`SPEC_MTR_MANAGE`.

**Abnahme:** Messstelle mit zwei Registern anlegen, bearbeiten, Zählerwechsel
durchführen; Verbrauchsreihe über den Wechsel korrekt (Service-Test).
```

```{wp} Erfassung per Formular
:id: WP_MVP_05
:status: done
:milestone: M1
:depends: WP_MVP_04
:covers: REQ_RDG_001, REQ_RDG_002, SPEC_RDG_FORM, SPEC_RDG_DELETE

Anwendungsfälle `CheckReadings`, `RecordReadings`, `DeleteReading`, Erfassungsformular mit
HTMX-Plausibilitätshinweis, Ablesungsliste in der Detailansicht, Löschen.

**Abnahme:** drei Interaktionen ab Dashboard-Platzhalter; Warnungen
erzwingen Bestätigung; doppelte Ablesung ergibt `Conflict`.
```

### M2: Fotos und AI

```{wp} Objektspeicher und Bildverarbeitung
:id: WP_MVP_06
:status: done
:milestone: M2
:depends: WP_MVP_01, WP_MVP_02
:covers: REQ_OPS_010, REQ_RDG_004, SPEC_STORE_BLOBS, SPEC_IMG_PROCESSING, SPEC_SEC_UPLOADS, SPEC_OPS_HOUSEKEEPING

Ports `BlobStore` und `ImageNormalizer`; Adapter `zaehler-storage`
(Dateisystem, S3) und `zaehler-media`; In-Memory- bzw. Stub-Adapter;
Anwendungsfälle für Foto-Auslieferung mit Tenant-Prüfung; Anwendungsfall
`Housekeeping` mit Intervall in `zaehler-server`.

**Abnahme:** Medien-Fixtures (Ausrichtung, GPS, HEIC, Textdatei);
S3-Test gegen MinIO in der CI; Housekeeping idempotent bei paralleler
Ausführung.

**Abweichung:** MinIO verteilt seit 2025 keine Images und Binärdateien der
Community-Edition mehr (Docker Hub und quay.io ohne Zugriff, `dl.min.io`
antwortet mit HTTP 410). Der S3-Test läuft deshalb gegen Versity Gateway
(`versitygw`, Backend `posix`), ebenfalls S3-kompatibel. Das Verwerfen
abgelaufener AI-Entwürfe im Housekeeping ist mit WP_MVP_07 umgesetzt, da die
Tabelle `ai_drafts` erst dort entsteht. {need}`SPEC_SEC_UPLOADS` bleibt
`draft`, bis das CSV-Limit mit WP_MVP_10 umgesetzt ist.
```

```{wp} Foto-Erfassung mit AI
:id: WP_MVP_07
:status: open
:milestone: M2
:depends: WP_MVP_05, WP_MVP_06
:covers: REQ_AI_001, REQ_AI_002, REQ_AI_003, REQ_AI_006, REQ_AI_007, REQ_RDG_006, SPEC_AI_FLOW, SPEC_AI_DRAFTS, SPEC_AI_NEW_METER, SPEC_IMG_CAPTURE_TIME

Port `DraftRepository` und `MeterRecognizer` (Adapter `zaehler-ai`),
Anwendungsfälle `AnalysePhoto`, `ConfirmDraft`, `DiscardDraft`, Seiten Upload
und Bestätigung, Zuordnung,
AI-Metriken; Verhalten ohne AI-Konfiguration (Seite zeigt nur manuelle
Erfassung mit Foto).

**Abnahme:** `wiremock`-Tests für Erfolg, ungültige Antwort, HTTP 500 und
Timeout; manuelle Probe mit 20 Zählerfotos verschiedener Zählertypen und konfiguriertem Modell,
Ergebnis (Anteil korrekt erkannter Werte) im Abschlussbericht des
Arbeitspakets dokumentiert.

**Stand:** Umsetzung und automatisierte Abnahme abgeschlossen (`wiremock`-Tests
in `zaehler-ai`, Anwendungsfall- und HTTP-Tests). Offen ist die manuelle
Probe mit 20 Zählerfotos und konfiguriertem Modell; bis dahin bleibt das
Arbeitspaket `open`. Abweichend vom Detail-Design antwortet `POST /capture`
mit 303 auf die Bestätigungsseite statt mit einem HTMX-Fragment; damit
funktioniert der Ablauf ohne JavaScript und ein Neuladen der Seite bleibt
möglich. Nachträglich aufgenommen: Anlegen einer Messstelle aus einem Foto
({need}`REQ_AI_007`, {need}`SPEC_AI_NEW_METER`) und Ablesezeitpunkt aus den
Fotometadaten ({need}`REQ_RDG_006`, {need}`SPEC_IMG_CAPTURE_TIME`), umgesetzt
mit Medien-, Anwendungsfall- und HTTP-Tests.
```

### M3: Auswertung, Dashboard, Import

```{wp} Auswertung und Prognose
:id: WP_MVP_08
:status: done
:milestone: M3
:depends: WP_MVP_05
:covers: REQ_ANA_001, REQ_ANA_003, REQ_ANA_004, REQ_ANA_005, REQ_ANA_007, REQ_FC_001, SPEC_ANA_MONTHLY_SERIES, SPEC_ANA_REGISTER_AGG, SPEC_ANA_COMPARE_VIEW, SPEC_FC_CURRENT_YEAR, SPEC_ANA_CHARTS

Domain-Module `series` und `analysis`, Anwendungsfälle `MeterAnalysis` und
`MeterChartData`, Auswertungsbereich
der Detailseite, `chart.json`, `charts.js`, ECharts einbinden.

**Abnahme:** Property-Tests der Monatsserien; Vergleiche mit unvollständigen
Referenzmonaten zeigen Hinweis; Prognose für Kalender- und Abrechnungsjahr;
Diagramme mit Tabellenalternative.

**Stand:** Abnahme mit Domain-Tests (`proptest` für Monatsserien, Vergleiche,
Schaltjahr, Prognose), Anwendungsfall-Test über zwei Register und
Zählerwechsel sowie HTTP-Test für Detailseite und `chart.json`. Diagramme in
Chromium mit der bestehenden Content-Security-Policy geprüft. ECharts wird
abweichend vom Entwurf nur auf der Detailseite geladen, das Dashboard enthält
im MVP keine Diagramme.
```

```{wp} Dashboard
:id: WP_MVP_09
:status: done
:milestone: M3
:depends: WP_MVP_08
:covers: REQ_ANA_008, SPEC_UI_DASHBOARD

Dashboard-Seite mit Karten, Fälligkeit, Sortierung, leerem Zustand.

**Abnahme:** Rendering mit 0, 1 und 20 Messstellen; überfällige Messstellen
zuerst.

**Stand:** Anwendungsfall `Dashboard` mit Test der Sortierung; HTTP-Test mit
0, 1 und 20 Messstellen.
```

```{wp} CSV-Import
:id: WP_MVP_10
:status: done
:milestone: M3
:depends: WP_MVP_05, WP_MVP_06
:covers: REQ_IMP_001, SPEC_IMP_CSV, SPEC_IMP_CSV_FORMAT

Anwendungsfälle `UploadCsv`, `PreviewImport`, `CommitImport`, Seiten Upload,
Zuordnung, Vorschau, Ergebnis; Erkennung
des Trennzeichens, Datums- und Zahlenformate, Duplikate.

**Abnahme:** CSV-Fixtures; 10 Jahre Monatswerte in einem Durchgang; erneuter
Import derselben Datei meldet nur Duplikate.

**Stand:** Unit-Tests der Formate (deutsch, ISO, Tab, Komma, BOM, ohne
Überschrift, Binärdatei, zu viele Zeilen), Anwendungsfall-Tests für 10 Jahre
Monatswerte, erneuten Import (nur Duplikate), Fehler- und Hinweiszeilen sowie
HTTP-Test des Ablaufs mit CSRF und 413. Zusätzlich gegen SQLite über den
laufenden Server geprüft. Abweichungen vom Entwurf stehen in
{need}`SPEC_IMP_CSV_FORMAT` (kein `insert_skip_duplicates`, Einbau des ersten
Geräts wird bei älteren Ablesungen vorverlegt). Mit dem CSV-Limit ist
{need}`SPEC_SEC_UPLOADS` umgesetzt.
```

### M4: Release-Kandidat

```{wp} Englische Oberfläche
:id: WP_MVP_11
:status: open
:milestone: M4
:depends: WP_MVP_01
:covers: REQ_NFR_002, SPEC_I18N

Englische Texte für alle Seiten, Sprachauswahl in den Einstellungen,
`Accept-Language`, englische Zahlen- und Datumsformate.

**Abnahme:** jede Seite rendert in `de` und `en` ohne fehlende Schlüssel.
```

```{wp} Progressive Web App
:id: WP_MVP_12
:status: open
:milestone: M4
:depends: WP_MVP_01
:covers: REQ_NFR_001, SPEC_UI_PWA

Manifest, Icons, Service Worker, Prüfung des Layouts ab 360 px.

**Abnahme:** Installation auf Android (Chrome) und iOS (Safari, „Zum
Home-Bildschirm“); Lighthouse-Kategorie PWA ohne Fehler.
```

```{wp} Observability
:id: WP_MVP_13
:status: open
:milestone: M4
:depends: WP_MVP_01
:covers: REQ_OPS_004, SPEC_OPS_OTEL

OpenTelemetry-Layer, fachliche Metriken gemäß {doc}`mvp-detail-design`,
Log-Felder (Request-ID, Haushalt, Nutzer).

**Abnahme:** Traces in einem lokalen OTel-Collector (compose-Override)
sichtbar; Metriken unter `/metrics`.
```

```{wp} Betrieb und Auslieferung
:id: WP_MVP_14
:status: open
:milestone: M4
:depends: WP_MVP_06, WP_MVP_13
:covers: REQ_OPS_001, REQ_OPS_003, REQ_OPS_005, REQ_OPS_006, REQ_OPS_007

Helm-Chart und compose um neue Konfiguration erweitern (Sitzung, Upload,
Bild, Housekeeping, OTel, S3-Zugangsdaten); compose-Overrides für Keycloak,
MinIO und OTel-Collector; CI-Smoke-Test mit compose; Betriebsdoku.

**Abnahme:** `helm lint` aller Varianten; Smoke-Test in der CI grün;
Installation mit CNPG in einem Test-Cluster.
```

```{wp} Qualitätssicherung und Abschluss
:id: WP_MVP_15
:status: open
:milestone: M4
:depends: WP_MVP_07, WP_MVP_08, WP_MVP_09, WP_MVP_10, WP_MVP_11, WP_MVP_12, WP_MVP_14
:covers: REQ_NFR_006, REQ_NFR_008, SPEC_PERF_BUDGET

Leistungstest mit Seed-Daten, Sicherheitsreview (CSRF, Tenant, Uploads,
Header), Barrierefreiheit Grundprüfung (Tastatur, Kontrast), Trace-Review
(jede MVP-Spezifikation mit Implementierung und Test), Status der Needs
aktualisieren.

**Abnahme:** {need}`SPEC_PERF_BUDGET` eingehalten; keine offenen Befunde der
Schwere hoch; Lückenlisten unter {doc}`../nachverfolgung/index` ohne
MVP-Einträge.
```

## Abhängigkeiten

```{mermaid}
flowchart LR
  WP01[01 Hexagonale Struktur, Web] --> WP02[02 Ports, UoW, DB-Adapter]
  WP01 --> WP03[03 Login und Haushalte]
  WP02 --> WP03
  WP03 --> WP04[04 Messstellen] --> WP05[05 Formular]
  WP01 --> WP06[06 Objektspeicher und Bilder]
  WP02 --> WP06
  WP05 --> WP07[07 Foto mit AI]
  WP06 --> WP07
  WP05 --> WP08[08 Auswertung] --> WP09[09 Dashboard]
  WP05 --> WP10[10 CSV-Import]
  WP06 --> WP10
  WP01 --> WP11[11 Englisch]
  WP01 --> WP12[12 PWA]
  WP01 --> WP13[13 Observability] --> WP14[14 Betrieb]
  WP06 --> WP14
  WP07 & WP09 & WP10 & WP11 & WP12 & WP14 --> WP15[15 Qualitätssicherung]
```

Kritischer Pfad: 01 → 02 → 03 → 04 → 05 → 08 → 09 → 15. WP_MVP_06
läuft parallel zu WP_MVP_03 bis WP_MVP_05, WP_MVP_11 bis WP_MVP_13 parallel zu
M2 und M3, sofern eine zweite Person verfügbar ist.

## Definition of Done (je Arbeitspaket)

1. Code entspricht {need}`SPEC_ARCH_LAYERS`; `cargo fmt`, `cargo clippy -D warnings`
   (inkl. `clippy.toml`) und alle Tests grün, Repository-Tests auf SQLite und
   PostgreSQL.
2. Jede umgesetzte Funktion trägt `// @impl SPEC_…`, jeder Test `// @test SPEC_…`;
   Status der Spezifikationen auf `implemented`, des Arbeitspakets auf `done`.
3. Neue Texte in `de` (ab WP_MVP_11 auch `en`); keine Inline-Skripte oder
   `style`-Attribute.
4. Doku-Build mit `-W` grün; Konfigurations- und Betriebsdoku aktualisiert.
5. Abnahmekriterien des Arbeitspakets erfüllt und im Merge-Request belegt.

## Risiken

| Risiko | Auswirkung | Maßnahme |
|---|---|---|
| Erkennungsqualität der AI unbekannt | Nutzen der Foto-Erfassung | Probe in WP_MVP_07, Korrekturquote als Metrik, Modell konfigurierbar |
| Unterschiede zwischen OIDC-Providern (Claims, Issuer-Formate) | Login schlägt bei einzelnen Providern fehl | Tests mit Keycloak und Authentik; Anzeigename mit Fallback auf `email` bzw. `sub` |
| SeaORM 2.0 ist eine neue Hauptversion | API-Änderungen in Patch-Releases, Fehler | Repository-Schicht kapselt SeaORM; Version pinnen; Tests auf beiden Backends |
| Größe von ECharts (ca. 1 MB, ca. 350 KB komprimiert) | Ladezeit mobil | Laden nur auf Seiten mit Diagramm, `immutable`-Caching |
| iOS-Uploads im HEIC-Format | Fehlermeldung statt Erkennung | `accept`-Attribut mit JPEG/PNG/WebP, Hinweistext; Beobachtung über Fehlermetrik |
| Ein Nutzer je Haushalt | Haushalte mit mehreren Personen im MVP nicht abbildbar | Einladungen früh in v1 einplanen |

## Entscheidungen

| Nr. | Frage | Entscheidung |
|---|---|---|
| E1 | Löschen von Ablesungen ohne Historie im MVP? | Ja ({need}`SPEC_RDG_DELETE`); Historie mit {need}`REQ_RDG_003` in v1 |
| E2 | Gelerntes Saisonprofil ({need}`REQ_ANA_002`) im MVP? | Nein; v1. Der MVP verwendet die Gleichverteilung ({need}`SPEC_FC_CURRENT_YEAR`) |
| E3 | Einladungen ({need}`REQ_TEN_004`) im MVP? | Nein; v1. Im MVP hat jeder Haushalt genau einen Nutzer |
| E4 | HEIC-Unterstützung? | Nein ({need}`ADR_017`) |
| E5 | Fahrzeuge mit Elektro-Antrieb ({need}`REQ_CAR_007` bis {need}`REQ_CAR_010`) im MVP? | Nein; v1, wie alle Fahrzeugfunktionen |

## Abdeckung

MVP-Anforderungen ohne Arbeitspaket:

```{needtable}
:filter: type == 'req' and release == 'MVP' and len(covers_back) == 0
:columns: id, title, priority
:style: table
```

Arbeitspakete:

```{needtable}
:types: wp
:columns: id, title, milestone, status, depends, covers
:style: table
:sort: id
```

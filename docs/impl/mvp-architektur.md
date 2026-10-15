# MVP: Architektur

Geltungsbereich: alle Anforderungen mit `release: MVP` (siehe {doc}`MVP`).
Die Architektur ist hexagonal ({need}`ADR_022`). Diese Seite beschreibt Kern,
Ports, Adapter, Laufzeitverhalten und querschnittliche Konzepte der
MVP-Phase. Schnittstellen, Datenmodell und Algorithmen im Detail stehen in
{doc}`mvp-detail-design`.

## Architekturtreiber

| Treiber | Quelle | Konsequenz für die Architektur |
|---|---|---|
| Mandantentrennung | {need}`REQ_TEN_001` | `Tenant` im Kern als Berechtigungsnachweis, Pflichtparameter aller fachlichen Ports |
| Rechengenauigkeit | {need}`REQ_NFR_005` | Dezimalarithmetik in `zaehler-domain`, Festkomma im DB-Adapter, keine Berechnungen in SQL |
| Austauschbare Adapter | {need}`REQ_OPS_002`, {need}`REQ_OPS_010` | Ports im Kern; SQLite/PostgreSQL und Dateisystem/S3 als Adapter; In-Memory-Adapter für Tests |
| Sicherheit | {need}`REQ_NFR_003` | CSRF-Token, Sicherheitsheader, Upload-Validierung im Web-Adapter, parametrisierte Abfragen im DB-Adapter |
| Datenschutz | {need}`REQ_NFR_004`, {need}`REQ_AI_006` | Metadaten aus Fotos entfernen, AI-Port optional, Haushalt vollständig löschbar |
| Antwortzeiten | {need}`REQ_NFR_006` | Auswertungen pro Request im Speicher; Datenmengen je Haushalt im Bereich von Tausenden Ablesungen |
| Mehrere Replicas | {need}`REQ_OPS_008` (v1) | zustandsloser Server: Sitzungen in der DB, Dateien im Objektspeicher |

## Bausteinsicht

### Ebene 1: Kern, Adapter, Composition Root

Crates und Abhängigkeiten: {doc}`../architektur/bausteine`. Gegenüber dem
Grundgerüst neu sind `zaehler-app`, `zaehler-web`, `zaehler-storage` und
`zaehler-media`; `zaehler-server` wird auf die Composition Root reduziert.
`zaehler-domain` erhält die Module `calendar`, `series` und `analysis` sowie
die Abhängigkeit `chrono-tz`.

```{spec} Hexagonale Abhängigkeitsregeln
:id: SPEC_ARCH_LAYERS
:status: implemented
:links: ADR_022

- `zaehler-domain` hängt von keinem Crate des Workspace ab.
- `zaehler-app` hängt nur von `zaehler-domain` ab (dazu `async-trait`,
  `thiserror`, `csv`, die Fassaden `tracing` und `metrics`); keine
  Framework-, Datenbank- oder HTTP-Bibliotheken.
- Adapter (`zaehler-web`, `zaehler-db`, `zaehler-storage`, `zaehler-media`,
  `zaehler-ai`, `zaehler-notify`) hängen von `zaehler-app` und
  `zaehler-domain` ab, nie voneinander.
- Nur `zaehler-server` hängt von Adaptern ab.
- Anwendungsfälle und Ports verwenden ausschließlich Kerntypen; Adapter bilden
  eigene Typen (SeaORM-Entities, OpenRouter-Antworten, Axum-Requests) an der
  Grenze ab.
- Ein CI-Schritt prüft die Abhängigkeiten aus `cargo metadata` gegen diese
  Regeln und bricht bei Verstoß ab.
```

### Ebene 2: Ports

Driving Ports sind die öffentlichen Anwendungsfälle von `zaehler-app`
(Tabelle in {doc}`mvp-detail-design`). Driven Ports:

| Port | Zweck | Adapter (MVP) | Test-Adapter |
|---|---|---|---|
| `UnitOfWorkFactory`, `UnitOfWork` | Transaktion, Zugriff auf Repositories | `zaehler-db` | In-Memory |
| `MembershipRepository`, `HouseholdRepository`, `UserRepository` | Haushalte, Nutzer, Mitgliedschaften | `zaehler-db` | In-Memory |
| `MeterRepository`, `ReadingRepository`, `DraftRepository` | Messstellen, Ablesungen, AI-Entwürfe (`DraftRepository` ab WP_MVP_07) | `zaehler-db` | In-Memory |
| `MaintenanceRepository` | Abfragen ohne Haushaltsbezug für Housekeeping (ab WP_MVP_06) | `zaehler-db` | In-Memory |
| `BlobStore` | Fotos, temporäre Dateien | `zaehler-storage` (Dateisystem, S3) | In-Memory |
| `ImageNormalizer` | Foto prüfen, ausrichten, verkleinern, als JPEG ohne Metadaten kodieren | `zaehler-media` | Stub |
| `MeterRecognizer` | Zählerstand aus Foto erkennen | `zaehler-ai` | Stub |
| `Clock`, `IdGenerator` | aktuelle Zeit, UUIDv7 | `SystemClock`, `UuidV7` in `zaehler-app` | `FixedClock`, `RandomIds` |
| `Readiness` | Bereitschaft für `/readyz` | `zaehler-db` | In-Memory |

Nicht über Ports, weil Teil des Web-Adapters: Sitzungen, CSRF, OIDC-Ablauf,
i18n und Templates. Der Sitzungsspeicher (`tower_sessions::SessionStore`) ist
in `zaehler-db` implementiert und wird in der Composition Root an
`zaehler-web` übergeben.

### Ebene 2: `zaehler-web`

| Modul | Verantwortung |
|---|---|
| `router` | Routen, Middleware-Reihenfolge |
| `extract` | `Ctx` (Nutzer, Tenant über `ResolveTenant`, Seitenkontext, CSRF-Token, HTMX-Erkennung), `AnonLang` |
| `pages/*` | Handler je Seite: Eingabe parsen, Anwendungsfall aufrufen, View-Modell rendern |
| `views/*` | View-Modelle, Formatierung |
| `auth` | OIDC-Client, Dev-Login, CSRF-Prüfung |
| `security` | Sicherheitsheader, Upload-Limits |
| `i18n` | Fluent-Ressourcen, Sprachauswahl, Zahl- und Datumsformat, Parsen lokalisierter Dezimalzahlen |
| `error` | Abbildung von `AppError` auf HTTP-Antworten |
| `assets` | eingebettete statische Dateien |

## Laufzeitsicht

### Anmeldung

```{mermaid}
sequenceDiagram
  actor U as Browser
  participant W as zaehler-web
  participant I as OIDC-Provider
  participant A as zaehler-app: SignIn
  participant P as Ports (UnitOfWork)
  U->>W: GET /auth/login
  W-->>U: 302 zum Provider (PKCE, state, nonce in Sitzung)
  U->>I: Login
  I-->>U: 302 /auth/callback?code&state
  U->>W: GET /auth/callback
  W->>I: Code tauschen, ID-Token prüfen
  W->>A: sign_in(issuer, subject, Anzeigename, E-Mail)
  A->>P: Nutzer laden oder anlegen, ohne Mitgliedschaft Haushalt anlegen
  A-->>W: Nutzer, aktiver Haushalt
  W-->>U: Sitzung (Cookie), 303 /
```

### Erfassung per Formular

```{mermaid}
sequenceDiagram
  actor U as Browser
  participant W as zaehler-web
  participant A as zaehler-app: RecordReadings
  participant D as zaehler-domain
  participant P as Ports (UnitOfWork, Repositories)
  U->>W: POST /meters/{id}/readings (CSRF, Werte je Register)
  W->>A: execute(tenant, Eingabe)
  A->>P: begin, letzte Ablesung je Register
  A->>D: check_plausibility
  alt Warnung und nicht bestätigt
    A-->>W: AppError::Validation (Warnungen)
    W-->>U: Formular mit Hinweisen und Bestätigungsfeld
  else
    A->>P: Ablesungen speichern, commit
    W-->>U: 303 zur Messstelle
  end
```

### Foto-Erfassung

```{mermaid}
sequenceDiagram
  actor U as Browser
  participant W as zaehler-web
  participant A as zaehler-app: AnalysePhoto, ConfirmDraft
  participant M as ImageNormalizer
  participant B as BlobStore
  participant R as MeterRecognizer
  participant P as UnitOfWork
  U->>W: POST /capture (multipart)
  W->>A: analyse(tenant, bytes)
  A->>M: normalize
  A->>B: put(drafts/...)
  A->>R: recognize(JPEG)
  R-->>A: Vorschlag
  A->>P: Zuordnung über Zählernummer, Entwurf speichern, commit
  A-->>W: Entwurf mit Vorschlag und Plausibilität
  W-->>U: HTMX-Fragment Bestätigung
  U->>W: POST /capture/{draft}/confirm
  W->>A: confirm(tenant, draft, Werte)
  A->>B: copy drafts/... nach readings/...
  A->>P: Ablesung speichern, Entwurf abschließen, commit
  A->>B: delete drafts/...
  W-->>U: 303 zur Messstelle
```

### Auswertung

Der Anwendungsfall `MeterAnalysis` lädt über Ports alle Ablesungen einer
Messstelle, bildet mit `zaehler-domain` Intervalle, Monatsserien, Vergleiche
und die Prognose und gibt ein Ergebnisobjekt zurück; `zaehler-web` erzeugt
daraus View-Modell und JSON für Diagramme. Es gibt keinen Zwischenspeicher.

## Verteilungssicht

Unverändert gegenüber {doc}`../architektur/kontext`, ergänzt um:

| Element | MVP |
|---|---|
| Sitzungen | Tabelle `sessions` in derselben Datenbank |
| Fotos | Objektspeicher unter `households/{household}/readings/{reading}.jpg`, Entwürfe unter `households/{household}/drafts/{draft}.jpg` |
| Temporäre CSV-Dateien | Objektspeicher unter `households/{household}/tmp/{uuid}.csv` |
| Housekeeping | Anwendungsfall in `zaehler-app`, ausgelöst von einem Tokio-Intervall in `zaehler-server`; idempotent, daher ohne Leader-Lock |
| OpenTelemetry | optionaler OTLP-Export (gRPC oder HTTP) an einen Collector |

## Querschnittliche Konzepte

```{spec} Unit of Work
:id: SPEC_ARCH_UOW
:status: implemented
:satisfies: REQ_NFR_005, REQ_OPS_002
:links: ADR_022

`UnitOfWorkFactory::begin() -> Box<dyn UnitOfWork>`; `UnitOfWork` liefert die
Repositories (`meters()`, `readings()`, `drafts()`, …), die alle in derselben
Transaktion arbeiten, sowie `commit()`. Ohne `commit` wird beim Drop
zurückgerollt. Lesende Anwendungsfälle verwenden ebenfalls eine Unit of Work
(konsistenter Lesestand). Operationen auf dem Objektspeicher sind nicht
transaktional: Anwendungsfälle schreiben neue Objekte vor dem `commit` und
löschen alte erst danach; verwaiste Objekte entfernt das Housekeeping. Der
DB-Adapter implementiert die Unit of Work über `DatabaseTransaction` von
SeaORM; der In-Memory-Adapter über eine Kopie des Zustands, die beim `commit`
übernommen wird.
```

```{spec} Mandantentrennung über den Typ Tenant
:id: SPEC_SEC_TENANT_SCOPE
:status: implemented
:satisfies: REQ_TEN_001, REQ_NFR_003
:links: ADR_022

`zaehler_app::Tenant { user_id, household_id, role }` hat private Felder und
entsteht ausschließlich im Anwendungsfall `ResolveTenant` nach Prüfung der
Mitgliedschaft über `MembershipRepository` (gewählter Haushalt, sonst erster
Haushalt des Nutzers; ohne Mitgliedschaft wird ein Haushalt angelegt). Alle fachlichen Anwendungsfälle
und Repository-Ports nehmen `&Tenant`. Adapter filtern jede Abfrage über
`household_id`; Kind-Tabellen (`registers`, `readings`) über einen Join auf
`meters.household_id`. IDs aus der URL werden nur zusammen mit dem Tenant
aufgelöst. Eine gemeinsame Port-Testsuite in `zaehler-app` (Feature
`testing`) prüft Tenant-Isolation für jede Implementierung (In-Memory,
SQLite, PostgreSQL).
```

```{spec} Sitzungen und CSRF-Schutz
:id: SPEC_SEC_SESSION_CSRF
:status: implemented
:satisfies: REQ_TEN_002, REQ_NFR_003
:links: ADR_016

Sitzungen über `tower-sessions` in `zaehler-web`; der `SessionStore` auf der
Tabelle `sessions` liegt in `zaehler-db` (beide Backends). Cookie
`zaehler_session`: `HttpOnly`, `SameSite=Lax`, `Secure` (Standard aus dem
Schema von `server.public_url`: `https://` ergibt `Secure`; explizit über
`session.cookie_secure`; ohne `Secure` bei nicht lokaler URL protokolliert der
Server beim Start eine Warnung), Laufzeit konfigurierbar (Standard 30 Tage,
gleitend).
Nach Login wird die Sitzungs-ID erneuert. Je Sitzung ein zufälliges
CSRF-Token (32 Byte, Base64URL). Alle Requests außer `GET`, `HEAD`, `OPTIONS`
müssen das Token im Header `X-CSRF-Token` (HTMX, gesetzt über `hx-headers` am
`body`) oder im Formularfeld `_csrf` enthalten; Vergleich in konstanter Zeit.
Zusätzlich wird der `Origin`-Header, falls vorhanden, gegen
`server.public_url` geprüft.
```

```{spec} Sicherheitsheader
:id: SPEC_SEC_HEADERS
:status: implemented
:satisfies: REQ_NFR_003

Für alle HTML-Antworten von `zaehler-web`: `Content-Security-Policy:
default-src 'self'; script-src 'self'; style-src 'self'; img-src 'self' data:
blob:; connect-src 'self'; frame-ancestors 'none'; form-action 'self';
base-uri 'none'`, `X-Content-Type-Options: nosniff`,
`Referrer-Policy: same-origin`, `Permissions-Policy: camera=(self),
geolocation=()`. HTMX wird über `<meta name="htmx-config">` mit
`includeIndicatorStyles: false` und `allowEval: false` konfiguriert; Templates
enthalten keine Inline-Skripte und keine `style`-Attribute. HSTS setzt der
Ingress.
```

```{spec} Upload-Verarbeitung
:id: SPEC_SEC_UPLOADS
:status: draft
:satisfies: REQ_NFR_003, REQ_RDG_004, REQ_IMP_001

Body-Limits je Route in `zaehler-web`: Fotos `upload.max_photo_bytes`
(Standard 15 MiB), CSV `upload.max_csv_bytes` (Standard 5 MiB), sonst 64 KiB.
Der Dateityp wird über Magic Bytes bestimmt, nicht über Dateiname oder
`Content-Type`: erlaubt sind JPEG, PNG und WebP bzw. UTF-8-Text für CSV.
Hochgeladene Dateien werden nie unverändert ausgeliefert: Fotos durchlaufen
{need}`SPEC_IMG_PROCESSING`, CSV wird nur geparst. Auslieferung von Fotos
ausschließlich über einen Anwendungsfall mit Tenant-Prüfung und
`Content-Type: image/jpeg`.
```

```{spec} Objektspeicher-Port
:id: SPEC_STORE_BLOBS
:status: draft
:satisfies: REQ_OPS_010, REQ_RDG_004
:links: ADR_006

Port `BlobStore` in `zaehler-app` mit `put(key, bytes)`, `get(key)`,
`copy(from, to)`, `delete(key)`, `delete_prefix(prefix)` und
`list_older_than(prefix, zeitpunkt)`. Schlüssel als Typ `BlobKey`, gebildet
nur über Konstruktoren im Kern (`BlobKey::reading_photo(household, reading)`,
`BlobKey::draft_photo(...)`, `BlobKey::tmp(...)`), immer mit Präfix
`households/{household_id}/`. Adapter `zaehler-storage` über `object_store`
(`LocalFileSystem`, `AmazonS3`), Auswahl über `storage.kind`; In-Memory-Adapter
für Tests.
```

```{spec} Zeitzone des Haushalts
:id: SPEC_TZ
:status: implemented
:satisfies: REQ_ANA_001, REQ_RDG_001
:links: ADR_019

Zeitpunkte werden in UTC gespeichert. Kalendertage, Monate und Jahre beziehen
sich auf die IANA-Zeitzone des Haushalts (`households.timezone`, Standard
`Europe/Berlin`). `zaehler_domain::calendar::Calendar` bildet ein Datum auf den
UTC-Zeitpunkt seines lokalen Tagesbeginns ab; existiert Mitternacht wegen einer
Zeitumstellung nicht, gilt der früheste gültige Zeitpunkt des Tages. Eingaben
im Formular sind lokale Zeitpunkte des Haushalts.
```

```{spec} Internationalisierung
:id: SPEC_I18N
:status: implemented
:satisfies: REQ_NFR_002
:links: ADR_015

Texte in `crates/zaehler-web/locales/{de,en}/main.ftl` über
`fluent-templates` (eingebettet). Sprache: Nutzerprofil (`users.locale`), sonst
`Accept-Language`, sonst `de`. View-Modelle erhalten ein `Lang`-Objekt; Templates
rufen `lang.t("schluessel")` bzw. `lang.t_args(...)` auf. Zahlen und Datumswerte
formatiert `i18n::format` für `de` (Dezimalkomma, `TT.MM.JJJJ`) und `en`
(Dezimalpunkt, `YYYY-MM-DD`). Eingaben akzeptieren Komma und Punkt als
Dezimaltrenner; Tausendertrennzeichen werden abgelehnt. Anwendungsfälle
liefern Fehler und Warnungen als Codes mit Argumenten; die Übersetzung erfolgt
im Web-Adapter. Ein Test prüft, dass beide Sprachdateien dieselben Schlüssel
enthalten.
```

```{spec} Fehlerbehandlung
:id: SPEC_ERR_HANDLING
:status: implemented
:satisfies: REQ_NFR_003, REQ_TEN_001

`zaehler_app::AppError` mit den Varianten `NotFound`, `Forbidden`,
`Validation(Feldfehler)`, `Conflict`, `Unauthenticated`, `Upstream` (AI,
Objektspeicher) und `Internal`; Port-Fehler der Adapter werden darauf
abgebildet. Zugriffe auf IDs anderer Haushalte ergeben `NotFound` (keine
Unterscheidung zu nicht existierenden IDs). `zaehler-web` bildet ab:
`Validation` rendert das Formular erneut mit Feldfehlern (HTTP 422);
`Internal` und `Upstream` liefern eine generische Fehlerseite mit Request-ID,
Details nur im Log; `Unauthenticated` leitet bei HTML-Requests auf
`/auth/login` um und antwortet bei HTMX-Requests mit `HX-Redirect`.
```

```{spec} OpenTelemetry-Export
:id: SPEC_OPS_OTEL
:status: draft
:satisfies: REQ_OPS_004

`zaehler-server` aktiviert bei gesetztem `otel.endpoint` oder
`OTEL_EXPORTER_OTLP_ENDPOINT` einen `tracing-opentelemetry`-Layer mit
OTLP-Export. Spans: HTTP-Request (Route-Template, Status), Anwendungsfall,
Port-Aufruf je Adapter (AI mit Modell, Dauer, Ergebnis, ohne Bildinhalt;
Objektspeicher; Datenbank-Transaktion). W3C-Trace-Context aus eingehenden
Headern wird übernommen. Ohne Endpunkt ist der Layer nicht aktiv.
```

```{spec} Housekeeping
:id: SPEC_OPS_HOUSEKEEPING
:status: draft
:satisfies: REQ_NFR_004, REQ_OPS_010

Anwendungsfall `Housekeeping` in `zaehler-app` über
`MaintenanceRepository` und `BlobStore`, ausgelöst von einem Tokio-Intervall
in `zaehler-server` (`housekeeping.interval_minutes`, Standard 60) in jeder
Instanz: offene AI-Entwürfe älter als `housekeeping.draft_ttl_hours`
(Standard 24) werden als `discarded` markiert und ihre Fotos gelöscht;
Objekte unter `households/*/tmp/` älter als 24 Stunden werden gelöscht;
abgelaufene Sitzungen werden entfernt. Jede Operation ist idempotent und
verträgt parallele Ausführung in mehreren Instanzen.
```

## Architekturentscheidungen der MVP-Phase

```{needtable}
:filter: type == 'adr' and id in ['ADR_015', 'ADR_016', 'ADR_017', 'ADR_019', 'ADR_022']
:columns: id, title, status
:style: table
```

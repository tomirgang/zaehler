# v1: Architektur

Geltungsbereich: alle Anforderungen mit `release: v1` (siehe {doc}`V1`).
Grundlage bleibt die hexagonale Architektur ({need}`ADR_022`) und die
MVP-Architektur ({doc}`mvp-architektur`); diese Seite beschreibt nur
Erweiterungen und Änderungen. Schnittstellen, Datenmodell und Algorithmen im
Detail stehen in {doc}`v1-detail-design`.

## Architekturtreiber

| Treiber | Quelle | Konsequenz für die Architektur |
|---|---|---|
| Mehrere Personen je Haushalt | {need}`REQ_TEN_003`, {need}`REQ_TEN_004` | Rollenprüfung in jedem schreibenden Anwendungsfall, auch über die API; Mitgliedschaften und Einladungen als eigene Ports |
| Nachvollziehbarkeit | {need}`REQ_RDG_003` | Änderungen an Ablesungen nur über Anwendungsfälle, die im selben Commit eine Revision schreiben |
| Kostenrechnung über Zeiträume | {need}`REQ_TAR_001` bis {need}`REQ_TAR_011`, {need}`REQ_FC_002` | Tarifmodell nach {need}`ADR_008` im Kern; Rechnung vollständig in `zaehler-domain`, Repositories liefern nur Stammdaten und Ablesungen |
| Abhängigkeiten zwischen Messstellen | {need}`REQ_MTR_008`, {need}`REQ_CAR_008`, {need}`REQ_PV_002` | Zwischenzähler und PV-Zuordnung als Beziehungen zwischen Messstellen; Auswertungen laden zusammengehörige Messstellen gemeinsam |
| Hintergrundverarbeitung | {need}`REQ_NTF_001`, {need}`REQ_NTF_004`, {need}`REQ_PV_008`, {need}`REQ_OPS_008` | Scheduler mit Leader-Lock ({need}`ADR_010`), Outbox für Ereignisse aus Anwendungsfällen ({need}`ADR_024`) |
| Maschinelle Clients | {need}`REQ_API_001` bis {need}`REQ_API_003` | zweiter Driving-Adapter-Teil in `zaehler-web` unter `/api/v1` mit Bearer-Token, ohne Sitzung und CSRF ({need}`ADR_023`) |
| Dokumente statt Fotos | {need}`REQ_AI_004`, {need}`REQ_AI_005` | PDF als neuer Upload-Typ, gespeichert ohne Normalisierung, ausgeliefert nur als Download |
| Antwortzeiten | {need}`REQ_NFR_006` | Kosten-, Fahrzeug- und PV-Auswertungen weiterhin pro Request im Speicher; Leistungsbudget um diese Seiten erweitert |

## Bausteinsicht

### Ebene 1: Änderungen an Crates

Die Crate-Struktur bleibt unverändert ({doc}`../architektur/bausteine`). Neu
oder geändert:

| Crate | Änderung |
|---|---|
| `zaehler-domain` | neue Module `cost` (Kostenrechnung über Zeiträume), `vehicle` (Energie, Verbrauch, Kosten pro km; ersetzt `car`), `pv` (Kennzahlen, Erwartung, Wirtschaftlichkeit), `schedule` (Fälligkeit), `assets` (Kostenbuch, Wiederholungen); `tariff`, `forecast`, `expectation` werden erweitert |
| `zaehler-app` | Ports für Mitglieder, Einladungen, Revisionen, Verträge, Fahrzeuge, Kostenbuch, PV, Outbox, Jobs, API-Tokens, `Notifier`, `LeaderLock`, `DocumentExtractor`; Anwendungsfälle je Bereich |
| `zaehler-db` | Migrationen v1, Repository-Implementierungen, `PgLeaderLock` |
| `zaehler-notify` | implementiert den Port `Notifier` aus `zaehler-app` mit `lettre`; eigene Fluent-Texte für E-Mails. Der bisherige Trait `Notifier` im Crate entfällt, da er gegen {need}`SPEC_ARCH_LAYERS` verstößt (Ports gehören in den Kern) |
| `zaehler-ai` | implementiert zusätzlich `DocumentExtractor` (Rechnung, Beleg) |
| `zaehler-web` | Module `api` (REST, OpenAPI), `pages` für Mitglieder, Verträge, Kosten, Fahrzeuge, PV, Benachrichtigungen |
| `zaehler-server` | Scheduler mit Leader-Lock und Job-Registrierung, SMTP-Konfiguration |

### Ebene 2: neue Driven Ports

| Port | Zweck | Adapter | Test-Adapter |
|---|---|---|---|
| `MembershipRepository` (erweitert), `InvitationRepository` | Mitglieder, Rollen, Einladungen | `zaehler-db` | In-Memory |
| `ReadingRepository` (erweitert), `RevisionRepository` | Korrektur, Löschen mit Historie | `zaehler-db` | In-Memory |
| `ContractRepository` | Verträge, Kostenpositionen, Preisstände, Abschläge, Anpassungen, Abrechnungen | `zaehler-db` | In-Memory |
| `VehicleRepository`, `AssetCostRepository`, `ChargingRepository` | Fahrzeuge, Kostenbuch, externes Laden | `zaehler-db` | In-Memory |
| `PvRepository` | PV-Anlagen und Zuordnung | `zaehler-db` | In-Memory |
| `DocumentRepository` | Metadaten hochgeladener Dokumente | `zaehler-db` | In-Memory |
| `OutboxRepository` | Ereignisse schreiben (im Anwendungsfall) und abholen (im Job) | `zaehler-db` | In-Memory |
| `JobRepository` | Abfragen ohne Haushaltsbezug für Jobs: fällige Messstellen, Empfänger, Versandprotokoll, `job_runs` | `zaehler-db` | In-Memory |
| `NotificationSettingsRepository` | E-Mail-Adresse und Ereignistypen je Nutzer | `zaehler-db` | In-Memory |
| `ApiTokenRepository` | API-Tokens (nur Hash) | `zaehler-db` | In-Memory |
| `Notifier` | E-Mail versenden | `zaehler-notify` (SMTP) | Aufzeichnender Stub |
| `LeaderLock` | genau eine aktive Instanz für Jobs | `zaehler-db` (`pg_try_advisory_lock`; SQLite: immer Leader) | Schalter |
| `DocumentExtractor` | Rechnung und Beleg strukturiert auslesen | `zaehler-ai` | Stub |

`JobRepository` ist neben `MaintenanceRepository` der zweite Port ohne
`Tenant`. Er liefert nur Haushalts-IDs und Nutzer-IDs; fachliche Daten liest
ein Job über die normalen Ports mit einem Job-Tenant
({need}`SPEC_JOB_OUTBOX`).

### Ebene 2: `zaehler-web`

| Modul | Neu in v1 |
|---|---|
| `api` | Router `/api/v1`, Extraktor `ApiTenant` (Bearer), JSON-Fehlerformat, OpenAPI über `utoipa` |
| `pages/members`, `pages/invite` | Mitgliederverwaltung, Annahme von Einladungen |
| `pages/contracts`, `pages/costs` | Verträge mit Vorlagen, Kostenansicht je Messstelle |
| `pages/vehicles`, `pages/assets` | Fahrzeuge, Kostenbuch, Ladevorgänge |
| `pages/pv` | PV-Anlagen, Hausanschluss-Auswertung, Sammelerfassung |
| `pages/documents` | Upload und Bestätigung von Rechnungen und Belegen |
| `pages/notifications` | Benachrichtigungseinstellungen |

## Laufzeitsicht

### Ereignis aus einem Anwendungsfall bis zur E-Mail

```{mermaid}
sequenceDiagram
  actor U as Browser oder API
  participant A as zaehler-app: RecordReadings
  participant P as UnitOfWork
  participant S as Scheduler (Leader)
  participant J as zaehler-app: DispatchOutbox
  participant N as Notifier
  U->>A: Ablesung speichern
  A->>P: Ablesungen und Outbox-Eintrag readings_recorded
  A->>P: commit
  S->>J: Tick (1 min)
  J->>P: Einträge abholen (pending, älteste zuerst)
  J->>J: Ampel und PV-Schwelle prüfen
  J->>N: E-Mail je Empfänger
  J->>P: Versandprotokoll, Eintrag erledigt, commit
```

Die E-Mail wird nie im Request versendet. Ein Fehler beim Versand verzögert
nur die Benachrichtigung; der Eintrag bleibt mit Fehlerzähler offen
({need}`SPEC_JOB_OUTBOX`).

### Kostenansicht einer Messstelle

```{mermaid}
sequenceDiagram
  actor U as Browser
  participant W as zaehler-web
  participant A as zaehler-app: MeterCosts
  participant P as Ports
  participant D as zaehler-domain
  U->>W: GET /meters/{id}/costs
  W->>A: execute(tenant, meter, Stichtag)
  A->>P: Messstelle, Ablesungen, Verträge, ggf. übergeordnete Messstelle und deren Vertrag
  A->>D: Monatsserie, Saisonprofil, Prognose
  A->>D: cost::calculate je Teilzeitraum (Preisstände, Staffeln, Anpassungen)
  A->>D: Abschläge, Saldo, Vorschlag
  A-->>W: Kostenergebnis (Ist, Prognose, Saldo, Abrechnung)
  W-->>U: Seite mit Tabelle und Diagramm
```

### Anfrage über die REST-API

```{mermaid}
sequenceDiagram
  participant C as Client (Home Assistant)
  participant W as zaehler-web: api
  participant T as zaehler-app: ResolveApiTenant
  participant A as zaehler-app: IngestReadings
  C->>W: POST /api/v1/readings (Authorization: Bearer)
  W->>T: Token-Hash prüfen, Mitgliedschaft und Rolle
  T-->>W: Tenant (Rolle = min(Token, Mitgliedschaft))
  W->>A: execute(tenant, token, Stände)
  A-->>W: je Stand gespeichert, ersetzt oder verworfen
  W-->>C: 200 JSON
```

## Verteilungssicht

| Element | v1 |
|---|---|
| Scheduler | in jeder Replica gestartet, aktiv nur mit Leader-Lock ({need}`SPEC_JOB_SCHEDULER`); Housekeeping wird ein Job des Schedulers |
| SMTP | externer Mail-Server; ohne Konfiguration sind Benachrichtigungen deaktiviert ({need}`ADR_011`); `compose.mail.yaml` mit Mailpit für Entwicklung und Smoke-Test |
| Dokumente | Objektspeicher unter `households/{household}/documents/{document}.pdf` bzw. `.jpg` |
| REST-API | gleicher Listener und Ingress wie die Oberfläche, Pfad `/api/v1` |
| Mehrere Replicas | PostgreSQL und S3; Migration unter Advisory-Lock (vorhanden), Jobs unter Leader-Lock |

## Querschnittliche Konzepte

```{spec} Rollen im Haushalt
:id: SPEC_TEN_ROLES
:status: implemented
:satisfies: REQ_TEN_003, REQ_NFR_003
:links: SPEC_SEC_TENANT_SCOPE

`Role` ist geordnet: `Viewer` < `Member` < `Owner`. Jeder Anwendungsfall
prüft zu Beginn `tenant.require(min)`; die Mindestrolle steht in der Tabelle
der Anwendungsfälle in {doc}`v1-detail-design`. Regeln:

- `Viewer`: nur lesende Anwendungsfälle, Benachrichtigungseinstellungen und
  Sprache des eigenen Nutzers, Austritt aus dem Haushalt.
- `Member`: zusätzlich Ablesungen erfassen, korrigieren und löschen,
  Foto-Erfassung, CSV-Import, Kostenbuch, Ladevorgänge, Belege.
- `Owner`: zusätzlich Messstellen, Verträge, Fahrzeuge, PV-Anlagen anlegen,
  ändern und löschen, Mitglieder und Einladungen verwalten, Haushalt
  exportieren und löschen.
- Ein Haushalt hat immer mindestens einen `Owner`: Herabstufen, Entfernen
  und Austritt des letzten `Owner` ergeben `AppError::Conflict(LastOwner)`.
  Die Prüfung läuft in der Unit of Work nach dem Lesen der Mitgliedschaften
  (PostgreSQL: Zeilensperre auf `memberships` des Haushalts).
- Die Oberfläche blendet Aktionen oberhalb der eigenen Rolle aus; maßgeblich
  ist die Prüfung im Anwendungsfall.
```

```{spec} Outbox und Job-Tenant
:id: SPEC_JOB_OUTBOX
:status: implemented
:satisfies: REQ_NTF_004, REQ_PV_008, REQ_OPS_008
:links: ADR_024, SPEC_JOB_SCHEDULER

Anwendungsfälle, die Folgeverarbeitung auslösen, schreiben einen Eintrag in
`outbox` in derselben Unit of Work wie die fachliche Änderung (v1:
`readings_recorded` aus `RecordReadings`, `ConfirmDraft`, `CorrectReading`,
`IngestReadings`, `RecordBulkReadings`; `CommitImport` schreibt keinen
Eintrag, damit ein Import historischer Daten keine Alarme auslöst). Der Job
`DispatchOutbox` holt offene Einträge in Reihenfolge der Erzeugung, verarbeitet
sie und markiert sie als erledigt. Schlägt die Verarbeitung fehl, steigt
`attempts`, `next_attempt_at` verschiebt sich exponentiell (1 min bis 6 h);
nach 10 Versuchen wird der Eintrag `failed` und gezählt. Erledigte Einträge
entfernt das Housekeeping nach 30 Tagen.

Jobs arbeiten je Haushalt mit einem Job-Tenant: `Tenant::for_job(household)`
ist nur innerhalb von `zaehler-app` sichtbar (`pub(crate)`), hat die Rolle
`Viewer` und keinen Nutzer. Schreibende Ports nehmen ihn nicht an; Jobs
schreiben nur Outbox-Status, Versandprotokoll und `job_runs` über
`JobRepository`.

Umsetzung: Der Job-Tenant trägt statt eines fehlenden Nutzers die Nil-UUID,
damit `Tenant::user_id()` ohne `Option` auskommt; kein Anwendungsfall des
Web-Adapters erhält ihn. `readings_recorded` entsteht aus `RecordReadings`,
`ConfirmDraft`, `CorrectReading`, `IngestReadings` (je Messstelle mit den
gespeicherten oder ersetzten Ablesungen) und `RecordBulkReadings` (je
Messstelle der Sammelerfassung).
```

```{spec} Authentifizierung der REST-API
:id: SPEC_API_AUTH
:status: implemented
:satisfies: REQ_API_002, REQ_TEN_003, REQ_NFR_003
:links: ADR_023

- Token: 32 Zufallsbytes, Base64URL, mit Präfix `zt_`; angezeigt genau
  einmal bei der Erzeugung. Gespeichert werden SHA-256 des Tokens, die ersten
  8 Zeichen zur Anzeige, Name, Haushalt, Nutzer, Rolle, `expires_at`
  (optional), `revoked_at`, `last_used_at` (höchstens einmal je Minute
  aktualisiert).
- Anfrage: Header `Authorization: Bearer zt_…`; andere Schemata, Cookies
  und Sitzungen werden unter `/api/v1` nicht ausgewertet. Daher entfällt die
  CSRF-Prüfung auf diesen Routen.
- `ResolveApiTenant`: Hash nachschlagen (Vergleich über Index auf dem Hash),
  abgelaufen oder widerrufen ergibt 401; die Mitgliedschaft des Nutzers wird
  bei jeder Anfrage geprüft, die wirksame Rolle ist das Minimum aus
  Token-Rolle und aktueller Mitgliedsrolle. Ohne Mitgliedschaft 401.
- Ein Token ist an genau einen Haushalt gebunden. Tokens erzeugt und widerruft
  jeder Nutzer für sich selbst; `Owner` sehen und widerrufen alle Tokens des
  Haushalts.
- Logs enthalten die Token-ID, nie das Token.

Umsetzung: Tabelle `api_tokens` (Migration `m20261019_000015_api`),
Anwendungsfälle in `zaehler-app/src/usecase/api.rs`, Extraktor `ApiCtx` in
`zaehler-web/src/api/auth.rs`. Das Token besteht aus `zt_` und 43 Zeichen;
andere Formen werden ohne Datenbankzugriff abgelehnt. Die Rolle eines neuen
Tokens ist höchstens die eigene Rolle, das Ablaufdatum liegt in der Zukunft.
Die Token-ID steht im Feld `api_token` des Request-Spans. Seite
`/settings/tokens` zum Erzeugen, Anzeigen und Widerrufen; das Token
erscheint dort genau einmal. API-Tokens sind im Installationsarchiv
enthalten, im Haushaltsarchiv nicht.
```

```{spec} Dokument-Uploads
:id: SPEC_DOC_UPLOADS
:status: implemented
:satisfies: REQ_AI_004, REQ_AI_005, REQ_NFR_003
:links: SPEC_SEC_UPLOADS, SPEC_STORE_BLOBS

Erweiterung von {need}`SPEC_SEC_UPLOADS`: PDF (Magic Bytes `%PDF-`) bis
`upload.max_document_bytes` (Standard 10 MiB) und Fotos wie bisher. Fotos
durchlaufen {need}`SPEC_IMG_PROCESSING`; PDFs werden unverändert unter
`BlobKey::document(household, document)` gespeichert, nicht serverseitig
gerendert und ausschließlich mit `Content-Type: application/pdf`,
`Content-Disposition: attachment` und `Content-Security-Policy: sandbox`
ausgeliefert. Dokumente gehören zum Haushalt, werden mit ihm gelöscht und
sind im Export enthalten. Ein Dokument ohne Verweis (Entwurf verworfen,
Kostenbuch-Eintrag gelöscht) entfernt das Housekeeping nach
`housekeeping.draft_ttl_hours`.

Umsetzung: Tabelle `documents` (Migration `m20261017_000012_documents`).
Verweise sind `settlements.document_id`, `charging_sessions.document_id` und
`asset_costs.document_id`; übernommene Rechnungen ohne Abrechnung gelten als
ohne Verweis. Fotos werden als JPEG
normalisiert und ebenfalls nur als Download ausgeliefert. Die CSP lautet
`sandbox; default-src 'none'`, der Dateiname im Header enthält nur ASCII.
Die Größengrenze prüft der Anwendungsfall; der Body des Uploads ist auf
`upload.max_document_bytes` plus 64 KiB begrenzt.
```

```{spec} Mehrere Replicas
:id: SPEC_OPS_REPLICAS
:status: implemented
:satisfies: REQ_OPS_008
:links: SPEC_JOB_SCHEDULER, SPEC_OPS_HELM

Mit PostgreSQL und S3 laufen beliebig viele Replicas: Sitzungen in der
Datenbank, Dateien im Objektspeicher, Migration unter Advisory-Lock
({need}`SPEC_OPS_MIGRATIONS`), Jobs nur auf der Replica mit Leader-Lock.
Die Lock-Verbindung ist eine eigene Verbindung außerhalb des Pools und
verbindet direkt zur Datenbank, nicht über einen Pooler im
Transaktionsmodus (`database.lock_url`, Standard `database.url`). Ein Test in
der CI startet zwei Instanzen gegen dieselbe PostgreSQL-Datenbank und prüft,
dass eine Erinnerung genau einmal versendet wird.

Umsetzung: Geprüft sind die Bausteine getrennt auf PostgreSQL: zwei
`PgLeaderLock` gegeneinander (genau einer Leader, Übernahme nach Freigabe)
und das Versandprotokoll (`notification_log` mit eindeutigem Schlüssel aus
Nutzer, Art und Schlüssel), das einen zweiten Versand auch bei gleichzeitiger
Ausführung verhindert. Zwei laufende Server-Prozesse prüft der Smoke-Test
mit `compose.replicas.yaml` (PostgreSQL, S3, Mailpit): eine Einladung mit
E-Mail kommt genau einmal in Mailpit an, genau eine Instanz meldet
`zaehler_leader 1`. Das Chart setzt `database.lock_url` über
`database.lockUrl` aus einem Secret.
```

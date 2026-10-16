# MVP: Software-Detail-Design

Grundlage: {doc}`mvp-architektur` (hexagonal, {need}`ADR_022`). Signaturen
sind Entwurfsskizzen in Rust-Notation; sie legen Verantwortung, Ein- und
Ausgaben fest, nicht die endgültige Syntax.

## Modulstruktur

```text
crates/
  zaehler-domain/src/                       Kern: Fachlogik
    calendar.rs        Calendar (Zeitzone), Tagesbeginn in UTC        SPEC_TZ
    series.rs          Monatsserien, Register-Aggregation              SPEC_ANA_MONTHLY_SERIES, SPEC_ANA_REGISTER_AGG
    analysis.rs        Vergleichsansicht, Prognose laufender Zeitraum  SPEC_ANA_COMPARE_VIEW, SPEC_FC_CURRENT_YEAR
    (vorhanden)        consumption, comparison, expectation, forecast, period, reading, meter, tariff, car
  zaehler-app/src/                          Kern: Anwendungsfälle und Ports
    ids.rs             HouseholdId, UserId, MeterId, DeviceId, RegisterId, ReadingId, DraftId, BlobKey
    tenant.rs          Tenant, Role                                    SPEC_SEC_TENANT_SCOPE
    error.rs           AppError                                        SPEC_ERR_HANDLING
    port/              Driven Ports (Traits)
      uow.rs           UnitOfWorkFactory, UnitOfWork                   SPEC_ARCH_UOW
      repo.rs          Membership-, Household-, User-, Meter-, Reading-, Draft-, MaintenanceRepository
      blob.rs          BlobStore                                       SPEC_STORE_BLOBS
      media.rs         ImageNormalizer                                 SPEC_IMG_PROCESSING
      recognize.rs     MeterRecognizer
      system.rs        Clock, IdGenerator
    usecase/           Driving Ports (Anwendungsfälle)
      auth.rs          SignIn, ResolveTenant, SetLocale
      household.rs     CreateHousehold, GetHousehold, UpdateHousehold, DeleteHousehold
      meter.rs         CreateMeter, UpdateMeter, SwapDevice, ListMeters, GetMeter
      reading.rs       CheckReadings, RecordReadings, DeleteReading, GetReadingPhoto
      capture.rs       AnalysePhoto, ConfirmDraft, DiscardDraft, GetDraftPhoto
      analysis.rs      MeterAnalysis, MeterChartData, Dashboard
      import.rs        UploadCsv, PreviewImport, CommitImport
      housekeeping.rs  Housekeeping                                    SPEC_OPS_HOUSEKEEPING
    testing/           In-Memory-Adapter und Port-Testsuite (Feature "testing")
  zaehler-db/src/                           Driven-Adapter
    entity/, migration/m20261015_000002_mvp.rs, fixed.rs (vorhanden bzw. erweitert)
    uow.rs             SeaOrmUnitOfWork
    repo/              Implementierungen der Repository-Ports
    session_store.rs   tower_sessions::SessionStore auf Tabelle sessions
  zaehler-storage/src/ ObjectStoreBlobStore (Dateisystem, S3)
  zaehler-media/src/   ImageCrateNormalizer
  zaehler-ai/src/      OpenRouterRecognizer (vorhanden, implementiert MeterRecognizer)
  zaehler-notify/src/  (ab v1)
  zaehler-web/src/                          Driving-Adapter
    router.rs          Routen, Middleware-Reihenfolge
    extract.rs         CurrentUser, TenantCtx, Lang, CsrfToken, HxRequest
    error.rs           AppError nach HTTP                              SPEC_ERR_HANDLING
    security.rs        Sicherheitsheader, CSRF-Prüfung, Body-Limits    SPEC_SEC_HEADERS, SPEC_SEC_SESSION_CSRF, SPEC_SEC_UPLOADS
    auth/              oidc.rs, dev_login.rs
    pages/             dashboard, meters, readings, capture, import, settings, auth
    views/             View-Modelle je Seite
    i18n/              mod.rs, format.rs, parse.rs                     SPEC_I18N
    assets.rs          eingebettete statische Dateien
  zaehler-web/templates/  layout/, partials/, pages/
  zaehler-web/static/     app.css, app.js, charts.js, htmx.min.js, echarts.min.js, sw.js, icons/
  zaehler-web/locales/    de/main.ftl, en/main.ftl
  zaehler-server/src/                       Composition Root
    main.rs, config.rs  (vorhanden, erweitert)
    wiring.rs          Adapter erzeugen, Anwendungsfälle verdrahten
    scheduler.rs       Housekeeping-Intervall
    telemetry.rs       (vorhanden, erweitert)                          SPEC_OPS_OTEL
```

## Zentrale Typen (`zaehler-app`)

```rust
pub struct HouseholdId(Uuid);            // analog UserId, MeterId, DeviceId, RegisterId, ReadingId, DraftId
pub enum Role { Owner, Member, Viewer }  // MVP: jeder Haushalt hat genau einen Owner
pub struct Tenant { user_id: UserId, household_id: HouseholdId, role: Role } // Felder privat
impl Tenant {
    pub fn household_id(&self) -> HouseholdId;
    pub fn user_id(&self) -> UserId;
    pub fn require(&self, min: Role) -> Result<(), AppError>;
}
pub struct BlobKey(String);              // nur über Konstruktoren je Verwendungszweck

pub enum AppError {
    NotFound, Forbidden, Unauthenticated,
    Validation(FieldErrors),             // Feldname -> Fehlercode + Argumente (Übersetzung im Web-Adapter)
    Conflict(ConflictKind),              // z.B. doppelte Ablesung
    Upstream(UpstreamKind),              // Ai, Storage
    Internal(Box<dyn std::error::Error + Send + Sync>),
}
```

`Role`-Prüfung: schreibende Anwendungsfälle verlangen `Role::Member`,
Haushaltsverwaltung und Löschen des Haushalts `Role::Owner`. Im MVP gibt es nur
Owner; die Prüfungen sind trotzdem implementiert, damit Mitglieder in v1 ohne
Änderung der Anwendungsfälle hinzukommen.

Anwendungsfälle sind Strukturen mit den benötigten Ports als `Arc<dyn …>` und
einer Methode `execute`. `zaehler-server` erzeugt sie einmal und legt sie im
Zustand von `zaehler-web` ab (`AppServices`).

```rust
pub struct RecordReadings {
    uow: Arc<dyn UnitOfWorkFactory>,
    clock: Arc<dyn Clock>,
    ids: Arc<dyn IdGenerator>,
}
impl RecordReadings {
    pub async fn execute(&self, t: &Tenant, input: RecordReadingsInput) -> Result<Vec<ReadingId>, AppError>;
}
```

## Datenmodell: Migration `m20261015_000002_mvp`

| Tabelle | Änderung | Zweck |
|---|---|---|
| `meters` | + `billing_start_month SMALLINT NULL`, `billing_start_day SMALLINT NULL` (beide oder keiner gesetzt) | Prognose für das Abrechnungsjahr ohne Vertragsmodell ({need}`SPEC_FC_CURRENT_YEAR`) |
| `meter_devices` | + `decimal_places SMALLINT NOT NULL DEFAULT 0`; Index auf `serial_number` | Eingabeprüfung, Zuordnung über Zählernummer |
| `readings` | Unique-Index `(register_id, device_id, read_at)` | Doppelerfassung und Import-Duplikate verhindern |
| `users` | + `last_login_at TIMESTAMPTZ NULL` | Betrieb, Diagnose |
| `sessions` (neu) | `id TEXT PK`, `data BLOB`, `expires_at TIMESTAMPTZ`; Index auf `expires_at` | SessionStore ({need}`SPEC_SEC_SESSION_CSRF`) |
| `ai_drafts` (neu, eigene Migration mit WP_MVP_07) | siehe unten | Entwürfe der Foto-Erfassung ({need}`SPEC_AI_DRAFTS`) |

`ai_drafts`:

| Spalte | Typ | Bedeutung |
|---|---|---|
| `id` | UUID PK | DraftId |
| `household_id` | UUID FK, Cascade | Mandant |
| `created_by` | UUID FK `users` | Nutzer |
| `photo_key` | TEXT | Objektschlüssel unter `drafts/` |
| `model` | TEXT | verwendetes Modell |
| `duration_ms` | INTEGER | Dauer des AI-Aufrufs |
| `suggestion` | TEXT (JSON) | Vorschlag gemäß `zaehler_app::port::Recognition` |
| `matched_meter_id` | UUID NULL | zugeordnete Messstelle |
| `status` | TEXT | `pending`, `confirmed`, `discarded`, `failed` |
| `confirmed_reading_id` | UUID NULL FK `readings` | gespeicherte Ablesung |
| `corrected` | BOOLEAN NULL | bestätigter Wert weicht vom Vorschlag ab |
| `created_at`, `finished_at` | TIMESTAMPTZ | Zeitstempel |
| `taken_at` | TIMESTAMPTZ NULL | Aufnahmezeit aus den Bildmetadaten ({need}`SPEC_IMG_CAPTURE_TIME`, Migration `m20261016_000004_draft_taken_at`) |

Index `(household_id, status, created_at)`.

## Driven Ports (`zaehler-app::port`)

Alle Ports sind `#[async_trait]`-Traits mit `Send + Sync`, Rückgabe
`Result<_, PortError>` (`PortError` wird im Anwendungsfall auf `AppError`
abgebildet). Mengen sind `Decimal`; der DB-Adapter rechnet über `fixed` in
Festkomma um und nutzt ausschließlich Query-Builder
({need}`SPEC_SEC_SQL_PARAMS`).

```rust
trait UnitOfWorkFactory { async fn begin(&self) -> Result<Box<dyn UnitOfWork>, PortError>; }
trait UnitOfWork: Send {
    fn memberships(&mut self) -> &mut dyn MembershipRepository;
    fn households(&mut self) -> &mut dyn HouseholdRepository;
    fn users(&mut self) -> &mut dyn UserRepository;
    fn meters(&mut self) -> &mut dyn MeterRepository;
    fn readings(&mut self) -> &mut dyn ReadingRepository;
    fn drafts(&mut self) -> &mut dyn DraftRepository;
    async fn commit(self: Box<Self>) -> Result<(), PortError>;
}
trait MembershipRepository {
    async fn role(&mut self, user: UserId, household: HouseholdId) -> Result<Option<Role>, PortError>;
    async fn households_of(&mut self, user: UserId) -> Result<Vec<HouseholdSummary>, PortError>;
}
trait HouseholdRepository {
    async fn create(&mut self, owner: UserId, h: NewHousehold) -> Result<HouseholdId, PortError>;
    async fn get(&mut self, t: &Tenant) -> Result<Household, PortError>;
    async fn update(&mut self, t: &Tenant, h: HouseholdUpdate) -> Result<(), PortError>;
    async fn delete(&mut self, t: &Tenant) -> Result<(), PortError>;          // Cascade über FKs
}
trait UserRepository {
    async fn upsert_oidc(&mut self, identity: &Identity) -> Result<User, PortError>;
    async fn set_locale(&mut self, user: UserId, locale: Locale) -> Result<(), PortError>;
}
trait MeterRepository {
    async fn list(&mut self, t: &Tenant) -> Result<Vec<MeterSummary>, PortError>;   // inkl. letzter Ablesung je Register
    async fn get(&mut self, t: &Tenant, id: MeterId) -> Result<Option<MeterAggregate>, PortError>;
    async fn create(&mut self, t: &Tenant, m: NewMeter) -> Result<MeterId, PortError>;
    async fn update(&mut self, t: &Tenant, id: MeterId, m: MeterUpdate) -> Result<(), PortError>;
    async fn add_device(&mut self, t: &Tenant, id: MeterId, d: NewDevice) -> Result<DeviceId, PortError>;
    async fn retire_device(&mut self, t: &Tenant, id: DeviceId, at: DateTime<Utc>) -> Result<(), PortError>;
    async fn find_by_serial(&mut self, t: &Tenant, serial: &str) -> Result<Option<(MeterId, DeviceId)>, PortError>;
}
trait ReadingRepository {
    async fn list_for_meter(&mut self, t: &Tenant, id: MeterId) -> Result<Vec<ReadingRow>, PortError>;
    async fn last_per_register(&mut self, t: &Tenant, id: MeterId) -> Result<Vec<ReadingRow>, PortError>;
    async fn insert(&mut self, t: &Tenant, r: &[NewReading]) -> Result<Vec<ReadingId>, PortError>; // Conflict bei Duplikat
    async fn delete(&mut self, t: &Tenant, id: ReadingId) -> Result<Option<DeletedReading>, PortError>;
    async fn photo_key(&mut self, t: &Tenant, id: ReadingId) -> Result<Option<BlobKey>, PortError>;
}
trait DraftRepository {
    async fn create(&mut self, t: &Tenant, d: NewDraft) -> Result<DraftId, PortError>;
    async fn get_pending(&mut self, t: &Tenant, id: DraftId) -> Result<Option<Draft>, PortError>;
    async fn finish(&mut self, t: &Tenant, id: DraftId, f: DraftOutcome) -> Result<(), PortError>;
}
trait MaintenanceRepository {                                   // ohne Tenant, nur für Housekeeping
    async fn expire_drafts(&self, cutoff: DateTime<Utc>, now: DateTime<Utc>) -> Result<Vec<BlobKey>, PortError>;
    async fn delete_expired_sessions(&self, now: DateTime<Utc>) -> Result<u64, PortError>;
}
trait BlobStore {
    async fn put(&self, key: &BlobKey, bytes: Bytes) -> Result<(), PortError>;
    async fn get(&self, key: &BlobKey) -> Result<Bytes, PortError>;
    async fn copy(&self, from: &BlobKey, to: &BlobKey) -> Result<(), PortError>;
    async fn delete(&self, key: &BlobKey) -> Result<(), PortError>;
    async fn delete_household(&self, household: HouseholdId) -> Result<(), PortError>;
    async fn list_tmp_older_than(&self, cutoff: DateTime<Utc>) -> Result<Vec<BlobKey>, PortError>;
}
trait ImageNormalizer { async fn normalize(&self, bytes: Bytes) -> Result<NormalizedJpeg, PortError>; }
trait MeterRecognizer {
    fn model(&self) -> &str;
    async fn recognize(&self, jpeg: &NormalizedJpeg) -> Result<Recognition, PortError>;
}
trait Clock { fn now(&self) -> DateTime<Utc>; }
trait IdGenerator { fn new_uuid(&self) -> Uuid; }               // UUIDv7
```

`MaintenanceRepository` ist der einzige Port ohne `Tenant`; ihn verwendet nur
der Anwendungsfall `Housekeeping`. Der Zählerwechsel ist kein eigener
Port-Aufruf, sondern ein Anwendungsfall aus `retire_device`, `add_device` und
`insert` innerhalb einer Unit of Work. `MeterRecognizer` ist optional: ohne
AI-Konfiguration verdrahtet `zaehler-server` keinen Recognizer, und
`AnalysePhoto` legt den Entwurf mit Status `failed` zur manuellen Eingabe an.

## Anwendungsfälle (`zaehler-app::usecase`)

| Anwendungsfall | Ablauf (Kurzform) | Spezifikation |
|---|---|---|
| `SignIn` | Nutzer per Identität laden oder anlegen; ohne Mitgliedschaft Haushalt „{Anzeigename}“ mit Zeitzone `Europe/Berlin` anlegen; liefert Nutzer und aktiven Haushalt | {need}`SPEC_TEN_HOUSEHOLDS` |
| `ResolveTenant` | Nutzer laden, Mitgliedschaften prüfen, aktiven Haushalt wählen (ohne Mitgliedschaft anlegen), `Tenant` erzeugen | {need}`SPEC_SEC_TENANT_SCOPE` |
| `SetLocale` | Sprache des Nutzers speichern | {need}`SPEC_I18N` |
| `CreateHousehold`, `GetHousehold`, `UpdateHousehold` | Owner-Prüfung, Daten speichern; der Wechsel des aktiven Haushalts ist ein Sitzungswert im Web-Adapter, geprüft von `ResolveTenant` | {need}`SPEC_TEN_HOUSEHOLDS` |
| `DeleteHousehold` | Bestätigung = Haushaltsname; `BlobStore::delete_household`, dann DB-Cascade | {need}`SPEC_PRIV_HOUSEHOLD_DELETE` |
| `CreateMeter`, `UpdateMeter`, `ListMeters`, `GetMeter` | Validierung, Standardeinheit aus Art, Register anlegen | {need}`SPEC_MTR_MANAGE` |
| `SwapDevice` | Endstände alt und Startstände neu für alle Register zum selben Zeitpunkt in einer Unit of Work | {need}`SPEC_MTR_SWAP_FLOW` |
| `CheckReadings` | Plausibilität je Register, keine Speicherung (HTMX) | {need}`SPEC_RDG_FORM` |
| `RecordReadings` | Plausibilität; Warnungen ohne Bestätigung führen zu `Validation`; sonst speichern | {need}`SPEC_RDG_FORM` |
| `DeleteReading` | Ablesung löschen, nach `commit` Foto löschen | {need}`SPEC_RDG_DELETE` |
| `GetReadingPhoto`, `GetDraftPhoto` | Tenant-Prüfung, Blob laden | {need}`SPEC_SEC_UPLOADS` |
| `AnalysePhoto` | `ImageNormalizer`, `BlobStore::put`, `MeterRecognizer`, Zuordnung, Entwurf anlegen | {need}`SPEC_AI_DRAFTS`, {need}`SPEC_IMG_PROCESSING` |
| `ConfirmDraft` | Blob umkopieren, Ablesung mit `source=photo` speichern, Entwurf abschließen, `commit`, Entwurfsblob löschen | {need}`SPEC_AI_DRAFTS` |
| `DiscardDraft` | Entwurf verwerfen, Blob löschen | {need}`SPEC_AI_DRAFTS` |
| `MeterAnalysis` | Ablesungen laden, Domain-Funktionen, Ergebnisobjekt | {need}`SPEC_ANA_COMPARE_VIEW`, {need}`SPEC_FC_CURRENT_YEAR` |
| `MeterChartData` | Monatsserien laufendes Jahr und Vorjahr, kumuliert mit Prognose | {need}`SPEC_ANA_CHARTS` |
| `Dashboard` | je Messstelle letzter Stand, Prognose, Vergleich Vormonat, Fälligkeit | {need}`SPEC_UI_DASHBOARD` |
| `UploadCsv`, `PreviewImport`, `CommitImport` | siehe {need}`SPEC_IMP_CSV_FORMAT` | {need}`SPEC_IMP_CSV` |
| `Housekeeping` | siehe {need}`SPEC_OPS_HOUSEKEEPING` | {need}`SPEC_OPS_HOUSEKEEPING` |

### Haushalte

```{spec} Haushalte anlegen, wechseln, umbenennen
:id: SPEC_TEN_HOUSEHOLDS
:status: implemented
:satisfies: REQ_TEN_001

Beim ersten Login ohne Mitgliedschaft entsteht ein Haushalt mit dem Namen
des Nutzers (Anzeigename aus dem ID-Token) und der Rolle `owner`. Weitere
Haushalte legt ein Nutzer unter „Einstellungen“ an. Der aktive Haushalt steht
in der Sitzung (`hid`); bei jedem Request ruft der Extraktor `Ctx` in
`zaehler-web` den Anwendungsfall `ResolveTenant` auf. Ist der gewählte
Haushalt nicht zugänglich (z.B. gelöscht), wählt `ResolveTenant` den ersten
verbleibenden Haushalt; ohne Mitgliedschaft legt er einen neuen an. Der Haushaltswechsel ist ein `POST`
mit CSRF-Token.
```

```{spec} Entwicklungs-Login
:id: SPEC_TEN_DEV_LOGIN
:status: implemented
:satisfies: REQ_TEN_002

Nur mit `auth.dev_login = true`: `GET /auth/dev-login?name=…` meldet einen
Nutzer mit Issuer `dev` und Subject = Name an. Beim Start wird eine Warnung
geloggt; ist zusätzlich OIDC konfiguriert oder `server.public_url` kein
`localhost`, verweigert der Server den Start. Integrationstests verwenden
diesen Weg.
```

```{spec} Haushalt vollständig löschen
:id: SPEC_PRIV_HOUSEHOLD_DELETE
:status: draft
:satisfies: REQ_NFR_004

Owner löschen einen Haushalt nach Eingabe des Haushaltsnamens als
Bestätigung. Reihenfolge: alle Objekte unter `households/{id}/` löschen,
danach die Zeile in `households` (Cascade auf Mitgliedschaften, Messstellen,
Geräte, Register, Ablesungen, Entwürfe). Schlägt das Löschen der Objekte fehl,
bleibt die DB unverändert und der Vorgang meldet einen Fehler; ein erneuter
Aufruf ist möglich. Nutzerkonten bleiben bestehen.
```

### Messstellen

```{spec} Messstellen anlegen und bearbeiten
:id: SPEC_MTR_MANAGE
:status: implemented
:satisfies: REQ_MTR_001, REQ_MTR_003, REQ_MTR_006

Formularfelder: Name (1-80 Zeichen, je Haushalt eindeutig), Art, Einheit
(Vorschlag aus `MeterKind::default_unit`, bei `custom` Pflicht), Zählernummer
(optional, je Haushalt eindeutig über alle aktiven Geräte), Vorkommastellen
(1-9, optional; gesetzt aktiviert Überlauferkennung), Nachkommastellen (0-3),
Ableseturnus in Tagen (optional), maximale Tagesrate (optional), Beginn des
Abrechnungsjahres (Monat/Tag, optional), Register (mindestens eines; Code
1-16 Zeichen, eindeutig je Messstelle; Richtung Bezug/Einspeisung),
Startstand je Register mit Zeitpunkt. Register sind nach der ersten Ablesung
nicht mehr löschbar, nur umbenennbar. Art, Einheit und Register-Richtung sind
nach der ersten Ablesung nicht mehr änderbar.
```

```{spec} Ablauf Zählerwechsel
:id: SPEC_MTR_SWAP_FLOW
:status: implemented
:satisfies: REQ_MTR_005

Eingaben: Zeitpunkt, Endstand je Register (altes Gerät), neue Zählernummer,
Vor- und Nachkommastellen, Startstand je Register (neues Gerät). In einer
Transaktion: Endstände als Ablesungen des alten Geräts speichern,
`removed_at` setzen, neues Gerät mit `installed_at` = Zeitpunkt anlegen,
Startstände als Ablesungen des neuen Geräts zum selben Zeitpunkt speichern.
Der Endstand wird gegen die letzte Ablesung des alten Geräts plausibilisiert.
Ein Zeitpunkt vor der letzten Ablesung ist unzulässig.
```

### Erfassung

```{spec} Erfassungsformular
:id: SPEC_RDG_FORM
:status: implemented
:satisfies: REQ_RDG_001, REQ_RDG_002

`GET /meters/{id}/readings/new` zeigt je Register den letzten Stand mit Datum,
ein Eingabefeld (`inputmode="decimal"`) und den Zeitpunkt (Vorgabe: jetzt in
der Zeitzone des Haushalts). Bei Änderung eines Werts prüft
`POST /meters/{id}/readings/check` (HTMX, `hx-trigger="change delay:300ms"`)
die Plausibilität und ersetzt den Hinweisbereich. Beim Speichern werden alle
Register in einer Transaktion gespeichert; leere Felder werden ignoriert,
mindestens ein Wert ist Pflicht. Liegen Warnungen vor, erscheint das Formular
erneut mit Hinweisen und der Checkbox „Trotzdem speichern“
(`confirm_warnings`). Mehr Nachkommastellen als am Gerät konfiguriert ergeben
einen Validierungsfehler. Ab Dashboard: Karte → „Stand erfassen“ →
Werte eingeben → Speichern (drei Interaktionen).
```

```{spec} Ablesung löschen
:id: SPEC_RDG_DELETE
:status: draft
:links: REQ_RDG_003

Löschen einer einzelnen Ablesung (Rolle `member`) nach Bestätigung, inklusive
Belegfoto. Ohne Änderungshistorie; die Historie folgt mit
{need}`REQ_RDG_003` (v1). Ablesungen, die Teil eines Zählerwechsels sind,
lassen sich nur gemeinsam mit dem Wechsel löschen; im MVP ist das Löschen
solcher Ablesungen gesperrt.
```

### Fotos und AI

```{spec} Bildverarbeitung
:id: SPEC_IMG_PROCESSING
:status: implemented
:satisfies: REQ_AI_001, REQ_RDG_004, REQ_NFR_004
:links: ADR_017

Port `ImageNormalizer`, Adapter `zaehler-media` (in `spawn_blocking`): Format über Magic Bytes
(JPEG, PNG, WebP), dekodieren mit `image` 0.25, EXIF-Ausrichtung anwenden,
auf höchstens `image.max_pixels` (Standard 2 000 000) verkleinern
(Lanczos3, Seitenverhältnis erhalten), als JPEG mit `image.jpeg_quality`
(Standard 85) kodieren. Das Ergebnis enthält keine Metadaten (EXIF, GPS, XMP).
Nicht dekodierbare Dateien und HEIC ergeben einen Validierungsfehler mit
Hinweis auf JPEG (`photo.unsupported`); Bilder mit mehr als 16 384 Pixeln
Kantenlänge oder mehr als 512 MiB Dekodierspeicher ergeben `photo.too_large`.
Gespeichert und an die AI gesendet wird nur das Ergebnis; aus den Metadaten
wird vorher nur die Aufnahmezeit übernommen ({need}`SPEC_IMG_CAPTURE_TIME`).
```

```{spec} Aufnahmezeit als Ablesezeitpunkt
:id: SPEC_IMG_CAPTURE_TIME
:status: implemented
:satisfies: REQ_RDG_006
:links: SPEC_IMG_PROCESSING, SPEC_TZ

`zaehler-media` liest vor dem Dekodieren die EXIF-Daten des Originals
(`kamadak-exif`): `DateTimeOriginal` mit `OffsetTimeOriginal`, sonst
`DateTime` mit `OffsetTime`; ungültige Werte (z.B. `0000:00:00 00:00:00`)
ergeben keine Zeit. Der Port liefert sie als `PhotoTime` (lokale Zeit,
optionaler Offset) im `NormalizedJpeg`. `AnalysePhoto` rechnet mit Offset
exakt, sonst in der Zeitzone des Haushalts nach UTC um, verwirft Zeitpunkte
mehr als 5 Minuten in der Zukunft und speichert das Ergebnis im Entwurf
(`ai_drafts.taken_at`, Migration `m20261016_000004_draft_taken_at`). Das
Bestätigungsformular und das Formular „Neue Messstelle aus diesem Foto“
({need}`SPEC_AI_NEW_METER`) verwenden diesen Zeitpunkt als Vorgabe mit dem
Hinweis „Aufnahmezeit aus den Fotodaten“; ohne Aufnahmezeit gilt jetzt.
```

```{spec} AI-Entwürfe und Zuordnung
:id: SPEC_AI_DRAFTS
:status: implemented
:satisfies: REQ_AI_002, REQ_AI_003
:links: SPEC_AI_FLOW

`analyse` legt einen Entwurf mit Status `pending` an, auch wenn der AI-Aufruf
fehlschlägt (dann `failed`, Formular zur manuellen Eingabe mit Foto).
Zuordnung in dieser Reihenfolge: (1) Zählernummer exakt nach Normalisierung
(Großbuchstaben, ohne Leerzeichen und Bindestriche) gegen aktive Geräte;
(2) eindeutige Messstelle passender Art, deren letzter Stand nicht größer
als der erkannte Wert ist und deren Tagesrate plausibel bleibt; (3) sonst
Auswahlliste aller Messstellen. Register: erkannter Code gegen Register der
Messstelle, sonst Register `main` bzw. Auswahl. Das Bestätigungsformular zeigt
Foto, Vorschlag, Konfidenz und Plausibilität; Werte bleiben editierbar.
`confirm` speichert die Ablesung (`source=photo`, `ai_confidence`,
`photo_key`), setzt `corrected` und den Status `confirmed`. Entwürfe sind
nur für den anlegenden Haushalt sichtbar. Metriken:
`zaehler_ai_requests_total{outcome}`, `zaehler_ai_request_duration_seconds`,
`zaehler_ai_corrections_total{corrected}`.
```

```{spec} Messstelle aus einem Foto anlegen
:id: SPEC_AI_NEW_METER
:status: implemented
:satisfies: REQ_AI_007, REQ_MTR_001
:links: SPEC_AI_DRAFTS, SPEC_MTR_MANAGE

Einstieg über die Foto-Erfassung: Die Bestätigungsseite eines offenen
Entwurfs bietet Ownern „Neue Messstelle aus diesem Foto“ an
(`GET /meters/new?draft={id}`); Messstellenliste und leeres Formular
verweisen auf `/capture`. Vorbelegung aus dem Vorschlag: Art (sonst Strom),
Name = Bezeichnung der Art, Einheit nur bei Abweichung von der
Standardeinheit, Zählernummer, Registercode (sonst `main`), Startstand,
Nachkommastellen aus dem erkannten Wert (höchstens 3), Zeitpunkt = jetzt. Das
Formular zeigt das Foto und überträgt die Entwurfs-ID im Feld `draft`.
`POST /meters` mit `draft` ruft `CreateMeter::execute_from_draft` auf:
Validierung wie {need}`SPEC_MTR_MANAGE`, zusätzlich ist mindestens ein
Startstand Pflicht (`meter.photo_start_required`). In einer Transaktion
entstehen Messstelle, Gerät, Register und Startstände; der erste Startstand
erhält `source=photo`, `ai_confidence` und das kopierte Foto, der Entwurf wird
mit `corrected` abgeschlossen. Nach dem `commit` wird das Entwurfsfoto
gelöscht.
```

### Auswertung und Prognose

```{spec} Monatsserien
:id: SPEC_ANA_MONTHLY_SERIES
:status: implemented
:satisfies: REQ_ANA_001
:links: SPEC_CONS_INTERPOLATION, SPEC_TZ

`series::monthly(registers, months, calendar) -> Vec<MonthValue>` mit
`MonthValue { month: Period, value: Option<Decimal> }`; `registers` sind die
Intervalle der beteiligten Register. Für jeden Monat werden Beginn und Ende
über `Calendar` in UTC umgerechnet und `series::total_in` (Summe von
`consumption_in` je Register) aufgerufen; `None`, wenn der Monat nicht
vollständig durch Ablesungen abgedeckt ist. Die Monate reichen vom Monat der
ersten Ablesung bis zum laufenden Monat. Werte werden erst bei der Anzeige gerundet. Die
Summe der Monatswerte eines vollständig abgedeckten Zeitraums entspricht dem
Verbrauch des Zeitraums (Property-Test).
```

```{spec} Aggregation über Register
:id: SPEC_ANA_REGISTER_AGG
:status: implemented
:satisfies: REQ_MTR_003, REQ_ANA_001

Intervalle werden je Register gebildet ({need}`SPEC_CONS_DEVICE_SWAP`). Für
die Messstelle gilt: Bezug = Summe aller Register mit Richtung `import`,
Einspeisung = Summe aller Register mit Richtung `export`; ein Monatswert ist
`None`, sobald ein beteiligtes Register `None` liefert. Ablesungen zum selben
Zeitpunkt werden nach der Einbaureihenfolge der Geräte sortiert (Endstand alt
vor Startstand neu). Ein Register mit inkonsistenten Ablesungen (sinkender
Stand ohne Überlauf) wird mit Hinweis angezeigt und liefert keine Werte.
Hauptserie für Vergleiche, Prognose und Diagramme ist der Bezug, ohne
Bezugsregister die Einspeisung. Die Auswertung zeigt Bezug, Einspeisung (falls
vorhanden) und bei mehreren Registern je Register eine Spalte der
Monatstabelle.
```

```{spec} Vergleichsansicht
:id: SPEC_ANA_COMPARE_VIEW
:status: implemented
:satisfies: REQ_ANA_003, REQ_ANA_004, REQ_ANA_005
:links: SPEC_CMP_PERIODS, SPEC_CMP_DIFF

Bezugsmonat ist der letzte vollständig abgedeckte Monat (wählbar über eine
Monatsauswahl, `GET /meters/{id}?month=JJJJ-MM`; nicht abgedeckte Monate
fallen auf den letzten vollständigen zurück). Für ihn werden Vormonat, Vorjahresmonat und die
Durchschnitte der letzten 3, 6 und 12 Monate mit absoluter und relativer
Differenz ausgegeben. Ein Vergleich entfällt mit dem Hinweis
„Daten unvollständig“, wenn einer der Referenzmonate `None` ist. Zusätzlich:
laufendes Jahr bis zum letzten Ablesezeitpunkt gegen denselben Zeitraum des
Vorjahres (Tag und Monat gleich, Schaltjahr: 28.02.).
```

```{spec} Prognose laufender Zeiträume
:id: SPEC_FC_CURRENT_YEAR
:status: implemented
:satisfies: REQ_FC_001
:links: SPEC_FC_PERIOD_FORECAST

Prognose für das laufende Kalenderjahr und, falls an der Messstelle ein
Beginn des Abrechnungsjahres hinterlegt ist, für das laufende Abrechnungsjahr.
Gemessen = Verbrauch vom Zeitraumbeginn bis zum Beginn des Tages der letzten
gemeinsamen Ablesung aller Register (`analysis::forecast_period`), damit
Messung und Gewichtung dieselben ganzen Tage umfassen; Prognose über
`forecast::project` mit `SeasonProfile::uniform()` (das gelernte Profil folgt
mit {need}`REQ_ANA_002`, v1). Ausgabe: gemessen, Prognose, Abdeckungsgrad.
Ohne Ablesung nach dem Zeitraumbeginn bzw. ohne Ablesung vor dem
Zeitraumbeginn (Startwert fehlt) entfällt die Prognose mit Hinweis.
```

```{spec} Diagramme
:id: SPEC_ANA_CHARTS
:status: implemented
:satisfies: REQ_ANA_007
:links: ADR_002

Apache ECharts 5.6.0 (Apache-2.0, eingebettet als `static/echarts.min.js`).
`static/charts.js` wird im Layout geladen; es lädt ECharts nur nach, wenn die
Seite einen Container mit `data-chart-url` enthält (im MVP nur die
Detailseite, das Dashboard enthält keine Diagramme), auch nach
HTMX-Seitenwechseln und Wiederherstellung aus dem Verlauf. Es lädt JSON von
`GET /meters/{id}/chart.json` (Monatsnamen in der Sprache des Nutzers, Werte
auf 3 Stellen gerundet) und rendert: (1) Monatsbalken laufendes Jahr
und Vorjahr nebeneinander, unvollständige Monate leer; (2) kumulierter
Jahresverlauf als Linie, Prognose gestrichelt bis Jahresende
(`analysis::projected_cumulative`, beginnend am letzten Monatsende mit
Messwert). Kein Inline-Skript; Farben über CSS-Variablen ausgelesen; bei
`prefers-reduced-motion` ohne Animation. Unter den Diagrammen steht dieselbe
Information als Tabelle (`<details>`); schlägt das Laden fehl, bleiben nur
die Tabellen.
```

### Dashboard, PWA, Import

```{spec} Dashboard
:id: SPEC_UI_DASHBOARD
:status: implemented
:satisfies: REQ_ANA_008, REQ_RDG_001

Je aktive Messstelle eine Karte: Name, Art, letzter Stand mit Datum,
Verbrauch des letzten vollständigen Monats mit Differenz zum Vormonat,
Jahresprognose mit Abdeckungsgrad, Fälligkeit (letzte Ablesung +
Ableseturnus; überfällig hervorgehoben), Schaltflächen „Stand erfassen“ und
„Foto“. Sortierung: überfällige zuerst, dann nach Name. Leerer Zustand:
Verweis auf „Messstelle anlegen“ und „CSV-Import“. Die Ampel
({need}`REQ_ANA_006`) folgt in v1.
```

```{spec} Progressive Web App
:id: SPEC_UI_PWA
:status: draft
:satisfies: REQ_NFR_001

`GET /manifest.webmanifest` (Name, Kurzname, `display: standalone`,
`start_url: /`, Theme-Farben, Icons 192/512 px und maskable). `GET /sw.js`
registriert einen Service Worker ohne Offline-Cache (Netzwerk-Durchreichung);
Offline-Betrieb ist nicht Teil des MVP. Layout ab 360 px Breite ohne
horizontales Scrollen, Bedienelemente mindestens 44 px hoch. Foto-Erfassung
über `<input type="file" accept="image/jpeg,image/png,image/webp"
capture="environment">`.
```

```{spec} CSV-Format und Import-Ablauf
:id: SPEC_IMP_CSV_FORMAT
:status: implemented
:satisfies: REQ_IMP_001
:links: SPEC_IMP_CSV

Schritt 1 `POST /import/upload` (Multipart, Feld `_csrf`, Limit
`upload.max_csv_bytes`): Datei (UTF-8, optional BOM, ohne NUL-Zeichen;
Trennzeichen `;`, `,` oder Tab, das häufigste der ersten Zeile, bei Gleichstand
in dieser Reihenfolge; höchstens 20 000 Datenzeilen) wird unter `tmp/`
abgelegt; der Schlüssel steht in der Sitzung. Schritt 2 `GET /import/preview`:
Zuordnung der Spalten (Datum, Uhrzeit optional, Wert, Register optional,
Zählernummer optional, Notiz optional), Ziel-Messstelle (`?meter=`),
Datumsformat (`TT.MM.JJJJ`, `JJJJ-MM-TT` jeweils optional mit Uhrzeit in
derselben Spalte, ISO 8601 mit Zeit und optionalem Offset), Dezimaltrenner,
Register für Zeilen ohne Angabe. Vorgeschlagen werden Überschrift (erste Zeile
ohne Datum), Spalten über Schlüsselwörter (de/en) bzw. die erste Datenzeile,
Datumsformat und Dezimaltrenner. Ohne Uhrzeit gilt 00:00 Uhr in der Zeitzone
des Haushalts. `POST /import/preview` zeigt je Zeile Ergebnis (`ok`,
`warnung`, `duplikat`, `fehler` mit Grund) und die Summen. Gerät: über die
Zählernummer, sonst das zum Zeitpunkt eingebaute Gerät; Zeilen vor dem Einbau
des ersten Geräts gehören zum ersten Gerät. Plausibilität je Register und
Gerät gegen Vorgänger und Nachfolger aus vorhandenen und importierten
Ablesungen. Duplikat = gleiches Register, Gerät, Zeitpunkt und Wert;
abweichender Wert zum selben Zeitpunkt ist ein Fehler. Schritt 3
`POST /import/commit`: Die Zeilen werden in der Transaktion erneut geprüft;
Zeilen `ok` und mit `confirm_warnings` die Zeilen `warnung` werden mit
`insert` gespeichert (`source=csv_import`), liegen sie vor dem Einbau des
ersten Geräts, wird dessen Einbau vorverlegt. Danach wird die Datei gelöscht.
Abweichend vom Port-Entwurf entfällt `insert_skip_duplicates`, da Duplikate
bereits in der Prüfung innerhalb der Transaktion erkannt werden. Metriken
`zaehler_import_rows_total{result}`, `zaehler_readings_created_total{source}`.
```

## HTTP-Schnittstelle

Alle Routen außer `/auth/*`, `/healthz`, `/readyz`, `/static/*`,
`/manifest.webmanifest` und `/sw.js` erfordern eine Sitzung. Rolle: `M` =
Member, `O` = Owner. `F` = HTMX-Fragment.

| Methode | Pfad | Rolle | Antwort | Spezifikation |
|---|---|---|---|---|
| GET | `/` | Viewer | Dashboard | {need}`SPEC_UI_DASHBOARD` |
| GET | `/auth/login`, `/auth/callback` | | Redirect | {need}`SPEC_TEN_OIDC` |
| POST | `/auth/logout` | | Redirect | {need}`SPEC_TEN_OIDC` |
| GET | `/auth/dev-login` | | Redirect | {need}`SPEC_TEN_DEV_LOGIN` |
| GET | `/meters` | Viewer | Liste | {need}`SPEC_MTR_MANAGE` |
| GET, POST | `/meters/new`, `/meters` | O | Formular, 303; `?draft=` bzw. Feld `draft` übernimmt einen Foto-Entwurf | {need}`SPEC_MTR_MANAGE`, {need}`SPEC_AI_NEW_METER` |
| GET | `/meters/{id}` | Viewer | Detail mit Ablesungen und Auswertung | {need}`SPEC_ANA_COMPARE_VIEW` |
| GET | `/meters/{id}/chart.json` | Viewer | JSON | {need}`SPEC_ANA_CHARTS` |
| GET, POST | `/meters/{id}/edit` | O | Formular, 303 | {need}`SPEC_MTR_MANAGE` |
| GET, POST | `/meters/{id}/swap` | O | Formular, 303 | {need}`SPEC_MTR_SWAP_FLOW` |
| GET, POST | `/meters/{id}/readings/new`, `/meters/{id}/readings` | M | Formular, 303 | {need}`SPEC_RDG_FORM` |
| POST | `/meters/{id}/readings/check` | M | F | {need}`SPEC_RDG_FORM` |
| POST | `/readings/{id}/delete` | M | 303 | {need}`SPEC_RDG_DELETE` |
| GET | `/readings/{id}/photo` | Viewer | `image/jpeg` | {need}`SPEC_SEC_UPLOADS` |
| GET | `/capture` | M | Upload-Seite | {need}`SPEC_AI_FLOW` |
| POST | `/capture` | M | 303 zur Bestätigung (Multipart, Feld `_csrf`) | {need}`SPEC_AI_DRAFTS` |
| GET | `/capture/{draft}` | M | Bestätigungsformular, `?meter=` wählt die Messstelle | {need}`SPEC_AI_DRAFTS` |
| GET | `/capture/{draft}/photo` | M | `image/jpeg` | {need}`SPEC_AI_DRAFTS` |
| POST | `/capture/{draft}/confirm`, `/capture/{draft}/discard` | M | 303 | {need}`SPEC_AI_DRAFTS` |
| GET, POST | `/import`, `/import/upload`, `/import/preview`, `/import/commit` | M | Seiten | {need}`SPEC_IMP_CSV_FORMAT` |
| GET, POST | `/settings` | Viewer | Sprache des Nutzers, Haushaltsdaten (O) | {need}`SPEC_I18N` |
| POST | `/households`, `/households/switch` | | 303 | {need}`SPEC_TEN_HOUSEHOLDS` |
| POST | `/households/delete` | O | 303 | {need}`SPEC_PRIV_HOUSEHOLD_DELETE` |
| GET | `/manifest.webmanifest`, `/sw.js` | | statisch | {need}`SPEC_UI_PWA` |
| GET | `/static/{version}/{name}` | | statisch, `Cache-Control: immutable` | {need}`SPEC_SEC_HEADERS` |

Middleware-Reihenfolge (außen nach innen): Request-ID, Trace, Metriken,
Kompression, Timeout (120 s), Sicherheitsheader, Sitzung, CSRF-Prüfung,
Body-Limit je Route, Handler. Statische Dateien enthalten die
Programmversion im Pfad; in Debug-Builds gilt `Cache-Control: no-cache`.

## Oberfläche

| Template | Inhalt |
|---|---|
| `layout/base.html` | Kopf mit Navigation, Haushaltswechsel, CSRF-Meta-Tag, `hx-headers` am `body`, Flash-Meldungen |
| `partials/form_errors.html`, `partials/plausibility.html`, `partials/reading_row.html` | wiederverwendete Fragmente |
| `pages/dashboard.html` | Karten je Messstelle |
| `pages/meters/{list,form,detail,swap}.html` | Messstellen |
| `pages/readings/form.html` | Erfassung |
| `pages/capture/{upload,confirm}.html` | Foto-Erfassung |
| `pages/import/{upload,mapping,preview,result}.html` | CSV-Import |
| `pages/settings.html`, `pages/error.html` | Einstellungen, Fehler |

JavaScript ohne Build-Schritt, je Datei ein Modul: `app.js` (HTMX-Konfiguration,
Lade-Indikator, Bestätigungsdialoge über `hx-confirm`), `charts.js`
({need}`SPEC_ANA_CHARTS`), `sw.js` ({need}`SPEC_UI_PWA`). Formulare
funktionieren ohne JavaScript (vollständige Seitenantworten); HTMX ergänzt
Fragmente und Indikatoren.

```{spec} Ladezustand von Formularen
:id: SPEC_UI_BUSY
:status: implemented
:satisfies: REQ_NFR_009

`app.js` wird in beiden Layouts (`layout/base.html`, `layout/plain.html`)
geladen und behandelt alle Formulare ohne Markup-Änderung:

- Start: per HTMX (`htmx:beforeRequest`, also nach `hx-confirm` und nach dem
  Einsammeln der Formularwerte) oder bei normalem Absenden nach dem
  `submit`-Event. Das Formular erhält `aria-busy="true"`, der auslösende
  Button (`SubmitEvent.submitter`, sonst der erste Submit-Button)
  `aria-busy="true"` und `disabled`.
- Ein weiteres `submit` auf ein Formular mit `aria-busy` wird verworfen
  (keine doppelten Requests, auch nicht per Enter-Taste).
- Ende: `htmx:afterRequest` (auch bei Fehler, Timeout, Abbruch). Vor
  `htmx:beforeHistorySave` und beim Wiederherstellen aus dem
  Back-Forward-Cache (`pageshow`) wird der Zustand zurückgesetzt.
- Darstellung in `app.css` über `button[aria-busy="true"]::before` (Spinner);
  bei `prefers-reduced-motion` langsamer.

Keine Inline-Skripte, die Content-Security-Policy bleibt unverändert
({need}`SPEC_SEC_HEADERS`).
```

## Konfiguration: Erweiterungen

| Schlüssel | Default | Zweck |
|---|---|---|
| `auth.dev_login` | `false` | {need}`SPEC_TEN_DEV_LOGIN` |
| `session.ttl_hours` | `720` | Sitzungsdauer, gleitend |
| `session.cookie_secure` | `true` | `false` nur bei `public_url` mit `http://localhost` zulässig |
| `upload.max_photo_bytes` | `15728640` | {need}`SPEC_SEC_UPLOADS` |
| `upload.max_csv_bytes` | `5242880` | {need}`SPEC_SEC_UPLOADS` |
| `image.max_pixels` | `2000000` | {need}`SPEC_IMG_PROCESSING` |
| `image.jpeg_quality` | `85` | {need}`SPEC_IMG_PROCESSING` |
| `housekeeping.interval_minutes` | `60` | {need}`SPEC_OPS_HOUSEKEEPING` |
| `housekeeping.draft_ttl_hours` | `24` | {need}`SPEC_OPS_HOUSEKEEPING` |
| `otel.endpoint` | leer | {need}`SPEC_OPS_OTEL`; alternativ `OTEL_EXPORTER_OTLP_ENDPOINT` |
| `otel.service_name` | `zaehler` | {need}`SPEC_OPS_OTEL` |

Neue Abhängigkeiten je Crate:

| Crate | Abhängigkeiten |
|---|---|
| `zaehler-domain` | `chrono-tz` 0.10 |
| `zaehler-app` | `async-trait`, `csv` 1, `bytes`; Tests `proptest` |
| `zaehler-web` | `openidconnect` 4, `tower-sessions` 0.15, `fluent-templates` 0.15 (dazu vorhandene `axum`, `askama`, `tower-http`) |
| `zaehler-db` | `tower-sessions` 0.15 (nur Trait `SessionStore`) |
| `zaehler-storage` | `object_store` 0.14 (Features `aws`) |
| `zaehler-media` | `image` 0.25 (Features `jpeg`, `png`, `webp`), `kamadak-exif` 0.6 (Aufnahmezeit) |
| `zaehler-ai` | Tests `wiremock` 0.6 |
| `zaehler-server` | `opentelemetry` 0.33, `opentelemetry_sdk` 0.33, `opentelemetry-otlp` 0.33, `tracing-opentelemetry` 0.34 |

## Observability

| Metrik | Labels | Quelle |
|---|---|---|
| `http_requests_total`, `http_request_duration_seconds` | method, path, status | vorhanden |
| `zaehler_readings_created_total` | source | `RecordReadings`, `ConfirmDraft`, `CommitImport` |
| `zaehler_ai_requests_total` | outcome (`ok`, `error`, `timeout`, `invalid`) | `AnalysePhoto` |
| `zaehler_ai_request_duration_seconds` | | `AnalysePhoto` |
| `zaehler_ai_corrections_total` | corrected | `ConfirmDraft` |
| `zaehler_import_rows_total` | result | `CommitImport` |
| `zaehler_housekeeping_removed_total` | kind (`draft`, `tmp`, `session`) | `Housekeeping` |

Logs enthalten Request-ID, Route, Haushalts-ID und Nutzer-ID, nie
Zählerwerte, Fotos, Tokens oder E-Mail-Adressen.

## Testdesign

| Ebene | Gegenstand | Werkzeug |
|---|---|---|
| Domain | neue Module `calendar`, `series`, `analysis`; Invarianten (Summe Monatswerte, Zählerwechsel, Zeitumstellung) | Unit-Tests, `proptest` |
| Ports | gemeinsame Port-Testsuite aus `zaehler-app` (Feature `testing`), ausgeführt gegen In-Memory-Adapter, SQLite (in-memory) und PostgreSQL (CI-Service); Tenant-Isolation, Unit of Work (Rollback ohne `commit`), Unique-Index Ablesungen | `tokio::test` |
| Anwendungsfälle | alle Anwendungsfälle mit In-Memory-Adaptern, fester `Clock` und Stubs für `ImageNormalizer` und `MeterRecognizer`; ohne Datenbank und Netzwerk | `tokio::test` |
| AI-Adapter | `zaehler-ai` gegen simulierten OpenRouter: Erfolg, ungültiges JSON, HTTP 500, Timeout | `wiremock` |
| Architektur | Abhängigkeitsregeln {need}`SPEC_ARCH_LAYERS` | CI-Schritt über `cargo metadata` |
| HTTP | Routen mit Dev-Login; CSRF fehlt oder falsch → 403; fremde IDs → 404; Body-Limits → 413; Validierung → 422 | `tower::ServiceExt::oneshot` |
| Medien | Fixtures mit EXIF-Ausrichtung 6 und GPS-Daten: Ergebnis gedreht, ohne EXIF, ≤ 2 MP; HEIC und Textdatei abgelehnt | Unit-Tests |
| CSV | Fixtures: deutsches Format, ISO, Tab, BOM, Duplikate, fehlerhafte Zeilen | Unit-Tests |
| i18n | Schlüsselgleichheit `de`/`en`; Rendern jeder Seite in beiden Sprachen | Unit-Tests |
| Leistung | {need}`SPEC_PERF_BUDGET` | Test im Release-Profil mit Seed-Daten |
| Betrieb | `docker compose up`, `/readyz`, Dev-Login, Messstelle anlegen, Stand erfassen | CI-Smoke-Test mit `curl` |

Jeder Test trägt `// @test SPEC_…`; jede Spezifikation dieser Seite hat zum
Ende der Phase mindestens einen Test.

```{spec} Leistungsbudget
:id: SPEC_PERF_BUDGET
:status: draft
:satisfies: REQ_NFR_006

Seed-Daten: ein Haushalt, 20 Messstellen mit je 2 Registern, 10 Jahre
monatliche Ablesungen (4 800 Ablesungen). Messung im Release-Build gegen
SQLite und PostgreSQL, je 200 Requests nach Aufwärmen: Dashboard,
Auswertungsseite und `chart.json` mit p95 < 300 ms serverseitig. Der Test
läuft in der CI als eigener Job und meldet die Werte; eine Überschreitung
lässt den Job fehlschlagen.
```

## Vorbereitung v1

Das MVP-Schema enthält keine Fahrzeug- und Tariftabellen. Folgende Punkte
halten die geplanten Erweiterungen offen:

- Fahrzeuge sind Messstellen der Art `vehicle`; die Tabellen `vehicles`,
  `vehicle_consumption_profiles` und `charging_sessions` sowie
  `meters.parent_meter_id` für Zwischenzähler ({need}`SPEC_CAR_ENERGY_COST`)
  kommen mit einer v1-Migration hinzu, ohne bestehende Tabellen umzubauen.
- `billing_start_month/day` an `meters` wird mit dem Vertragsmodell
  ({need}`REQ_TAR_001`) durch den Abrechnungszeitraum des Vertrags ersetzt;
  die Migration übernimmt vorhandene Werte.
- `Role` und die Rollenprüfungen sind vorhanden; Einladungen
  ({need}`REQ_TEN_004`) benötigen nur eine Tabelle und Seiten.

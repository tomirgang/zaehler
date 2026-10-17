# v1: Software-Detail-Design

Grundlage: {doc}`v1-architektur` und {doc}`mvp-detail-design`. Signaturen sind
Entwurfsskizzen in Rust-Notation; sie legen Verantwortung, Ein- und Ausgaben
fest, nicht die endgültige Syntax. Fachliche Rechenvorschriften stehen in
{doc}`../architektur/berechnungen` und werden hier nur referenziert.

## Modulstruktur

Nur neue oder geänderte Module:

```text
crates/
  zaehler-domain/src/
    schedule.rs        Ableseplan, Fälligkeit                              SPEC_RDG_SCHEDULE
    forecast.rs        + SeasonProfile::learn, Standardprofile je Art      SPEC_FC_SEASON_PROFILE
    expectation.rs     + Ampel je Monat aus Historie oder Ziel             SPEC_EXP_BAND, SPEC_ANA_TRAFFIC_LIGHT
    tariff.rs          Vertrag, Position, Preisstand, Anpassung, Abschlag (Typen erweitert)
    cost.rs            Kosten eines Zeitraums, Kostenprognose, Saldo       SPEC_TAR_COST_CALC, SPEC_TAR_INSTALLMENT_SETTLEMENT
    assets.rs          Kostenbuch, Wiederholungen, Kategorien je Anlagentyp SPEC_ASSET_COSTS
    vehicle.rs         Energie, Verbrauch, Kosten pro km (ersetzt car.rs)  SPEC_CAR_ENERGY_COST, SPEC_CAR_FUEL_CONSUMPTION, SPEC_CAR_COST_PER_KM
    pv.rs              Kennzahlen, Speicher, Erwartung, Wirtschaftlichkeit SPEC_PV_KPI, SPEC_PV_BATTERY, SPEC_PV_YIELD_EXPECTATION, SPEC_PV_ECONOMICS
  zaehler-app/src/
    tenant.rs          + Tenant::for_job (pub(crate))                       SPEC_JOB_OUTBOX
    port.rs            + neue Ports (siehe unten)
    usecase/
      member.rs        ListMembers, ChangeRole, RemoveMember, LeaveHousehold, CreateInvitation, RevokeInvitation, AcceptInvitation
      reading.rs       + CorrectReading, ReadingHistory, RecordBulkReadings
      meter.rs         + ArchiveMeter, RestoreMeter
      analysis.rs      + Ampel, Saisonprofil, Hausstrom mit/ohne Zwischenzähler
      contract.rs      CreateContract, UpdateContract, DeleteContract, AddPriceComponent, SetInstallment, AddAdjustment, RecordSettlement, ContractTemplates
      cost.rs          MeterCosts, HouseholdCosts
      document.rs      UploadDocument, AnalyseDocument, ConfirmInvoice, ConfirmReceipt, DiscardDocument, GetDocument
      vehicle.rs       CreateVehicle, UpdateVehicle, SetConsumptionProfile, RecordCharging, VehicleReport
      asset_cost.rs    AddAssetCost, UpdateAssetCost, DeleteAssetCost, ListAssetCosts
      pv.rs            CreatePvSystem, UpdatePvSystem, AssignMeters, GridAnalysis, PvSystemReport
      notify.rs        GetNotificationSettings, UpdateNotificationSettings
      jobs.rs          DispatchOutbox, SendReminders (Jobs)
      api.rs           ResolveApiTenant, CreateApiToken, RevokeApiToken, ListApiTokens, IngestReadings
      export.rs        + ExportHouseholdCsv
  zaehler-db/src/
    migration/         m…_000006_members bis m…_000015_api
    lock.rs            PgLeaderLock, SqliteLeaderLock                      SPEC_JOB_SCHEDULER
  zaehler-notify/src/  SmtpNotifier (lettre), locales/{de,en}/notify.ftl   SPEC_NTF_EMAIL
  zaehler-ai/src/      + OpenRouterDocumentExtractor, Schemata Rechnung/Beleg SPEC_AI_INVOICE, SPEC_AI_RECEIPT
  zaehler-web/src/
    api/               mod.rs (Router, OpenAPI), auth.rs (ApiTenant), dto.rs, error.rs  SPEC_API_REST
    pages/             members, invite, contracts, costs, vehicles, assets, pv, documents, notifications, tokens
  zaehler-server/src/
    scheduler.rs       Scheduler mit LeaderLock, Jobs: DispatchOutbox, SendReminders, Housekeeping
```

## Zentrale Typen

```rust
// zaehler-app
pub struct InvitationId(Uuid);  // analog ContractId, PriceComponentId, VehicleId, AssetCostId,
                                // ChargingId, PvSystemId, DocumentId, ApiTokenId, OutboxId
pub enum Role { Viewer, Member, Owner }            // Ord: Viewer < Member < Owner
impl Tenant {
    pub(crate) fn for_job(household: HouseholdId) -> Tenant;   // Rolle Viewer, ohne Nutzer
    pub fn user_id(&self) -> Option<UserId>;                   // None nur beim Job-Tenant
}
pub enum ConflictKind { /* MVP */ DuplicateReading, /* v1 */ LastOwner, ContractOverlap,
                        SubmeterHasContract, DriveTypeLocked, InvitationUsed }

// zaehler-domain
pub enum MeterPurpose { General, HeatPump, Wallbox }
pub enum ReadingSchedule { EveryDays(u16), MonthlyOnDay(u8) }   // 1..=28; ersetzt reading_interval_days
pub enum Direction { Import, Export }                           // vorhanden an Registern
pub enum DriveType { Combustion, Electric, PluginHybrid }
pub enum AssetType { Vehicle, PvSystem }
pub enum AssetCategory { Vehicle(VehicleCostCategory), Pv(PvCostCategory) }
pub enum PvRole { Grid, Generation, Battery }
pub enum Traffic { Green, Yellow, Red }
pub struct Estimated<T> { pub value: T, pub estimated: bool }   // Kennzeichnung von Schätzungen
```

`ReadingSource` erhält die Werte `invoice` (aus Rechnungsimport) und `api`
({need}`SPEC_API_DAILY_READINGS`).

## Datenmodell: Migrationen v1

Jede Migration gehört zu genau einem Arbeitspaket, wird nur angehängt und
nach Freigabe mit ihrer Prüfsumme in `upgrade_tests.rs` eingetragen
({need}`SPEC_OPS_UPGRADE`). Mit jeder Migration werden Export und Import
({need}`SPEC_BACKUP_DATA`) und die In-Memory-Adapter erweitert. Der Datumsteil
des Namens wird beim Anlegen vergeben.

| Migration | Arbeitspaket | Änderung |
|---|---|---|
| `…_000006_members` | WP_V1_01 | `invitations` (neu) |
| `…_000007_reading_revisions` | WP_V1_02 | `reading_revisions` (neu); `readings.note`, `readings.updated_at` |
| `…_000008_meter_v1` | WP_V1_03 | `meters.purpose`, `meters.schedule_kind`, `meters.schedule_value`, `meters.archived_at`; `meter_devices.parent_meter_id` |
| `…_000009_expectation` | WP_V1_04 | `meters.season_weights` (TEXT JSON, NULL), `meters.annual_target_milli`, `meters.target_tolerance_pct` |
| `…_000010_jobs` | WP_V1_05, WP_V1_06 | `outbox`, `job_runs`, `notification_log`, `notification_settings` (neu) |
| `…_000011_tariffs` | WP_V1_07 | `contracts`, `cost_positions`, `price_components`, `installments`, `adjustments`, `settlements` (neu; `settlements.document_id` ohne Fremdschlüssel) |
| `…_000012_documents` | WP_V1_10 | `documents` (neu, mit Status, Modell und Erkennungsergebnis als JSON); `ai_drafts` bleibt unverändert |
| `…_000013_vehicles` | WP_V1_11, WP_V1_12 | `vehicles`, `vehicle_consumption_profiles`, `vehicle_acquisitions`, `charging_sessions`, `asset_costs` (neu) |
| `…_000014_pv` | WP_V1_15 | `pv_systems`, `pv_system_meters` (neu) |
| `…_000015_api` | WP_V1_20 | `api_tokens` (neu); `readings.api_token_id` |

Abweichungen vom geplanten Modell in {doc}`../architektur/datenmodell`:

- `parent_meter_id` liegt an `meter_devices` statt an `meters`. Ein Wechsel
  vom Zwischenzähler zum eigenen Wärmepumpenzähler ist damit ein Zählerwechsel
  ({need}`REQ_MTR_008`), und die Zuordnung gilt genau für die Einbauzeit des
  Geräts.
- `season_profiles` entfällt: das gelernte Profil wird bei jeder Auswertung
  berechnet ({need}`SPEC_FC_SEASON_PROFILE`), gespeichert wird nur ein
  manuell angepasstes Standardprofil in `meters.season_weights`.
- `meters.reading_interval_days` bleibt erhalten; die Migration setzt
  `schedule_kind = 'every_days'` und `schedule_value` aus dem vorhandenen Wert.
- `meters.billing_start_*` bleibt als Rückfall für Messstellen ohne Vertrag.

Tabellen (Auszug; Mengen `*_milli`, Beträge `*_micro`, {need}`SPEC_STORE_FIXED_POINT`):

| Tabelle | Spalten |
|---|---|
| `invitations` | `id`, `household_id` FK Cascade, `role`, `token_hash` UNIQUE, `email` NULL, `created_by`, `created_at`, `expires_at`, `accepted_by` NULL, `accepted_at` NULL, `revoked_at` NULL |
| `reading_revisions` | `id`, `household_id` FK Cascade, `reading_id` (ohne FK, bleibt nach Löschen), `meter_id`, `register_id`, `action` (`update`, `delete`), `changed_by`, `changed_at`, `old_read_at`, `old_value_milli`, `new_read_at` NULL, `new_value_milli` NULL, `old_note`, `new_note` |
| `outbox` | `id`, `household_id` FK Cascade, `kind`, `payload` TEXT JSON, `status` (`pending`, `done`, `failed`), `attempts`, `next_attempt_at`, `created_at`, `finished_at`; Index `(status, next_attempt_at)` |
| `notification_log` | `id`, `household_id`, `user_id`, `kind`, `subject_key` (z.B. `meter:{id}:2026-10-16`), `sent_at`; UNIQUE `(user_id, kind, subject_key)` |
| `notification_settings` | `user_id` PK, `email` NULL, `events` TEXT JSON, `updated_at` |
| `job_runs` | `job` PK, `last_started_at`, `last_finished_at`, `last_outcome` |
| `contracts` | `id`, `meter_id` FK Cascade, `direction`, `provider`, `contract_number` NULL, `starts_on`, `ends_on` NULL, `billing_start_month`, `billing_start_day`, `note` |
| `cost_positions` | `id`, `contract_id` FK Cascade, `name`, `register_id` NULL (NULL = alle Register der Richtung), `factor_micro`, `vat_pct_milli` NULL, `sort` |
| `price_components` | `id`, `position_id` FK Cascade, `valid_from`, `base_price_micro`, `base_interval` (`day`, `month`, `year`), `unit_pricing` TEXT JSON, `factor_micro` NULL (überschreibt die Position) |
| `installments` | `id`, `contract_id` FK Cascade, `valid_from`, `amount_micro`, `interval_months` |
| `adjustments` | `id`, `contract_id` FK Cascade, `kind` (`bonus`, `credit`), `booked_on`, `amount_micro`, `note` |
| `settlements` | `id`, `contract_id` FK Cascade, `period_start`, `period_end`, `amount_micro`, `document_id` NULL |
| `documents` | `id`, `household_id` FK Cascade, `blob_key`, `media_type`, `bytes`, `created_by`, `created_at`, `original_name` |
| `vehicles` | `meter_id` PK FK Cascade, `plate`, `model`, `drive_type`, `home_charging_meter_id` NULL, `pv_share_override_pct_milli` NULL |
| `vehicle_consumption_profiles` | `id`, `vehicle_id`, `valid_from`, `kwh_per_100km_milli`, `charging_loss_pct_milli`, `electric_share_pct_milli` NULL |
| `vehicle_acquisitions` | `vehicle_id` PK, `purchased_on`, `price_micro`, `residual_micro`, `planned_end_on`, `sold_on` NULL, `sale_price_micro` NULL |
| `charging_sessions` | `id`, `vehicle_id`, `charged_at`, `energy_milli`, `amount_micro`, `odometer_milli` NULL, `provider` NULL, `kind` (`ac`, `dc`), `source` (`form`, `receipt`), `document_id` NULL |
| `asset_costs` | `id`, `household_id`, `asset_type`, `asset_id`, `booked_on`, `category`, `amount_micro`, `quantity_milli` NULL, `odometer_milli` NULL, `full_tank` NULL, `recurrence_months` NULL, `recurrence_until` NULL, `document_id` NULL, `note` |
| `pv_systems` | `id`, `household_id`, `name`, `kind`, `kwp_milli`, `commissioned_on`, `azimuth`, `tilt`, `inverter`, `battery_kwh_milli`, `specific_yield_milli`, `degradation_pct_milli`, `month_shares` TEXT JSON NULL, `alarm_threshold_pct` |
| `pv_system_meters` | `pv_system_id`, `meter_id`, `role`; PK `(pv_system_id, meter_id)`; Eindeutigkeit von `generation` und `battery` je Messstelle wird im Anwendungsfall und per Teilindex geprüft |
| `api_tokens` | `id`, `household_id` FK Cascade, `user_id` FK Cascade, `name`, `role`, `token_hash` UNIQUE, `prefix`, `created_at`, `expires_at` NULL, `revoked_at` NULL, `last_used_at` NULL |

Export: `outbox`, `notification_log`, `job_runs` und `sessions` sind
Betriebsdaten und nicht Teil der Archive. `invitations` und `api_tokens` sind
im Installationsarchiv enthalten, im Haushaltsarchiv nicht, da dort die
Nutzerkonten fehlen ({need}`SPEC_BACKUP_HOUSEHOLD`).

## Driven Ports: Erweiterungen

```rust
trait UnitOfWork {                                      // MVP-Methoden unverändert
    fn invitations(&mut self) -> &mut dyn InvitationRepository;
    fn revisions(&mut self) -> &mut dyn RevisionRepository;
    fn contracts(&mut self) -> &mut dyn ContractRepository;
    fn vehicles(&mut self) -> &mut dyn VehicleRepository;
    fn asset_costs(&mut self) -> &mut dyn AssetCostRepository;
    fn pv(&mut self) -> &mut dyn PvRepository;
    fn documents(&mut self) -> &mut dyn DocumentRepository;
    fn outbox(&mut self) -> &mut dyn OutboxRepository;
    fn notification_settings(&mut self) -> &mut dyn NotificationSettingsRepository;
    fn api_tokens(&mut self) -> &mut dyn ApiTokenRepository;
}
trait MembershipRepository {                            // + v1
    async fn list(&mut self, t: &Tenant) -> PortResult<Vec<Member>>;
    async fn lock_household(&mut self, t: &Tenant) -> PortResult<()>;    // PostgreSQL: SELECT … FOR UPDATE
    async fn set_role(&mut self, t: &Tenant, user: UserId, role: Role) -> PortResult<()>;
    async fn remove(&mut self, t: &Tenant, user: UserId) -> PortResult<()>;
    async fn add(&mut self, household: HouseholdId, user: UserId, role: Role) -> PortResult<()>; // nur AcceptInvitation
}
trait InvitationRepository {
    async fn create(&mut self, t: &Tenant, i: NewInvitation) -> PortResult<InvitationId>;
    async fn find_by_hash(&mut self, hash: &[u8; 32]) -> PortResult<Option<Invitation>>;  // ohne Tenant: Einlösen
    async fn mark_accepted(&mut self, id: InvitationId, user: UserId, at: DateTime<Utc>) -> PortResult<bool>; // false: bereits benutzt
    async fn list_open(&mut self, t: &Tenant, now: DateTime<Utc>) -> PortResult<Vec<Invitation>>;
    async fn revoke(&mut self, t: &Tenant, id: InvitationId, at: DateTime<Utc>) -> PortResult<()>;
}
trait ReadingRepository {                               // + v1
    async fn get(&mut self, t: &Tenant, id: ReadingId) -> PortResult<Option<ReadingRow>>;
    async fn update(&mut self, t: &Tenant, id: ReadingId, u: ReadingUpdate) -> PortResult<()>;
    async fn upsert_daily(&mut self, t: &Tenant, r: NewReading, token: ApiTokenId, day: Period) -> PortResult<DailyOutcome>;
    async fn list_for_meters(&mut self, t: &Tenant, ids: &[MeterId]) -> PortResult<Vec<ReadingRow>>;
}
trait RevisionRepository {
    async fn append(&mut self, t: &Tenant, r: NewRevision) -> PortResult<()>;
    async fn for_meter(&mut self, t: &Tenant, meter: MeterId) -> PortResult<Vec<Revision>>;
}
trait ContractRepository {
    async fn for_meter(&mut self, t: &Tenant, meter: MeterId) -> PortResult<Vec<ContractAggregate>>; // inkl. Positionen, Preisstände, Abschläge, Anpassungen, Abrechnungen
    async fn get(&mut self, t: &Tenant, id: ContractId) -> PortResult<Option<ContractAggregate>>;
    async fn create(&mut self, t: &Tenant, c: NewContract) -> PortResult<ContractId>;
    async fn update(&mut self, t: &Tenant, id: ContractId, c: ContractUpdate) -> PortResult<()>;   // ersetzt Positionen und Preisstände
    async fn delete(&mut self, t: &Tenant, id: ContractId) -> PortResult<()>;
    async fn add_installment(&mut self, t: &Tenant, id: ContractId, i: Installment) -> PortResult<()>;
    async fn add_adjustment(&mut self, t: &Tenant, id: ContractId, a: Adjustment) -> PortResult<()>;
    async fn add_settlement(&mut self, t: &Tenant, id: ContractId, s: NewSettlement) -> PortResult<()>;
}
trait VehicleRepository { /* get, list, create, update, set_profile, set_acquisition, charging_sessions, add_charging */ }
trait AssetCostRepository {
    async fn list(&mut self, t: &Tenant, asset: AssetRef) -> PortResult<Vec<AssetCost>>;
    async fn create(&mut self, t: &Tenant, c: NewAssetCost) -> PortResult<AssetCostId>;
    async fn update(&mut self, t: &Tenant, id: AssetCostId, c: NewAssetCost) -> PortResult<()>;
    async fn delete(&mut self, t: &Tenant, id: AssetCostId) -> PortResult<Option<DocumentId>>;
}
trait PvRepository { /* list, get, create, update, delete, assign(system, Vec<(MeterId, PvRole)>), systems_for_grid(meter) */ }
trait DocumentRepository { /* create, get, delete, unreferenced_older_than (Housekeeping) */ }
trait OutboxRepository { async fn enqueue(&mut self, t: &Tenant, e: OutboxEvent) -> PortResult<()>; }
trait JobRepository {                                   // ohne Tenant, nur Jobs
    async fn claim_outbox(&self, now: DateTime<Utc>, limit: u32) -> PortResult<Vec<OutboxEntry>>;
    async fn finish_outbox(&self, id: OutboxId, r: OutboxResult) -> PortResult<()>;
    async fn households_with_schedules(&self) -> PortResult<Vec<HouseholdId>>;
    async fn recipients(&self, household: HouseholdId, kind: NotificationKind) -> PortResult<Vec<Recipient>>; // Nutzer, Adresse, Sprache
    async fn log_once(&self, entry: NotificationLogEntry) -> PortResult<bool>;  // false: bereits versendet
    async fn record_run(&self, job: &str, run: JobRun) -> PortResult<()>;
}
trait NotificationSettingsRepository { /* get(user), set(user, settings) */ }
trait ApiTokenRepository {
    async fn create(&mut self, t: &Tenant, tok: NewApiToken) -> PortResult<ApiTokenId>;
    async fn find_by_hash(&mut self, hash: &[u8; 32]) -> PortResult<Option<ApiTokenRow>>;  // ohne Tenant: Authentifizierung
    async fn list(&mut self, t: &Tenant) -> PortResult<Vec<ApiTokenRow>>;
    async fn revoke(&mut self, t: &Tenant, id: ApiTokenId, at: DateTime<Utc>) -> PortResult<()>;
    async fn touch(&mut self, id: ApiTokenId, at: DateTime<Utc>) -> PortResult<()>;
}
trait Notifier {
    fn enabled(&self) -> bool;
    async fn send(&self, to: &EmailAddress, n: &Notification, locale: Locale) -> Result<(), PortError>;
}
trait LeaderLock { async fn try_acquire(&self) -> PortResult<bool>; async fn is_held(&self) -> bool; }
trait DocumentExtractor {
    fn model(&self) -> &str;
    async fn invoice(&self, doc: &DocumentContent) -> Result<InvoiceExtraction, RecognitionError>;
    async fn receipt(&self, doc: &DocumentContent) -> Result<ReceiptExtraction, RecognitionError>;
}
```

`Notification` ist ein Enum mit fachlichen Daten (`ReadingDue`,
`ReadingOverdue`, `ConsumptionAlarm`, `PvYieldAlarm`, `Invitation`); Texte
erzeugt der Adapter. Damit bleiben Fluent-Ressourcen außerhalb des Kerns.

## Anwendungsfälle

Mindestrolle gemäß {need}`SPEC_TEN_ROLES`: `V` Viewer, `M` Member, `O` Owner,
`J` nur Job.

| Bereich | Anwendungsfälle | Rolle |
|---|---|---|
| Mitglieder | `ListMembers` V; `ChangeRole`, `RemoveMember`, `CreateInvitation`, `RevokeInvitation` O; `LeaveHousehold` V; `AcceptInvitation` (angemeldeter Nutzer ohne Tenant) | |
| Erfassung | `CorrectReading`, `RecordBulkReadings` M; `ReadingHistory` V | |
| Messstellen | `ArchiveMeter`, `RestoreMeter` O | |
| Verträge | `CreateContract`, `UpdateContract`, `DeleteContract`, `AddPriceComponent`, `SetInstallment` O; `AddAdjustment`, `RecordSettlement` M; `ContractTemplates` V | |
| Kosten | `MeterCosts`, `HouseholdCosts` V | |
| Dokumente | `UploadDocument`, `AnalyseDocument`, `ConfirmInvoice`, `ConfirmReceipt`, `DiscardDocument` M; `GetDocument` V | |
| Fahrzeuge | `CreateVehicle`, `UpdateVehicle`, `SetConsumptionProfile`, `SetAcquisition` O; `RecordCharging` M; `VehicleReport` V | |
| Kostenbuch | `AddAssetCost`, `UpdateAssetCost`, `DeleteAssetCost` M; `ListAssetCosts` V | |
| PV | `CreatePvSystem`, `UpdatePvSystem`, `DeletePvSystem`, `AssignMeters` O; `GridAnalysis`, `PvSystemReport` V | |
| Benachrichtigung | `GetNotificationSettings`, `UpdateNotificationSettings` V (nur eigener Nutzer); `DispatchOutbox`, `SendReminders` J | |
| API | `CreateApiToken`, `RevokeApiToken`, `ListApiTokens` V (eigene), O (alle); `ResolveApiTenant`; `IngestReadings` M | |
| Export | `ExportHouseholdCsv` O | |

### Mitglieder und Einladungen

```{spec} Einladungen
:id: SPEC_TEN_INVITES
:status: implemented
:satisfies: REQ_TEN_004, REQ_TEN_003
:links: SPEC_TEN_ROLES, SPEC_NTF_EMAIL

`CreateInvitation` (Owner) erzeugt 32 Zufallsbytes, speichert SHA-256 mit
Rolle und `expires_at = now + 7 Tage` und gibt den Link
`{public_url}/invite/{token}` einmalig zurück. Optional mit E-Mail-Adresse:
dann versendet der Anwendungsfall die Einladung über eine Outbox-Nachricht
(`Notification::Invitation`); ohne SMTP wird nur der Link angezeigt.
`GET /invite/{token}` ohne Sitzung speichert den Token in der Sitzung und
leitet zur Anmeldung, danach zurück. `AcceptInvitation` prüft in einer Unit of
Work: Hash bekannt, nicht widerrufen, nicht abgelaufen, `mark_accepted` liefert
`true` (bedingtes Update auf `accepted_at IS NULL`, damit gleichzeitiges
Einlösen nur einmal gelingt). Ist der Nutzer schon Mitglied, bleibt die höhere
Rolle erhalten. Danach wird der Haushalt aktiv. Fehlerfälle ergeben eine Seite
„Einladung ungültig oder abgelaufen“ ohne Angabe des Grundes.

Umsetzung: Das Token besteht aus zwei UUIDv4 (64 Hex-Zeichen, 244 Bit
Zufall); die Gültigkeit ist über `invitations.ttl_days` (Standard 7)
einstellbar. Für den E-Mail-Versand steht das Token im Outbox-Eintrag
`invitation_created`, bis dieser erledigt ist und vom Housekeeping entfernt
wird; der Link ist nach Annahme, Widerruf oder Ablauf wertlos. Zusätzlich
sind `ListInvitations` und `PreviewInvitation` (Anzeige vor der Annahme)
umgesetzt.
```

### Erfassung

```{spec} Korrektur und Löschen mit Historie
:id: SPEC_RDG_HISTORY
:status: implemented
:satisfies: REQ_RDG_003
:links: SPEC_RDG_PLAUSIBILITY, SPEC_RDG_DELETE

`CorrectReading` ändert Zeitpunkt, Wert und Notiz einer Ablesung. Die
Plausibilität wird wie bei der Erfassung gegen die zeitlichen Nachbarn ohne
die geänderte Ablesung geprüft ({need}`SPEC_RDG_PLAUSIBILITY`), Warnungen
erfordern Bestätigung. Im selben Commit schreibt der Anwendungsfall eine
Revision mit altem und neuem Wert, Nutzer und Zeitpunkt; `DeleteReading`
schreibt eine Revision mit `action = delete`. Revisionen sind nicht änderbar,
bleiben nach dem Löschen der Ablesung erhalten und werden nur mit dem
Haushalt gelöscht. Die Detailseite der Messstelle zeigt je Ablesung den
Hinweis „geändert“ und eine Liste der Änderungen (`ReadingHistory`).
Zählerwechsel-Ablesungen (End- und Startstand) sind nur gemeinsam änderbar:
der Zeitpunkt beider Ablesungen ändert sich zusammen.

Umsetzung: Zusätzlich `GetReading` (Ablesung mit Kontext, ob sie zu einem
Zählerwechsel gehört) und der Port-Typ `ReadingUpdate`; eine Zeitänderung
beim Zählerwechsel verschiebt auch Aus- und Einbauzeit der Geräte.
```

```{spec} Ableseplan und Fälligkeit
:id: SPEC_RDG_SCHEDULE
:status: implemented
:satisfies: REQ_RDG_005, REQ_NTF_001

`ReadingSchedule::EveryDays(n)`: fällig am Tag `letzte Ablesung + n`.
`MonthlyOnDay(d)`: fällig am nächsten Tag *d* nach dem Tag der letzten
Ablesung; eine Ablesung bis zu 3 Tage vor dem Stichtag zählt für diesen
Stichtag. Ohne Ablesung ist eine Messstelle mit Plan sofort fällig. Überfällig
ist eine Messstelle ab dem Tag nach der Fälligkeit (Kalendertage in der
Zeitzone des Haushalts). Archivierte Messstellen sind nie fällig. Das
Dashboard ({need}`SPEC_UI_DASHBOARD`) zeigt fällig und überfällig getrennt und
sortiert überfällige zuerst.

Umsetzung: Statt einer Datenmigration gilt bei fehlendem `schedule_kind` der
bisherige Turnus `reading_interval_days` als `EveryDays(n)`; beim Speichern
eines Plans `EveryDays` wird `reading_interval_days` weiter geschrieben, damit
ältere Programmversionen nach einem Rückschritt denselben Turnus sehen.
```

### Messstellen

```{spec} Archivieren von Messstellen
:id: SPEC_MTR_ARCHIVE
:status: implemented
:satisfies: REQ_MTR_007

`ArchiveMeter` setzt `archived = true` und `archived_at`; `RestoreMeter`
hebt das auf. Archivierte Messstellen erscheinen nicht im Dashboard, in der
Auswahl der Erfassung, der Foto-Zuordnung, der Sammelerfassung, im CSV-Import
und bei Erinnerungen, aber in der Messstellenliste (Filter „archiviert“) und
mit allen Auswertungen und Kosten vergangener Zeiträume. Neue Ablesungen für
archivierte Messstellen ergeben `Validation`. Verweisen Fahrzeug, PV-Anlage
oder Zwischenzähler auf die Messstelle, bleibt der Verweis gültig.
```

```{spec} Zwischenzähler und Verwendungszweck
:id: SPEC_MTR_SUBMETER
:status: implemented
:satisfies: REQ_MTR_008, REQ_CAR_008, REQ_MTR_002

Eine Strom-Messstelle hat einen Verwendungszweck (`general`, `heat_pump`,
`wallbox`). Ein Gerät einer Strom-Messstelle kann auf eine übergeordnete
Strom-Messstelle desselben Haushalts verweisen (`parent_meter_id`); die
Zuordnung gilt für die Einbauzeit des Geräts. Regeln:

- keine Ketten: die übergeordnete Messstelle ist selbst kein Zwischenzähler im
  betreffenden Zeitraum; keine Zuordnung auf sich selbst;
- solange ein Gerät mit übergeordneter Messstelle eingebaut ist, darf kein
  Vertrag der Messstelle diesen Zeitraum überschneiden
  (`Conflict(SubmeterHasContract)`);
- der Wechsel auf einen eigenen Zähler ist ein Zählerwechsel
  ({need}`SPEC_MTR_SWAP_FLOW`) mit einem neuen Gerät ohne Verweis.

Die Auswertung der übergeordneten Messstelle bietet die Umschaltung
„mit Zwischenzählern“ und „ohne Zwischenzähler“ (`?submeters=exclude`): ohne
Zwischenzähler wird je Monat der Verbrauch aller im Monat zugeordneten
Zwischenzähler abgezogen; ist einer davon im Monat undefiniert, ist der
Monatswert undefiniert. Die Art „Sonstiges“ mit freier Einheit
({need}`REQ_MTR_002`) bleibt unverändert und wird in der Abnahme mit Heizöl
(l) und Pellets (kg) geprüft.

Umsetzung: Die Umschaltung wirkt auf Monatstabelle und Vergleiche;
Prognosen und Diagramme enthalten weiterhin den Verbrauch der
Zwischenzähler. Die Regel zu Verträgen prüfen `CreateContract`,
`UpdateContract`, `UpdateMeter` und `SwapDevice` (Code `contract.submeter`).
`meter_devices.parent_meter_id` hat keinen Fremdschlüssel, weil
SQLite ihn nicht nachträglich anlegen kann; die Prüfung liegt im Anwendungsfall.
```

### Auswertung

```{spec} Saisonprofil und Ampel in der Auswertung
:id: SPEC_ANA_TRAFFIC_LIGHT
:status: implemented
:satisfies: REQ_ANA_006, REQ_ANA_002
:links: SPEC_EXP_BAND, SPEC_FC_SEASON_PROFILE, SPEC_FC_CURRENT_YEAR

`MeterAnalysis` bildet das Saisonprofil: gelernt aus der Monatsserie, sobald
mindestens 12 aufeinanderfolgende vollständige Monate vorliegen; sonst
`meters.season_weights`, sonst das Standardprofil der Art (Strom, Wasser,
Fahrzeug, Sonstiges: Gleichverteilung; Gas und Wärme: Jan 17, Feb 15, Mär 12,
Apr 8, Mai 4, Jun 2, Jul 2, Aug 2, Sep 4, Okt 8, Nov 12, Dez 14 Prozent). Das
Profil ersetzt `SeasonProfile::uniform()` in {need}`SPEC_FC_CURRENT_YEAR` und
wird mit seiner Herkunft angezeigt.

Ampel je Monat: mit mindestens zwei Werten desselben Kalendermonats aus
Vorjahren Band nach {need}`SPEC_EXP_BAND`; sonst mit Jahresziel
(`annual_target`) oder, falls ein Vertrag mit Abschlag besteht, mit der
Verbrauchsmenge, die der Abschlag beim aktuellen Arbeitspreis deckt, verteilt
mit dem Saisonprofil, und Toleranz `target_tolerance_pct` (Standard 20 %);
sonst keine Ampel. Bei einem Hausanschluss mit Erzeugungszähler bewertet die
Ampel den Gesamtverbrauch ({need}`SPEC_PV_KPI`). Angezeigt wird die Ampel des
Bezugsmonats auf der Detailseite und im Dashboard sowie je Monat in der
Tabelle.

Umsetzung: Bezugsmonat ist der letzte vollständige Monat. „Abschlagsbasis“
nutzt den heute gültigen Bezugsvertrag: Jahresbetrag der Abschläge, abzüglich
der Grundpreise, umgerechnet mit den heute gültigen Arbeitspreisen (auch
Staffeln und Faktoren) in eine Jahresmenge; Positionen auf einem einzelnen
Register zählen näherungsweise mit dem Anteil 1 / Anzahl der Bezugsregister.
Die Quelle wird als „Abschlag“ angezeigt. Die Bewertung des
Gesamtverbrauchs bei PV-Anlagen folgt mit M8.
```

### Benachrichtigungen

```{spec} E-Mail-Versand
:id: SPEC_NTF_EMAIL
:status: implemented
:satisfies: REQ_NTF_002, REQ_NTF_005
:links: ADR_011

`SmtpNotifier` (`lettre`, `AsyncSmtpTransport<Tokio1Executor>`, rustls):
`notify.smtp.host`, `notify.smtp.port`, `notify.smtp.security` (`starttls`
Standard, `tls`, `none` nur bei `localhost`), `notify.smtp.username`,
`notify.smtp.password` (Geheimnis), `notify.smtp.from`, Timeout 30 s. Ohne
`notify.smtp` ist `enabled() == false`; die
Einstellungsseite zeigt dann einen Hinweis, Einladungen nur als Link. Jede
E-Mail ist Text (UTF-8) mit Betreff und Link auf die betroffene Seite, in der
Sprache des Empfängers. Einstellungen je Nutzer: Adresse (Standard: E-Mail aus
dem OIDC-Login, falls vorhanden), Ereignistypen `reminder`, `alarm`,
`pv_alarm` (Standard alle an), Abmeldelink in jeder E-Mail führt auf die
Einstellungsseite. E-Mail-Adressen erscheinen nicht in Logs.

Umsetzung: Der Konfigurationsschlüssel `notify.smtp.*` aus dem MVP bleibt
erhalten. Der Ereignistyp `pv_alarm` entsteht mit M8; bis dahin speichert
`notification_settings` `reminder` und `alarm`. SMTP-Fehler werden nur als
Kategorie (`permanent`, `transient`, `timeout`, `tls`, `connection` mit
Statuscode) weitergegeben, weil Serverantworten Adressen enthalten können;
Logs nennen nur die Nutzer-ID. Die Einstellungen sind im
Installationsarchiv enthalten, Versandprotokoll und Outbox nicht.
```

```{spec} Ablese-Erinnerungen
:id: SPEC_NTF_REMINDERS
:status: implemented
:satisfies: REQ_NTF_001
:links: SPEC_RDG_SCHEDULE, SPEC_JOB_SCHEDULER

Job `SendReminders` (stündlich): je Haushalt mit Ableseplan die fälligen und
überfälligen Messstellen ({need}`SPEC_RDG_SCHEDULE`) bestimmen. Je Empfänger
(Mitglied mit Rolle ≥ Member und Ereignistyp `reminder`) höchstens eine
E-Mail je Kalendertag des Haushalts, die alle fälligen Messstellen
zusammenfasst; Versand frühestens ab 08:00 Uhr lokal. Der Schlüssel
`reminder:{household}:{datum}` in `notification_log` verhindert den
Doppelversand.
```

```{spec} Abweichungsalarme
:id: SPEC_NTF_ALARMS
:status: implemented
:satisfies: REQ_NTF_004, REQ_PV_008
:links: SPEC_ANA_TRAFFIC_LIGHT, SPEC_PV_MONITORING, SPEC_JOB_OUTBOX

`DispatchOutbox` verarbeitet `readings_recorded`: für die Messstelle wird der
Verbrauch seit der vorherigen Ablesung als Tagesrate gegen die
saisonbereinigte Erwartung desselben Zeitraums bewertet (Band aus
{need}`SPEC_ANA_TRAFFIC_LIGHT`, umgerechnet auf die Tage des Intervalls). Rot
ergibt einen Alarm an Empfänger mit Ereignistyp `alarm`. Ist die Messstelle
Erzeugungszähler einer PV-Anlage, gilt zusätzlich {need}`SPEC_PV_MONITORING`
(Ereignistyp `pv_alarm`). Je Ablesung und Empfänger höchstens ein Alarm
(Schlüssel `alarm:{reading}`). Intervalle unter 24 h werden nicht bewertet.

Umsetzung: Bewertet wird die Summe des Intervalls gegen die anteilige Summe
der Monatsbänder (gewichtet mit dem Saisonprofil), was der Tagesrate
entspricht. Der PV-Teil folgt mit M8. Archivierte Messstellen lösen keinen
Alarm aus.
```

### Tarife und Kosten

```{spec} Verträge und Preisstände
:id: SPEC_TAR_CONTRACTS
:status: implemented
:satisfies: REQ_TAR_001, REQ_TAR_002, REQ_TAR_003, REQ_TAR_004, REQ_TAR_010
:links: ADR_008, SPEC_TAR_TIERED, SPEC_TAR_BASE_PRORATION

Ein Vertrag gehört zu einer Messstelle und einer Richtung (`import` für Bezug,
`export` für Einspeisevergütung). Verträge derselben Messstelle und Richtung
überlappen nicht (`Conflict(ContractOverlap)`); ein Anbieterwechsel ist ein
neuer Vertrag. Abrechnungszeitraum: Beginn über Monat und Tag, Dauer
12 Monate. Positionen beziehen sich auf ein Register oder auf alle Register
der Richtung und haben einen Umrechnungsfaktor (Standard 1); ein Preisstand
kann den Faktor ab seinem `valid_from` überschreiben (z.B. jährlich neuer
Brennwert × Zustandszahl). Preisstände: Grundpreis mit Intervall (Tag, Monat,
Jahr), Arbeitspreis als `UnitPricing::Flat` oder `Tiered`; `Dynamic` ist im
Modell vorhanden, wird in v1 aber nicht angeboten ({need}`REQ_TAR_007`,
später). Alle Preise brutto in EUR; optionaler MwSt-Satz je Position nur für
die Netto-Anzeige. Vorlagen (`ContractTemplates`) belegen Positionen vor:
Strom (Arbeitspreis, Grundpreis), Strom HT/NT (je Register), Gas (Faktor
Brennwert × Zustandszahl), Wasser (Frischwasser, Schmutzwasser, Zählermiete),
Wärme, Einspeisung (negativer Arbeitspreis). Vertragsänderungen nach dem
Beginn eines Abrechnungszeitraums wirken über neue Preisstände, nicht über
Ändern alter.

Umsetzung: Migration `m20261017_000011_tariffs`; Beträge und Preise als
Festkomma 10⁻⁶, Mengen 10⁻³. Konflikte als `Conflict` mit den Codes
`contract.overlap` und `contract.submeter`. Zusätzlich zu den Vorlagen der
Spezifikation gibt es „Sonstiges“ (eine Position „Kosten“); die Gas-Vorlage
lässt den Faktor leer, damit er aus der Rechnung eingetragen wird. Neue
Preisstände ergänzt `AddPriceComponent` auf der Vertragsseite; `UpdateContract`
ersetzt Positionen und Preisstände vollständig. Für Fahrzeuge gibt es keine
Verträge (`contract.meter_kind_invalid`).
```

```{spec} Kosten von Zwischenzählern
:id: SPEC_TAR_SUBMETER_COST
:status: implemented
:satisfies: REQ_MTR_008, REQ_CAR_008
:links: SPEC_MTR_SUBMETER, SPEC_CAR_ENERGY_COST

Kosten eines Zwischenzählers in einem Zeitraum *P* = Verbrauch des
Zwischenzählers in *P* × `p_haus` der übergeordneten Messstelle in *P*
(durchschnittlicher Arbeitspreis ohne Grundpreis und Anpassungen,
{need}`SPEC_CAR_ENERGY_COST`). Für Zeiträume, in denen ein eigener Vertrag
gilt, rechnet der eigene Vertrag. Die Kostenansicht der übergeordneten
Messstelle weist den Anteil der Zwischenzähler getrennt aus; die Summe bleibt
der Rechnungsbetrag des Vertrags.

Umsetzung: `p_haus` ist der durchschnittliche Arbeitspreis des Bezugsvertrags
der übergeordneten Messstelle im jeweiligen Abrechnungsabschnitt. Die
Haushaltssumme zählt Zwischenzähler nicht doppelt; sie erscheinen dort mit
dem Hinweis „enthalten in“.
```

```{spec} Kostenansicht und Kostenprognose
:id: SPEC_TAR_COST_VIEW
:status: implemented
:satisfies: REQ_FC_002, REQ_TAR_005, REQ_TAR_009, REQ_MTR_004
:links: SPEC_TAR_COST_CALC, SPEC_TAR_INSTALLMENT_SETTLEMENT, SPEC_FC_PERIOD_FORECAST

`MeterCosts` liefert für den laufenden Abrechnungszeitraum des gültigen
Vertrags und für das laufende Kalenderjahr:

- Ist-Kosten bis zum letzten gemessenen Tag ({need}`SPEC_TAR_COST_CALC`),
  aufgeteilt nach Position und Preisstand, Grundpreis und Arbeitspreis;
- Prognose der restlichen Tage: Menge je Teilzeitraum aus
  `forecast::project` mit dem Saisonprofil ({need}`SPEC_ANA_TRAFFIC_LIGHT`),
  bewertet mit dem jeweils gültigen Preisstand; ohne künftigen Preisstand
  gilt der letzte bekannte; Staffeln werden über die kumulierte Menge aus Ist
  und Prognose angewendet;
- Anpassungen (Boni, Gutschriften) mit Buchungsdatum im Zeitraum;
- Einspeisung als Gutschrift aus einem Vertrag mit Richtung `export`
  ({need}`SPEC_PV_FEED_IN`), getrennt vom Bezug ausgewiesen;
- gezahlte und noch fällige Abschläge, Saldo und Vorschlag für den neuen
  Abschlag ({need}`SPEC_TAR_INSTALLMENT_SETTLEMENT`).

Alle Beträge tragen die Kennzeichnung gemessen oder prognostiziert; der
Abdeckungsgrad wird mit angezeigt. Ohne Vertrag zeigt die Seite einen Verweis
auf „Vertrag anlegen“. `HouseholdCosts` summiert alle Messstellen des
Haushalts für das Kalenderjahr.

Umsetzung: Die Prognose nutzt nicht `forecast::project`, sondern
`cost::Quantities`: Menge je Teilzeitraum = Gewicht des Saisonprofils ×
Rate des gemessenen Anteils im Abrechnungszeitraum (ohne Messung darin: die
letzten bis zu 365 gemessenen Tage). Damit sind Ist und Prognose an jeder
Grenze von Preisständen additiv (Property-Test). Mengen werden auf lokaler
Kalenderzeit interpoliert (jeder Tag gleich lang), damit Zeitumstellungen den
Tagesanteil nicht verschieben. Zusätzlich: Monatsdiagramm je Messstelle und
Haushalt (`/meters/{id}/costs.json`, `/costs.json`), Kalenderjahre über
`?year=`.
```

```{spec} Erfassung der tatsächlichen Abrechnung
:id: SPEC_TAR_ACTUAL_SETTLEMENT
:status: implemented
:satisfies: REQ_TAR_011

`RecordSettlement` speichert Zeitraum und Betrag einer Jahresabrechnung,
optional mit Dokument. Die Kostenansicht zeigt für jeden abgerechneten
Zeitraum den berechneten Betrag (nur Ist, ohne Prognose), den abgerechneten
Betrag, die Differenz absolut und in Prozent sowie die Prognose, die zum
Stand von 6 Monaten vor Ende des Zeitraums berechnet worden wäre
(Prognosegüte; berechnet aus den Ablesungen bis zu diesem Stichtag, nicht
gespeichert).

Umsetzung: Erfassung auf der Vertragsseite oder über den Rechnungsimport
({need}`SPEC_AI_INVOICE`), dann mit Verweis `settlements.document_id` auf das
Dokument (Download als „Beleg“). Der Vergleich steht auf der Vertragsseite
und in der Kostenansicht.
```

### Dokumente und AI

```{spec} Rechnungsimport
:id: SPEC_AI_INVOICE
:status: implemented
:satisfies: REQ_AI_005
:links: SPEC_AI_EXTRACTION, SPEC_DOC_UPLOADS, SPEC_TAR_CONTRACTS

Upload (PDF oder Foto) → `DocumentExtractor::invoice` mit JSON-Schema:
Anbieter, Vertragsnummer, Abrechnungszeitraum, Zählernummern mit Anfangs- und
Endstand und Datum, Positionen mit Grundpreis, Intervall, Arbeitspreis und
Gültigkeit, Abschläge (Betrag, Turnus, ab), Rechnungsbetrag. Zuordnung der
Messstelle über die Zählernummer, sonst Auswahl. Das Bestätigungsformular
zeigt jeden Vorschlag einzeln abwählbar: Vertrag anlegen oder bestehenden
Vertrag um Preisstände ergänzen, Ablesungen (Quelle `invoice`, Plausibilität
wie {need}`SPEC_RDG_PLAUSIBILITY`), Abschlag, Abrechnungsbetrag
({need}`SPEC_TAR_ACTUAL_SETTLEMENT`). Speichern in einer Unit of Work, Dokument
am Abrechnungseintrag. Ohne AI-Konfiguration ist nur die Ablage des Dokuments
möglich.

Umsetzung: Die Erkennung läuft synchron im Request des Uploads; Fehler oder
fehlende AI-Konfiguration hinterlassen ein offenes Dokument mit Status
`failed`, dessen Formular manuell ausgefüllt werden kann (mehr als „nur
Ablage“), erneute Erkennung über `/documents/{id}/analyse`. Ein Stand zum
letzten Tag des Abrechnungszeitraums wird als Ablesung am Folgetag 0:00 Uhr
lokal vorgeschlagen. Für einen neuen Vertrag bilden Preiszeilen gleicher
Bezeichnung eine Position (Richtung `import`, Faktor 1); Faktor und MwSt
werden danach im Vertrag ergänzt. Vertrag, Preisstände und Abschlag
erfordern die Rolle Owner, Ablesungen und Abrechnung Member. Modell über
`ai.document_model` (sonst `ai.model`); Metriken `zaehler_documents_total`
und `zaehler_document_analysis_seconds`.
```

```{spec} Belegerkennung
:id: SPEC_AI_RECEIPT
:status: draft
:satisfies: REQ_AI_004, REQ_CAR_010
:links: SPEC_AI_EXTRACTION, SPEC_ASSET_COSTS

Foto oder PDF → `DocumentExtractor::receipt`: Datum, Betrag, Menge mit
Einheit (l, kWh), Kategorie (aus der Liste der Fahrzeugkategorien), Anbieter,
Kilometerstand, Ladeart. Vorschlag: Ladevorgang (Einheit kWh) oder
Kostenbuch-Eintrag; das Fahrzeug wird gewählt (vorbelegt, wenn der Haushalt
genau eines hat). Bestätigung speichert den Eintrag mit Verweis auf das
Dokument (`source = receipt`).
```

### Fahrzeuge und Kostenbuch

```{spec} Fahrzeuge
:id: SPEC_CAR_VEHICLES
:status: draft
:satisfies: REQ_CAR_001, REQ_CAR_007, REQ_CAR_004
:links: SPEC_CAR_DEPRECIATION

`CreateVehicle` legt in einer Unit of Work eine Messstelle der Art `vehicle`
(Einheit km, ein Register) und den Datensatz `vehicles` an (Kennzeichen,
Modell, Antriebsart). Die Antriebsart steuert die erlaubten Einträge:
Verbrenner keine Ladevorgänge und keine Kategorie `charging`, Elektro keine
Kategorie `fuel`. Nach dem ersten Kostenbuch-Eintrag oder Ladevorgang ist die
Antriebsart nicht mehr änderbar (`Conflict(DriveTypeLocked)`). Optional:
Heimlade-Zwischenzähler (Strom-Messstelle mit Zweck `wallbox` und
übergeordneter Messstelle), Verbrauchsprofile mit `valid_from`, PV-Anteil,
Anschaffung (Kaufdatum, Preis, Restwert, geplantes Ende, Verkauf).
```

```{spec} Gemeinsames Kostenbuch
:id: SPEC_ASSET_COSTS
:status: draft
:satisfies: REQ_CAR_002, REQ_PV_011
:links: ADR_020

`asset_costs` mit `asset_type` und `asset_id`; Kategorien je Anlagentyp
({need}`ADR_020`). Ein Eintrag ist einmalig oder wiederkehrend
(`recurrence_months`, optional `recurrence_until`); `assets::occurrences(entry,
period)` liefert die Buchungstage im Zeitraum, Grundlage für alle Summen.
Kraftstoff-Einträge haben Menge (l), Kilometerstand und Kennzeichen
„vollgetankt“; ein Kilometerstand kann beim Speichern zusätzlich als Ablesung
der Fahrzeug-Messstelle übernommen werden (Plausibilität wie
{need}`SPEC_RDG_PLAUSIBILITY`). Seiten: Liste je Anlage mit Filter nach Jahr
und Kategorie, Formular, Jahressumme je Kategorie.
```

```{spec} Kraftstoffverbrauch nach Vollbetankungsmethode
:id: SPEC_CAR_FUEL_CONSUMPTION
:status: draft
:satisfies: REQ_CAR_003

Zwischen zwei Vollbetankungen *i* < *j* mit Kilometerständen gilt
`l/100 km = Σ Menge(Betankungen i+1 … j) / (km_j - km_i) · 100`. Ein Abschnitt
mit einer Betankung ohne Kilometerstand oder mit `km_j ≤ km_i` wird
ausgelassen. Ausgewiesen werden der Wert je Abschnitt und der Durchschnitt je
Kalenderjahr (Summe Menge / Summe km der vollständigen Abschnitte im Jahr).
Für Elektro-Energie gilt {need}`SPEC_CAR_ENERGY_COST`.
```

```{spec} Fahrzeugauswertung
:id: SPEC_CAR_REPORT
:status: draft
:satisfies: REQ_CAR_005, REQ_CAR_006, REQ_CAR_003
:links: SPEC_CAR_COST_PER_KM, SPEC_CAR_ENERGY_COST, SPEC_FC_PERIOD_FORECAST

`VehicleReport` je Kalenderjahr und über die Haltedauer: gefahrene km (aus der
Messstelle), variable Kosten und Vollkosten je km
({need}`SPEC_CAR_COST_PER_KM`), Energie- und Ladekosten getrennt nach Heim
und extern, Verbrauch l/100 km und kWh/100 km (gemessen oder hinterlegt,
gekennzeichnet), Wertverlust, Kosten je Kategorie. Prognose für das laufende
Jahr: Fahrleistung über {need}`SPEC_FC_PERIOD_FORECAST`, erwartete
Jahreskosten = bisherige Fixkosten des Jahres + offene wiederkehrende
Fixkosten + prognostizierte km × variable Kosten je km der letzten 12 Monate.
```

### Photovoltaik

Die Fachlogik ist in {doc}`../architektur/berechnungen` spezifiziert
({need}`SPEC_PV_KPI`, {need}`SPEC_PV_BATTERY`, {need}`SPEC_PV_YIELD_EXPECTATION`,
{need}`SPEC_PV_MONITORING`, {need}`SPEC_PV_FEED_IN`, {need}`SPEC_PV_ECONOMICS`),
Zuordnung und Darstellung in {need}`SPEC_PV_ASSIGNMENT`,
{need}`SPEC_PV_BULK_CAPTURE` und {need}`SPEC_PV_CHARTS`. Umsetzung:

- `GridAnalysis(grid_meter)` lädt die Hausanschluss-Messstelle, alle Anlagen
  mit dieser Messstelle in Rolle `grid` und deren Messstellen
  (`list_for_meters`), bildet je Messstelle die Monatsserie und ruft
  `pv::grid_kpis(months, inputs)` auf. Ergebnis je Monat und je Jahr bzw.
  Abrechnungsjahr: *G*, *E*, *B*, *L*, *U*, *EV*, *V*, Quoten, Hinweise
  (Annahme ohne Einspeiseregister, negatives *EV*).
- `PvSystemReport(system)` ergänzt Erwartung, Ampel, spezifischen Ertrag,
  Vergütung, Ersparnis, Bilanz, Amortisation und Rendite; Kosten aus dem
  Kostenbuch.
- Die Detailseite der Hausanschluss-Messstelle verwendet den Gesamtverbrauch
  *V* für Vergleiche und Ampel, die Kostenansicht den Bezug *B*
  ({need}`REQ_PV_006`).
- Der Ladepreis mit PV ({need}`REQ_CAR_011`) verwendet den Autarkiegrad aus
  `GridAnalysis` und `v_ein` aus dem Einspeisevertrag.
- Sammelerfassung: `GET/POST /pv/grid/{meter}/readings` mit allen Registern;
  Fotos über die vorhandene Mehrfach-Erfassung ({need}`SPEC_AI_MULTI`) mit
  Parameter `grid={meter}`, gemeinsamer Bestätigungsseite und gemeinsamem
  Zeitpunkt.

### REST-API

```{spec} REST-API v1
:id: SPEC_API_REST
:status: draft
:satisfies: REQ_API_001, REQ_API_003
:links: ADR_023, SPEC_API_AUTH, SPEC_API_DAILY_READINGS

Ressourcen (JSON, Dezimalwerte als String, Zeitpunkte RFC 3339 in UTC):

| Methode | Pfad | Rolle | Inhalt |
|---|---|---|---|
| GET | `/api/v1/meters` | V | Messstellen mit Registern und letztem Stand |
| GET | `/api/v1/meters/{id}` | V | Messstelle mit Geräten |
| GET | `/api/v1/meters/{id}/readings?from&to` | V | Ablesungen, seitenweise (`limit` ≤ 1000, `cursor`) |
| POST | `/api/v1/meters/{id}/readings` | M | Ablesungen erfassen wie das Formular; Warnungen ohne `confirm: true` ergeben 422 mit Warnungen |
| POST | `/api/v1/readings` | M | automatische Stände: Liste aus (`register_id` oder `serial` + `register`), `read_at`, `value`; Ergebnis je Eintrag `stored`, `replaced`, `ignored`, `error` |
| GET | `/api/v1/meters/{id}/analysis` | V | Monatsserie, Vergleiche, Prognose, Ampel |
| GET | `/api/v1/meters/{id}/costs` | V | Kostenansicht ({need}`SPEC_TAR_COST_VIEW`) |
| GET | `/api/v1/contracts`, `/api/v1/contracts/{id}` | V | Verträge mit Positionen, Preisständen, Abschlägen |
| GET | `/api/v1/openapi.json` | | OpenAPI 3.1, ohne Authentifizierung |

Fehler als `application/problem+json` (RFC 9457) mit `type`, `title`,
`status`, bei Validierung `errors` je Feld (Fehlercode, nicht übersetzt).
Body-Limit 1 MiB, höchstens 1000 Stände je Anfrage. Die Versionierung erfolgt
über den Pfad; Änderungen innerhalb von v1 sind nur additiv.
```

### Export

```{spec} Export je Entität als CSV und JSON
:id: SPEC_IMP_EXPORT_CSV
:status: draft
:satisfies: REQ_IMP_002
:links: SPEC_BACKUP_HOUSEHOLD

`GET /settings/export?format=csv` liefert ein ZIP mit einer CSV-Datei je
Entität (`meters.csv`, `devices.csv`, `registers.csv`, `readings.csv`,
`contracts.csv`, `cost_positions.csv`, `price_components.csv`,
`installments.csv`, `adjustments.csv`, `settlements.csv`, `vehicles.csv`,
`asset_costs.csv`, `charging_sessions.csv`, `pv_systems.csv`,
`pv_system_meters.csv`, `reading_revisions.csv`): UTF-8 mit BOM, Komma als
Trenner, Dezimalpunkt, ISO-8601, Kopfzeile mit Spaltennamen, IDs als UUID.
`format=json` ist das vorhandene Haushaltsarchiv
({need}`SPEC_BACKUP_HOUSEHOLD`). Fotos und Dokumente sind nur im
Haushaltsarchiv enthalten.
```

### Betrieb und Qualität

```{spec} Datensicherung mit CNPG und SQLite
:id: SPEC_OPS_BACKUP_CNPG
:status: draft
:satisfies: REQ_OPS_011

`database.cnpg.backup` wird unverändert als `spec.backup` übernommen
(vorhanden); ergänzt um `ScheduledBackup` (`database.cnpg.scheduledBackup`,
Standard täglich) und ein Beispiel für Barman Cloud gegen S3 in den Values.
{doc}`../betrieb/backup` beschreibt Wiederherstellung per
`bootstrap.recovery` in einen neuen Cluster und für SQLite `VACUUM INTO`
sowie Litestream; beide Wege sind einmal in einem Test-Cluster bzw. lokal
durchgespielt und das Ergebnis dokumentiert.
```

```{spec} Barrierefreiheit
:id: SPEC_A11Y
:status: draft
:satisfies: REQ_NFR_007

Prüfung aller Seiten in `de` mit axe-core (Regelsatz WCAG 2.2 AA) über
Playwright gegen die compose-Umgebung mit Seed-Daten; der CI-Job schlägt bei
Verstößen der Stufen `serious` und `critical` fehl. Manuell: Bedienung aller
Abläufe nur mit Tastatur, sichtbarer Fokus, Bildschirmleser-Probe (Orca) für
Erfassung und Auswertung. Diagramme haben eine Tabellenalternative
({need}`SPEC_ANA_CHARTS`), Ampeln zusätzlich Text.
```

## HTTP-Schnittstelle: neue Routen

| Methode | Pfad | Rolle | Spezifikation |
|---|---|---|---|
| GET, POST | `/settings/members`, `/settings/members/{user}/role`, `/settings/members/{user}/remove`, `/settings/leave` | V/O | {need}`SPEC_TEN_ROLES` |
| GET, POST | `/settings/invitations`, `/settings/invitations/{id}/revoke` | O | {need}`SPEC_TEN_INVITES` |
| GET, POST | `/invite/{token}`, `/invite/{token}/accept` | angemeldet | {need}`SPEC_TEN_INVITES` |
| GET, POST | `/readings/{id}/edit` | M | {need}`SPEC_RDG_HISTORY` |
| GET | `/meters/{id}/history` | V | {need}`SPEC_RDG_HISTORY` |
| POST | `/meters/{id}/archive`, `/meters/{id}/restore` | O | {need}`SPEC_MTR_ARCHIVE` |
| GET, POST | `/meters/{id}/contracts/new`, `/contracts/{id}`, `/contracts/{id}/edit`, `/contracts/{id}/delete` | V/O | {need}`SPEC_TAR_CONTRACTS` |
| POST | `/contracts/{id}/installments`, `/contracts/{id}/adjustments`, `/contracts/{id}/settlements` | O/M | {need}`SPEC_TAR_COST_VIEW`, {need}`SPEC_TAR_ACTUAL_SETTLEMENT` |
| GET | `/meters/{id}/costs`, `/costs` | V | {need}`SPEC_TAR_COST_VIEW` |
| GET, POST | `/documents`, `/documents/{id}`, `/documents/{id}/confirm`, `/documents/{id}/discard` | M | {need}`SPEC_AI_INVOICE`, {need}`SPEC_AI_RECEIPT` |
| GET | `/documents/{id}/file` | V | {need}`SPEC_DOC_UPLOADS` |
| GET, POST | `/vehicles`, `/vehicles/new`, `/vehicles/{id}`, `/vehicles/{id}/edit`, `/vehicles/{id}/charging` | V/O/M | {need}`SPEC_CAR_VEHICLES`, {need}`SPEC_CAR_REPORT` |
| GET, POST | `/assets/{type}/{id}/costs`, `/asset-costs/{id}/edit`, `/asset-costs/{id}/delete` | V/M | {need}`SPEC_ASSET_COSTS` |
| GET, POST | `/pv`, `/pv/new`, `/pv/{id}`, `/pv/{id}/edit`, `/pv/grid/{meter}`, `/pv/grid/{meter}/readings`, `/pv/grid/{meter}/chart.json` | V/O/M | {need}`SPEC_PV_ASSIGNMENT`, {need}`SPEC_PV_BULK_CAPTURE`, {need}`SPEC_PV_CHARTS` |
| GET, POST | `/settings/notifications` | V | {need}`SPEC_NTF_EMAIL` |
| GET, POST | `/settings/tokens`, `/settings/tokens/{id}/revoke` | V | {need}`SPEC_API_AUTH` |
| GET | `/settings/export?format=csv` | O | {need}`SPEC_IMP_EXPORT_CSV` |
| * | `/api/v1/*` | Token | {need}`SPEC_API_REST` |

Multipart ist zusätzlich für `POST /documents` zugelassen
({need}`SPEC_SEC_SESSION_CSRF`). Die Middleware für `/api/v1` lässt Sitzung
und CSRF-Prüfung aus und setzt keine HTML-Sicherheitsheader außer `nosniff`.

## Oberfläche

| Template | Inhalt |
|---|---|
| `pages/settings/{members,invitations,notifications,tokens}.html` | Mitglieder, Einladungen, Benachrichtigungen, API-Tokens |
| `pages/invite.html` | Einladung annehmen |
| `pages/readings/{edit,history}.html` | Korrektur, Änderungsverlauf |
| `pages/contracts/{form,detail}.html` | Vertrag mit Positionen und Preisständen (HTMX: Zeilen hinzufügen) |
| `pages/costs/{meter,household}.html` | Kostenansicht mit Tabelle und Diagramm |
| `pages/documents/{upload,invoice,receipt}.html` | Dokument hochladen, Rechnung bzw. Beleg bestätigen |
| `pages/vehicles/{list,form,detail,charging}.html` | Fahrzeuge |
| `pages/assets/{list,form}.html` | Kostenbuch |
| `pages/pv/{list,form,grid,bulk}.html` | PV-Anlagen, Hausanschluss, Sammelerfassung |
| `partials/traffic_light.html` | Ampel mit Text |

Navigation: „Kosten“, „Fahrzeuge“ und „PV“ erscheinen nur, wenn der Haushalt
Verträge, Fahrzeuge bzw. PV-Anlagen hat; sonst über „Messstellen“ und
„Einstellungen“ erreichbar. Diagramme verwenden `charts.js` mit neuen Typen
(gestapelte Balken, Linie mit Band, kumulierte Bilanz).

## Konfiguration: Erweiterungen

| Schlüssel | Default | Zweck |
|---|---|---|
| `smtp.host`, `smtp.port`, `smtp.security`, `smtp.username`, `smtp.password`, `smtp.from` | leer, `587`, `starttls` | {need}`SPEC_NTF_EMAIL` |
| `scheduler.tick_seconds` | `60` | {need}`SPEC_JOB_SCHEDULER` |
| `database.lock_url` | `database.url` | {need}`SPEC_OPS_REPLICAS` |
| `upload.max_document_bytes` | `10485760` | {need}`SPEC_DOC_UPLOADS` |
| `invitations.ttl_days` | `7` | {need}`SPEC_TEN_INVITES` |
| `api.enabled` | `true` | {need}`SPEC_API_REST` |
| `ai.document_model` | `ai.model` | Modell für Rechnungen und Belege |

Neue Abhängigkeiten:

| Crate | Abhängigkeiten |
|---|---|
| `zaehler-notify` | `lettre` 0.11 (Features `tokio1-rustls-tls`, `builder`, `smtp-transport`), `fluent-templates` 0.15 |
| `zaehler-web` | `utoipa` 5, `utoipa-axum` 0.2 |
| `zaehler-app` | `sha2` 0.10 (Token-Hash) |
| Tests | Mailpit (compose, CI-Service), Playwright mit `@axe-core/playwright` (Node, nur CI) |

## Observability

| Metrik | Labels | Quelle |
|---|---|---|
| `zaehler_job_runs_total` | job, outcome | Scheduler |
| `zaehler_leader` | | Scheduler (Gauge 0/1) |
| `zaehler_outbox_pending` | | `DispatchOutbox` (Gauge) |
| `zaehler_notifications_total` | kind, outcome | `DispatchOutbox`, `SendReminders` |
| `zaehler_api_readings_total` | outcome (`stored`, `replaced`, `ignored`, `error`) | `IngestReadings` |
| `zaehler_api_auth_failures_total` | reason | `ResolveApiTenant` |
| `zaehler_documents_total` | kind, outcome | `AnalyseDocument` |

## Testdesign

| Ebene | Gegenstand | Werkzeug |
|---|---|---|
| Domain | Kostenrechnung mit Golden-Tests aus realen Abrechnungen (Strom mit Preiswechsel, Gas mit Brennwert, Wasser mit drei Positionen, Staffel, Bonus), Summe der Teilzeiträume = Gesamtzeitraum (Property-Test), Saisonprofil, Fälligkeit, Vollbetankung, PV-Kennzahlen inkl. Grenzfälle aus {need}`REQ_PV_004` | Unit-Tests, `proptest` |
| Ports | Port-Testsuite erweitert: Tenant-Isolation aller neuen Repositories, Einladung nur einmal einlösbar (parallel), letzter Owner, Vertragsüberlappung, `upsert_daily` | `tokio::test` gegen In-Memory, SQLite, PostgreSQL |
| Anwendungsfälle | Rollenmatrix: jeder schreibende Anwendungsfall mit `Viewer` ergibt `Forbidden`; Revisionen; Outbox-Eintrag nur bei Erfassung, nicht bei Import | `tokio::test` |
| Jobs | Leader-Lock mit zwei Instanzen; Erinnerung einmal je Tag; Alarm einmal je Ablesung; Wiederholung nach SMTP-Fehler | `tokio::test`, PostgreSQL in der CI, Mailpit |
| AI-Adapter | Rechnung und Beleg: Erfolg, ungültiges JSON, Timeout | `wiremock` |
| HTTP | neue Routen, CSRF, 404 bei fremden IDs, API ohne Token 401, mit Viewer-Token schreibend 403, Cookie ohne Bearer unter `/api/v1` 401, OpenAPI valide | `tower::ServiceExt::oneshot`, OpenAPI-Validierung |
| Upgrade | jede v1-Migration im Upgrade-Test, Export und Import mit allen neuen Tabellen | vorhandener Test |
| Barrierefreiheit | {need}`SPEC_A11Y` | Playwright, axe-core |
| Leistung | {need}`SPEC_PERF_BUDGET` erweitert: Seed mit Verträgen (2 Preiswechsel je Jahr), 2 Fahrzeugen mit 10 Jahren Kostenbuch, 1 PV-Anlage mit Speicher; Kostenansicht, Fahrzeugauswertung, Hausanschluss-Auswertung p95 < 300 ms | Release-Test |

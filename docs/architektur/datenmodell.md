# Datenmodell

```{spec} Kernschema Haushalt, Zähler, Ablesung
:id: SPEC_DM_CORE
:status: implemented
:satisfies: REQ_TEN_001, REQ_MTR_001, REQ_MTR_003, REQ_MTR_005, REQ_RDG_001

Das Kernschema (Migration `m20261015_000001_core`) besteht aus den Tabellen
`households`, `users`, `memberships`, `meters`, `meter_devices`, `registers`
und `readings`. Primärschlüssel sind UUIDv7, Zeitpunkte `timestamptz` (UTC),
Mengen Festkomma-Integer ({need}`SPEC_STORE_FIXED_POINT`).
```

```{mermaid}
erDiagram
  households ||--o{ memberships : hat
  users ||--o{ memberships : hat
  households ||--o{ meters : besitzt
  meters ||--o{ meter_devices : "Gerät (Wechsel)"
  meters ||--o{ registers : "HT/NT/export"
  registers ||--o{ readings : ""
  meter_devices ||--o{ readings : ""

  households {
    uuid id PK
    string name
    string timezone
  }
  users {
    uuid id PK
    string oidc_issuer
    string oidc_subject
    string locale
  }
  memberships {
    uuid household_id PK
    uuid user_id PK
    string role
  }
  meters {
    uuid id PK
    uuid household_id FK
    string kind
    string unit
    int reading_interval_days
    bigint max_daily_rate_milli
    bool archived
  }
  meter_devices {
    uuid id PK
    uuid meter_id FK
    string serial_number
    timestamptz installed_at
    timestamptz removed_at
    smallint rollover_digits
  }
  registers {
    uuid id PK
    uuid meter_id FK
    string code
    string direction
  }
  readings {
    uuid id PK
    uuid register_id FK
    uuid device_id FK
    timestamptz read_at
    bigint value_milli
    string source
    string photo_key
    double ai_confidence
  }
```

## Geplante Erweiterungen (v1)

```{mermaid}
erDiagram
  meters ||--o{ contracts : ""
  contracts ||--o{ cost_positions : ""
  cost_positions ||--o{ price_components : "gültig ab"
  contracts ||--o{ installments : "Abschläge"
  contracts ||--o{ adjustments : "Bonus, Deckel"
  contracts ||--o{ settlements : "tatsächliche Abrechnung"
  meters ||--o{ meters : "parent_meter_id (Zwischenzähler)"
  meters ||--o| vehicles : "nur Fahrzeug"
  vehicles ||--o| meters : "home_charging_meter_id"
  vehicles ||--o{ vehicle_consumption_profiles : "gültig ab"
  vehicles ||--o{ charging_sessions : "extern"
  households ||--o{ pv_systems : ""
  pv_systems ||--o{ pv_system_meters : "Rolle grid, generation, battery"
  pv_system_meters }o--|| meters : ""
  pv_systems ||--o{ asset_costs : "asset_type pv_system"
  vehicles ||--o{ asset_costs : "asset_type vehicle"
  meters ||--o| vehicle_acquisitions : "nur Fahrzeug"
  users ||--o| notification_settings : ""
  households ||--o{ season_profiles : "je Zähler"
```

| Tabelle | Inhalt |
|---|---|
| `contracts` | Anbieter, Vertragsnummer, Beginn/Ende, Start des Abrechnungsjahres |
| `cost_positions` | Name, Umrechnungsfaktor (micro), Bezug auf Register oder Messstelle |
| `price_components` | `valid_from`, Grundpreis (micro EUR) + Intervall, Arbeitspreis als JSON (`flat`/`tiered`/`dynamic`) |
| `installments` | Betrag, Turnus in Monaten, `valid_from` |
| `adjustments` | Bonus/Gutschrift oder Preisdeckel |
| `settlements` | Betrag der tatsächlichen Abrechnung je Abrechnungszeitraum |
| `meters.parent_meter_id` | Verweis eines Zwischenzählers auf die übergeordnete Messstelle (nullable) |
| `vehicles` | 1:1 zur Fahrzeug-Messstelle: Kennzeichen, Modell, Antriebsart (`combustion`, `electric`, `plugin_hybrid`), optional `home_charging_meter_id` |
| `vehicle_consumption_profiles` | `valid_from`, Durchschnittsverbrauch kWh/100 km (milli), Ladeverlust %, elektrischer Fahranteil % (Plug-in-Hybrid) |
| `charging_sessions` | Externes Laden: Datum, kWh (milli), Betrag (micro EUR), Kilometerstand, Anbieter, `ac`/`dc`, Quelle (Formular, Beleg), Belegfoto |
| `asset_costs` | Gemeinsames Kostenbuch ({need}`ADR_020`): `asset_type` (`vehicle`, `pv_system`), `asset_id`, Datum, Kategorie, Betrag, Menge, Wiederholung, Belegfoto |
| `pv_systems` | Name, Typ (`surplus`, `full_feed_in`, `plug_in`), kWp (milli), Inbetriebnahme, Azimut, Neigung, Wechselrichter, Speicherkapazität kWh (milli), spezifischer Jahresertrag, Degradation, Monatsanteile (optional), Alarmschwelle |
| `pv_system_meters` | `pv_system_id`, `meter_id`, Rolle (`grid`, `generation`, `battery`); eindeutig für `generation` und `battery` je Messstelle |
| `vehicles.pv_share_override` | optionaler PV-Anteil am Heimladen in % ({need}`REQ_CAR_011`) |
| `readings.api_token_id` | Herkunft automatisch gelieferter Stände ({need}`SPEC_API_DAILY_READINGS`); `readings.source` erhält den Wert `api` |
| `vehicle_acquisitions` | Kaufdatum, Preis, Restwert, geplantes Ende |
| `notification_settings` | E-Mail-Adresse, Ereignistypen je Nutzer |
| `job_runs` | Letzte Ausführung je Hintergrundjob (Scheduler) |

## Photovoltaik

```{spec} Zuordnung von Messstellen zu PV-Anlagen
:id: SPEC_PV_ASSIGNMENT
:status: draft
:satisfies: REQ_PV_001, REQ_PV_002
:links: ADR_021

Eine PV-Anlage (`pv_systems`) referenziert über `pv_system_meters` genau eine
Messstelle mit Rolle `grid` und mindestens eine mit Rolle `generation`;
Rolle `battery` ist optional. Messstellen mit Rolle `generation` oder
`battery` gehören zu genau einer Anlage, eine Messstelle mit Rolle `grid` kann
zu mehreren Anlagen gehören (gemeinsamer Hausanschluss). Alle zugeordneten
Messstellen haben die Art Strom und gehören zum selben Haushalt. Eine
Messstelle mit Rolle `battery` hat genau die Register `charge` (Bezug) und
`discharge` (Einspeisung). Anlagen vom Typ `surplus` oder `full_feed_in`
setzen am Hausanschluss ein Register mit Richtung Einspeisung voraus.
```

# Querschnittliche Konzepte

## Persistenz

```{spec} Festkomma-Speicherung
:id: SPEC_STORE_FIXED_POINT
:status: implemented
:satisfies: REQ_NFR_005, REQ_OPS_002

Dezimalwerte werden als `BIGINT` gespeichert: Mengen mit Skala 10⁻³
(`*_milli`), Geldbeträge und Preise mit Skala 10⁻⁶ (`*_micro`). Werte mit mehr
Nachkommastellen werden abgewiesen, nicht gerundet.
```

```{spec} Datenbank-Backends
:id: SPEC_OPS_DB_BACKENDS
:status: implemented
:satisfies: REQ_OPS_002

Das Backend ergibt sich aus dem URL-Schema (`postgres://`, `sqlite://`).
SQLite wird mit `journal_mode=WAL` (persistent in der Datei) sowie je
Verbindung `foreign_keys=ON` und `busy_timeout=5s` (sqlx-Defaults) betrieben;
das Verzeichnis der Datenbankdatei wird bei Bedarf angelegt. Es werden nur
Konstrukte verwendet, die beide Backends unterstützen (keine DB-spezifischen
Typen in Migrationen).
```

```{spec} Migrationen
:id: SPEC_OPS_MIGRATIONS
:status: implemented
:satisfies: REQ_OPS_009, REQ_OPS_008

Rust-Migrationen (`sea-orm-migration`), ausgeführt beim Start
(`database.auto_migrate`) oder per `zaehler migrate`. Bei PostgreSQL läuft die
Migration in einer Transaktion unter `pg_advisory_xact_lock`, damit parallel
startende Replicas nicht kollidieren.
```

```{spec} Upgrade ohne Datenverlust
:id: SPEC_OPS_UPGRADE
:status: implemented
:satisfies: REQ_OPS_012, REQ_OPS_009
:links: SPEC_OPS_MIGRATIONS, SPEC_BACKUP_DATA

Vor jeder Migration vergleicht `zaehler_db::schema_status` die angewendeten
Migrationen (`seaql_migrations`) mit denen der Programmversion. Unbekannte
angewendete Migrationen bedeuten eine Datenbank einer neueren Version: Start
und `zaehler migrate` brechen ohne Änderung ab. Liegen ausstehende
Migrationen vor und ist die Datenbank nicht leer, schreibt der Server mit
`backup.before_migrate` (Standard an) ein vollständiges Archiv
({need}`SPEC_BACKUP_INSTALLATION`) nach `backup.dir`, behält die neuesten
`backup.keep` und migriert erst danach; ohne `database.auto_migrate` verweigert
der Webserver den Start bei ausstehenden Migrationen. Der Export liest dafür
das noch nicht migrierte Schema ({need}`SPEC_BACKUP_DATA`).

Regeln für Migrationen: nur anhängen; freigegebene Migrationen sind mit
SHA-256 ihres Quelltexts in `zaehler-db/src/upgrade_tests.rs` festgehalten und
dürfen nicht geändert werden; Spalten werden nur ergänzt (mit Vorgabe oder
`NULL`), Umbenennen oder Entfernen erfordert eine Migration, die die Daten
überführt, und eine Anpassung des Exports. Ein Test spielt Daten im Schema
der ersten Migration ein, wendet jede weitere Migration einzeln an und
vergleicht nach jedem Schritt den Export mit den Ausgangsdaten.
```

## Sicherheit

```{spec} Nur parametrisierte Datenbankabfragen
:id: SPEC_SEC_SQL_PARAMS
:status: implemented
:satisfies: REQ_NFR_003

Werte gelangen ausschließlich als Bind-Parameter in SQL: über die
Query-Builder von SeaORM/sea-query oder `Statement::from_sql_and_values` mit
Platzhaltern (`$1` bzw. `?`). APIs, die SQL als Text ohne Parameter annehmen
(`ConnectionTrait::execute_unprepared`, `Statement::from_string`,
`Expr::cust`), sind per `clippy.toml` (`disallowed-methods`) verboten; die CI
baut mit `-D warnings`. Eine Ausnahme erfordert ein lokales
`#[allow(clippy::disallowed_methods)]` mit Begründung im Code-Review.
Bezeichner (Tabellen, Spalten) werden nie aus Eingaben gebildet.
```

## Konfiguration

```{spec} Konfigurationsquellen
:id: SPEC_OPS_CONFIG
:status: implemented
:satisfies: REQ_OPS_003

Reihenfolge (spätere überschreiben frühere): eingebaute Defaults →
TOML-Datei (`--config` / `ZAEHLER_CONFIG`) → Umgebungsvariablen `ZAEHLER_*`
mit `__` als Trenner (`ZAEHLER_DATABASE__URL`), ergänzt um eine optionale
`.env` im Arbeitsverzeichnis (gesetzte Variablen haben Vorrang). Geheimnisse sind nicht
serialisierbar; `zaehler config` gibt die wirksame Konfiguration ohne
Geheimnisse und mit maskierten Zugangsdaten in URLs aus.
```

## Observability

```{spec} Metriken
:id: SPEC_OPS_METRICS
:status: implemented
:satisfies: REQ_OPS_004

Prometheus-Format auf separatem Listener (`server.metrics_listen`, Standard
`:9090`, nicht über Ingress). Basis-Metriken: `http_requests_total` und
`http_request_duration_seconds` mit Labels `method`, `path` (Route-Template,
keine IDs) und `status`.
```

```{spec} Health-Endpunkte
:id: SPEC_OPS_HEALTH
:status: implemented
:satisfies: REQ_OPS_005

`GET /healthz` → 200, sobald der Server läuft. `GET /readyz` → 200 bei
erfolgreichem DB-Ping, sonst 503.
```

## Auslieferung

```{spec} Container-Image
:id: SPEC_OPS_IMAGE
:status: implemented
:satisfies: REQ_OPS_001, REQ_NFR_003

Mehrstufiger Build (`rust:1.98-trixie` → `gcr.io/distroless/cc-debian13:nonroot`),
Binary `/usr/local/bin/zaehler`, UID/GID 65532, Arbeitsverzeichnis
`/var/lib/zaehler` (SQLite-Default und Dateiablage unter `data/`), Ports 8080
(HTTP) und 9090 (Metriken). Templates und Assets sind ins Binary eingebettet,
das Root-Dateisystem kann read-only sein.
```

```{spec} Helm-Chart
:id: SPEC_OPS_HELM
:status: implemented
:satisfies: REQ_OPS_006, REQ_OPS_008, REQ_OPS_010

`deploy/helm/zaehler`: `database.type` = `cnpg` (legt `postgresql.cnpg.io/v1
Cluster` an, URL aus Secret `<cluster>-app/uri`), `existing` oder `sqlite`
(1 Replica, `Recreate`, PVC). `storage.kind` = `s3` oder `fs`. Das Chart
verweigert Konfigurationen, die mit mehreren Replicas nicht funktionieren
(`fs`-Storage ohne RWX). Probes auf `/healthz` und `/readyz`, optional Ingress
und ServiceMonitor; Geheimnisse nur über `existingSecret`-Referenzen.
```

```{spec} docker compose
:id: SPEC_OPS_COMPOSE
:status: implemented
:satisfies: REQ_OPS_007, REQ_OPS_010

`compose.yaml`: App mit SQLite und Dateiablage im Volume `zaehler-data`,
optionale `.env`.
`compose.postgres.yaml`: Override mit `postgres:18` und Healthcheck.
```

## Dokumentation

```{spec} Code-Tracing für die Dokumentation
:id: SPEC_DOC_TRACING
:status: implemented
:satisfies: REQ_NFR_008

`docs/_ext/codetrace.py` liest beim Sphinx-Start `// @impl`/`// @test`
(Rust) bzw. `# @impl`/`# @test` (YAML, Dockerfile, Python) und erzeugt
`nachverfolgung/code.md` mit `impl`/`test`-Needs, verlinkt über `implements`
bzw. `verifies`. Tote Links sind Warnungen; die CI baut mit `-W`.
```

## Oberfläche

- Server-seitiges Rendering mit Askama, Interaktion mit HTMX ({need}`ADR_002`).
- Diagramme mit Apache ECharts (eingebettet), Daten als JSON-Endpunkte.
- i18n mit Fluent-Dateien je Sprache ({need}`ADR_015`).
- PWA-Manifest; Kamera über `<input type="file" accept="image/*" capture="environment">`.

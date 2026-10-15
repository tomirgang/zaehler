# ADR-003: SeaORM mit PostgreSQL (CNPG) und SQLite

```{adr} SeaORM für PostgreSQL/CNPG und SQLite
:id: ADR_003
:status: accepted
:decides: REQ_OPS_002, REQ_OPS_009, SPEC_OPS_DB_BACKENDS, SPEC_OPS_MIGRATIONS

**Datum:** 2026-10-15
```

## Kontext
In Kubernetes stellt CloudNativePG PostgreSQL mit HA und Backups bereit. Für
docker compose und Einzelinstallationen benötigt SQLite keinen separaten Dienst. Beide
müssen mit einem Schema und einer Codebasis funktionieren.

## Entscheidung
**SeaORM 2** mit `sqlx-postgres` und `sqlx-sqlite`; Migrationen in Rust
(`sea-orm-migration`), damit eine Migrationsquelle für beide Backends genügt.
Auswahl ausschließlich über die Datenbank-URL. CNPG-Verbindungsdaten kommen
aus dem Secret `<cluster>-app` (Key `uri`).

## Alternativen
- *sqlx* direkt: compile-time-geprüfte Queries, aber zwei Migrationssätze und
  unterschiedliche SQL-Dialekte (Platzhalter, Typen).
- *Diesel*: synchron bzw. `diesel-async` ohne SQLite-Unterstützung.

## Konsequenzen
- Nur Features beider Backends (kein `jsonb`-Index, keine Arrays).
- Komplexe Auswertungen laufen in Rust (`zaehler-domain`), nicht in SQL.
- CI testet Migrationen gegen beide Backends.

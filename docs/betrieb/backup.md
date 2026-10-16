# Datensicherung, Umzug und Upgrade

Siehe {need}`REQ_OPS_011`, {need}`REQ_IMP_004` und {need}`REQ_OPS_012`.

## Export und Import (alle Backends)

Ein Archiv enthält alle Daten einer Installation einschließlich der Fotos
({need}`SPEC_BACKUP_FORMAT`). Es ist unabhängig vom Datenbank-Backend und
eignet sich für Umzüge, z.B. von SQLite nach PostgreSQL.

```bash
# Archiv schreiben (vorhandene Datei wird nicht überschrieben)
zaehler export /backup/zaehler-$(date +%F).zip

# in eine neue, leere Installation einspielen (migriert vorher die Datenbank)
zaehler import /backup/zaehler-2026-10-16.zip
```

Der Import ist nur in eine leere Installation möglich (keine Nutzer, keine
Haushalte); IDs, Zeitstempel, Fotos und Anmeldungen (OIDC-Issuer und
Subject) bleiben erhalten. Sitzungen und temporäre Dateien des CSV-Imports
gehören nicht zum Archiv; nach dem Import melden sich alle Nutzer neu an.

In docker compose:

```bash
docker compose exec zaehler zaehler export /var/lib/zaehler/data/backups/manuell.zip
```

Einzelne Haushalte exportieren und importieren Nutzer unter „Einstellungen“,
Abschnitt „Daten sichern und übertragen“ ({need}`SPEC_BACKUP_HOUSEHOLD`).

## Sicherung vor Migrationen

Liegen beim Start oder bei `zaehler migrate` ausstehende Migrationen vor,
schreibt der Server zuerst ein vollständiges Archiv nach `backup.dir`
(`zaehler-before-migration-<Zeitpunkt>-<Migration>.zip`) und behält die
neuesten `backup.keep` Archive. Schlägt die Sicherung fehl, wird nicht
migriert. Wiederherstellung: Archiv mit der vorherigen oder einer neueren
Programmversion per `zaehler import` in eine leere Installation einspielen.

| Umgebung | Verzeichnis | Hinweis |
|---|---|---|
| lokal | `data/backups` | Standard |
| docker compose | `/var/lib/zaehler/data/backups` | Volume `zaehler-data` |
| Helm mit Daten-Volume (`sqlite` oder `storage.kind=fs`) | `/var/lib/zaehler/data/backups` | `backup.beforeMigrate`, `backup.keep` |
| Helm ohne Daten-Volume (CNPG und S3) | abgeschaltet | CNPG-Backups; vor Upgrades `zaehler export` |

Die Archive enthalten alle Fotos und belegen entsprechend Platz.

## Upgrade

Jede neuere Version aktualisiert die Datenbank jeder älteren Version
automatisch ({need}`SPEC_OPS_UPGRADE`). Ein Downgrade ist nicht möglich: Eine
ältere Version startet nicht gegen eine Datenbank, die Migrationen einer
neueren Version enthält.

## PostgreSQL / CNPG

CNPG sichert per Barman Cloud (Plugin) in einen Objektspeicher inkl. WAL-Archiv
(Point-in-Time-Recovery). Die Konfiguration erfolgt am `Cluster`-Objekt; das
Chart wird dafür Werte bereitstellen (offen). Wiederherstellung über einen neuen
Cluster mit `bootstrap.recovery`.

## SQLite

- Konsistente Kopie im laufenden Betrieb:
  `sqlite3 /var/lib/zaehler/data/zaehler.db "VACUUM INTO '/backup/zaehler-$(date +%F).db'"`
- Kontinuierlich: [Litestream](https://litestream.io/) als Sidecar repliziert in S3.

## Fotos und Dokumente

- `storage.kind=fs`: Verzeichnis `data/files` mitsichern (gleiches Volume wie SQLite).
- `storage.kind=s3`: Versionierung/Lifecycle des Buckets nutzen.

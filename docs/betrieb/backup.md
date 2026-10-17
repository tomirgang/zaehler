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

CNPG sichert per Barman Cloud in einen Objektspeicher inkl. WAL-Archiv
(Point-in-Time-Recovery). Das Chart übernimmt `database.cnpg.backup`
unverändert als `spec.backup` des `Cluster`-Objekts;
`database.cnpg.scheduledBackup.enabled=true` legt zusätzlich ein
`ScheduledBackup` an (Standard täglich 02:00 UTC, `schedule` im
Cron-Format von CNPG mit Sekunden). Beispiel:

```yaml
database:
  cnpg:
    backup:
      retentionPolicy: 30d
      barmanObjectStore:
        destinationPath: s3://backup/zaehler
        endpointURL: https://s3.example.com
        s3Credentials:
          accessKeyId: { name: zaehler-backup-s3, key: ACCESS_KEY_ID }
          secretAccessKey: { name: zaehler-backup-s3, key: ACCESS_SECRET_KEY }
    scheduledBackup:
      enabled: true
```

Wiederherstellung in einen neuen Cluster (gleiche CNPG-Version, gleiche
Zugangsdaten zum Objektspeicher):

```yaml
apiVersion: postgresql.cnpg.io/v1
kind: Cluster
metadata:
  name: zaehler-db-restore
spec:
  instances: 2
  storage: { size: 5Gi }
  bootstrap:
    recovery:
      source: zaehler-db
      # optional Point-in-Time: recoveryTarget: { targetTime: "2026-10-17 02:00:00+00" }
  externalClusters:
    - name: zaehler-db
      barmanObjectStore:
        destinationPath: s3://backup/zaehler
        endpointURL: https://s3.example.com
        s3Credentials:
          accessKeyId: { name: zaehler-backup-s3, key: ACCESS_KEY_ID }
          secretAccessKey: { name: zaehler-backup-s3, key: ACCESS_SECRET_KEY }
```

Danach die Anwendung mit `database.type=existing` auf das Secret
`zaehler-db-restore-app` (Key `uri`) zeigen lassen und zur Kontrolle
`zaehler export` vor und nach der Wiederherstellung vergleichen (Prüfsumme
`data_sha256` im Manifest). Der Ablauf ist in einem Test-Cluster noch nicht
durchgespielt. Neuere CNPG-Versionen sichern über das
Barman-Cloud-Plugin; dessen Konfiguration (`plugins`) ist nicht Teil des
Charts.

## SQLite

- Konsistente Kopie im laufenden Betrieb:
  `sqlite3 /var/lib/zaehler/data/zaehler.db "VACUUM INTO '/backup/zaehler-$(date +%F).db'"`
- Kontinuierlich: [Litestream](https://litestream.io/) als Sidecar repliziert in S3.

Wiederherstellung: Server stoppen, Kopie als `zaehler.db` ablegen (ohne
alte `-wal`- und `-shm`-Dateien), Verzeichnis `files` aus derselben
Sicherung zurückspielen, Server starten.

Probe (2026-10-17): Kopie per `VACUUM INTO` aus einer laufenden Instanz,
danach `zaehler export` gegen Original und Kopie. Beide Archive enthalten
3 Haushalte, 12 Messstellen und 267 Ablesungen; `data.json` ist
byte-identisch (gleicher SHA-256). Litestream ist nicht durchgespielt.

## Fotos und Dokumente

- `storage.kind=fs`: Verzeichnis `data/files` mitsichern (gleiches Volume wie SQLite).
- `storage.kind=s3`: Versionierung/Lifecycle des Buckets nutzen.

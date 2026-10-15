# Datensicherung

Siehe {need}`REQ_OPS_011`.

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

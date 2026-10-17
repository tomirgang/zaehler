# Konfiguration

Quellen in aufsteigender Priorität ({need}`SPEC_OPS_CONFIG`):

1. eingebaute Defaults
2. TOML-Datei (`zaehler --config pfad.toml` oder `ZAEHLER_CONFIG`), Beispiel:
   `config/zaehler.example.toml`
3. Umgebungsvariablen `ZAEHLER_<SEKTION>__<SCHLÜSSEL>`; eine `.env` im
   Arbeitsverzeichnis wird zusätzlich eingelesen, bereits gesetzte Variablen
   haben Vorrang (Vorlage: `.env.example`)

`zaehler config` gibt die wirksame Konfiguration ohne Geheimnisse aus.

Der Server startet nur mit Anmeldekonfiguration: entweder `oidc` oder
`auth.dev_login = true` (nur mit `server.public_url` auf `localhost`); beides
zugleich ist unzulässig.

| Schlüssel | Umgebungsvariable | Default |
|---|---|---|
| `server.listen` | `ZAEHLER_SERVER__LISTEN` | `0.0.0.0:8080` |
| `server.metrics_listen` | `ZAEHLER_SERVER__METRICS_LISTEN` | `0.0.0.0:9090` |
| `server.public_url` | `ZAEHLER_SERVER__PUBLIC_URL` | `http://localhost:8080` |
| `database.url` | `ZAEHLER_DATABASE__URL` | `sqlite://data/zaehler.db?mode=rwc` |
| `database.max_connections` | `ZAEHLER_DATABASE__MAX_CONNECTIONS` | `10` |
| `database.auto_migrate` | `ZAEHLER_DATABASE__AUTO_MIGRATE` | `true` |
| `database.lock_url` | `ZAEHLER_DATABASE__LOCK_URL` | `database.url`; direkte Verbindung für den Leader-Lock, nicht über einen Pooler im Transaktionsmodus |
| `scheduler.tick_seconds` | `ZAEHLER_SCHEDULER__TICK_SECONDS` | `60` |
| `invitations.ttl_days` | `ZAEHLER_INVITATIONS__TTL_DAYS` | `7` |
| `log.format` | `ZAEHLER_LOG__FORMAT` | `json` (`pretty`) |
| `log.filter` | `ZAEHLER_LOG__FILTER` / `RUST_LOG` | `info` |
| `storage.kind` | `ZAEHLER_STORAGE__KIND` | `fs` (`s3`) |
| `storage.path` | `ZAEHLER_STORAGE__PATH` | `data/files` |
| `storage.bucket`, `endpoint`, `region` | `ZAEHLER_STORAGE__…` | kein Default |
| `storage.access_key_id`, `secret_access_key` | `ZAEHLER_STORAGE__…` | aus `AWS_ACCESS_KEY_ID`, `AWS_SECRET_ACCESS_KEY` |
| `storage.allow_http` | `ZAEHLER_STORAGE__ALLOW_HTTP` | `false` |
| `image.max_pixels` | `ZAEHLER_IMAGE__MAX_PIXELS` | `2000000` |
| `image.jpeg_quality` | `ZAEHLER_IMAGE__JPEG_QUALITY` | `85` (1 bis 100) |
| `housekeeping.interval_minutes` | `ZAEHLER_HOUSEKEEPING__INTERVAL_MINUTES` | `60` |
| `housekeeping.draft_ttl_hours` | `ZAEHLER_HOUSEKEEPING__DRAFT_TTL_HOURS` | `24` |
| `upload.max_photo_bytes` | `ZAEHLER_UPLOAD__MAX_PHOTO_BYTES` | `15728640` (15 MiB) |
| `upload.max_csv_bytes` | `ZAEHLER_UPLOAD__MAX_CSV_BYTES` | `5242880` (5 MiB) |
| `upload.max_document_bytes` | `ZAEHLER_UPLOAD__MAX_DOCUMENT_BYTES` | `10485760` (10 MiB), Rechnungen als PDF oder Foto |
| `upload.max_backup_bytes` | `ZAEHLER_UPLOAD__MAX_BACKUP_BYTES` | `1073741824` (1 GiB), Haushaltsarchive |
| `backup.before_migrate` | `ZAEHLER_BACKUP__BEFORE_MIGRATE` | `true`; Archiv vor ausstehenden Migrationen |
| `backup.dir` | `ZAEHLER_BACKUP__DIR` | `data/backups` |
| `backup.keep` | `ZAEHLER_BACKUP__KEEP` | `3` (mindestens 1) |
| `oidc.issuer_url`, `client_id`, `client_secret` | `ZAEHLER_OIDC__…` | kein Default |
| `auth.dev_login` | `ZAEHLER_AUTH__DEV_LOGIN` | `false`; nur mit `public_url` auf localhost zulässig |
| `session.ttl_hours` | `ZAEHLER_SESSION__TTL_HOURS` | `720` (gleitend) |
| `session.cookie_secure` | `ZAEHLER_SESSION__COOKIE_SECURE` | aus dem Schema von `public_url` |
| `ai.api_key`, `ai.model`, `ai.base_url`, `ai.timeout_secs` | `ZAEHLER_AI__…` | Base-URL `https://openrouter.ai/api/v1`, 60 s |
| `ai.document_model` | `ZAEHLER_AI__DOCUMENT_MODEL` | ohne Angabe `ai.model`; Modell für Rechnungen (PDF als `file`-Part) |
| `notify.smtp.host`, `port`, `security`, `username`, `password`, `from` | `ZAEHLER_NOTIFY__SMTP__…` | ohne `host` kein Versand; Port `587`, `security` `starttls` (`tls`, `none` nur mit `localhost`) |
| `otel.endpoint` | `ZAEHLER_OTEL__ENDPOINT` | leer; ohne Angabe `OTEL_EXPORTER_OTLP_ENDPOINT`, sonst kein Trace-Export |
| `otel.service_name` | `ZAEHLER_OTEL__SERVICE_NAME` | `zaehler` |

S3-Zugangsdaten werden aus `storage.access_key_id` und
`storage.secret_access_key` gelesen, ohne Angabe aus den Standardvariablen
`AWS_ACCESS_KEY_ID` und `AWS_SECRET_ACCESS_KEY`. Der Bucket muss vorhanden
sein; adressiert wird im Pfadstil (`endpoint/bucket/schlüssel`), passend für
MinIO, Garage und Ceph RGW. `storage.allow_http` ist nur für Endpunkte
im selben Netz gedacht.

Das Housekeeping ({need}`SPEC_OPS_HOUSEKEEPING`) läuft in jeder Instanz beim
Start und danach im Abstand von `housekeeping.interval_minutes`: offene
Foto-Entwürfe älter als `housekeeping.draft_ttl_hours` werden verworfen und
ihre Fotos gelöscht, temporäre Objekte unter `households/*/tmp/` älter als
24 Stunden und abgelaufene Sitzungen werden entfernt. Parallele Läufe
mehrerer Replicas sind zulässig.

Ohne Abschnitt `ai` ist die automatische Erkennung abgeschaltet
({need}`REQ_AI_006`); die Foto-Erfassung speichert dann das Foto und
verlangt die Eingabe des Stands von Hand. An OpenRouter gehen nur das
verkleinerte JPEG ohne Metadaten und ein fester Prompt.

Hintergrundjobs ({need}`SPEC_JOB_SCHEDULER`) laufen nur auf der Instanz mit
dem Leader-Lock (PostgreSQL: Advisory-Lock über `database.lock_url`; SQLite:
immer): `DispatchOutbox` in jedem Takt von `scheduler.tick_seconds`,
`SendReminders` stündlich. Ohne `notify.smtp` werden keine E-Mails
versendet; Einladungen erscheinen dann nur als Link. Erinnerungen gehen
frühestens ab 08:00 Uhr in der Zeitzone des Haushalts und höchstens einmal je
Tag an Mitglieder und Eigentümer; Alarme bei auffälligem Verbrauch höchstens
einmal je Ablesung. Benutzername und Passwort sind nur gemeinsam zulässig.
Der Mail-Server sollte SPF und DKIM für die Absenderdomain unterstützen,
sonst landen E-Mails im Spam. Fehler beim Versand zeigt
`zaehler_notifications_total{outcome="error"}`; die Outbox wiederholt mit
Backoff bis zu zehn Versuche.

Logs, Metriken und Traces beschreibt {doc}`observability`.

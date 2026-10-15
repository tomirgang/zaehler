# Konfiguration

Quellen in aufsteigender Priorität ({need}`SPEC_OPS_CONFIG`):

1. eingebaute Defaults
2. TOML-Datei (`zaehler --config pfad.toml` oder `ZAEHLER_CONFIG`), Beispiel:
   `config/zaehler.example.toml`
3. Umgebungsvariablen `ZAEHLER_<SEKTION>__<SCHLÜSSEL>`

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
| `log.format` | `ZAEHLER_LOG__FORMAT` | `json` (`pretty`) |
| `log.filter` | `ZAEHLER_LOG__FILTER` / `RUST_LOG` | `info` |
| `storage.kind` | `ZAEHLER_STORAGE__KIND` | `fs` (`s3`) |
| `storage.path` | `ZAEHLER_STORAGE__PATH` | `data/files` |
| `storage.bucket`, `endpoint`, `region` | `ZAEHLER_STORAGE__…` | kein Default |
| `oidc.issuer_url`, `client_id`, `client_secret` | `ZAEHLER_OIDC__…` | kein Default |
| `auth.dev_login` | `ZAEHLER_AUTH__DEV_LOGIN` | `false`; nur mit `public_url` auf localhost zulässig |
| `session.ttl_hours` | `ZAEHLER_SESSION__TTL_HOURS` | `720` (gleitend) |
| `session.cookie_secure` | `ZAEHLER_SESSION__COOKIE_SECURE` | aus dem Schema von `public_url` |
| `ai.api_key`, `ai.model`, `ai.base_url`, `ai.timeout_secs` | `ZAEHLER_AI__…` | Base-URL `https://openrouter.ai/api/v1`, 60 s |
| `notify.smtp.*` | `ZAEHLER_NOTIFY__SMTP__…` | Port 587 |

S3-Zugangsdaten werden über die Standardvariablen `AWS_ACCESS_KEY_ID` und
`AWS_SECRET_ACCESS_KEY` gelesen.

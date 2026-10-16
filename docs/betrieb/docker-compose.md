# docker compose

## SQLite (Standard)

```bash
cp .env.example .env          # OpenRouter-Key, OIDC usw. eintragen
docker compose up -d --build
open http://localhost:8080
```

Daten (SQLite-Datei und Fotos) liegen im Volume `zaehler-data`.

Der Port auf dem Host ist über `COMPOSE_HTTP_PORT` änderbar (Standard
`8080`); `server.public_url` muss dazu passen.

## Overrides

Weitere Compose-Dateien werden mit `-f` kombiniert:

```bash
docker compose -f compose.yaml -f compose.postgres.yaml -f compose.s3.yaml up -d --build
```

| Datei | Wirkung |
|---|---|
| `compose.postgres.yaml` | PostgreSQL 18 statt SQLite |
| `compose.s3.yaml` | Fotos und Archive in einem S3-kompatiblen Objektspeicher (Versity Gateway, Bucket `zaehler`) statt im Volume |
| `compose.otel.yaml` | OpenTelemetry-Collector; Traces der App und Metriken von `/metrics` erscheinen im Log des Collectors (`docker compose logs -f otel-collector`) |
| `compose.keycloak.yaml` | Anmeldung über OIDC mit Keycloak (Realm `zaehler`, Nutzer `demo`/`demo`) statt Entwicklungs-Login |

Alle Zugangsdaten der Overrides gelten nur für die lokale Umgebung. Keycloak
ist im Browser und im Compose-Netz unter `http://keycloak.localhost:8180`
erreichbar; Namen unter `.localhost` lösen Browser auf `127.0.0.1` auf.

## Smoke-Test

`deploy/smoke-test.sh` baut das Image, startet die Umgebung, prüft
`/readyz`, meldet sich mit dem Entwicklungs-Login an, legt eine Messstelle
an, erfasst einen Stand und prüft Detailseite und `chart.json`. Danach wird
die Umgebung samt Volumes entfernt. Weitere Compose-Dateien werden als
Argumente übergeben; die CI führt den Test für SQLite sowie für PostgreSQL mit
S3 und OTel aus.

```bash
deploy/smoke-test.sh
deploy/smoke-test.sh compose.postgres.yaml compose.s3.yaml compose.otel.yaml
```

Der Test liest keine lokale `.env` und veröffentlicht den Port `18080`.

## E-Mail-Benachrichtigungen

SMTP-Zugang in `.env` eintragen (siehe `.env.example`):

```bash
ZAEHLER_NOTIFY__SMTP__HOST=smtp.example.com
ZAEHLER_NOTIFY__SMTP__USERNAME=…
ZAEHLER_NOTIFY__SMTP__PASSWORD=…
ZAEHLER_NOTIFY__SMTP__FROM=zaehler@example.com
```

# docker compose

## SQLite (Standard)

```bash
cp .env.example .env          # OpenRouter-Key, OIDC usw. eintragen
docker compose up -d --build
open http://localhost:8080
```

Daten (SQLite-Datei und Fotos) liegen im Volume `zaehler-data`.

## PostgreSQL

```bash
docker compose -f compose.yaml -f compose.postgres.yaml up -d --build
```

## E-Mail-Benachrichtigungen

SMTP-Zugang in `.env` eintragen (siehe `.env.example`):

```bash
ZAEHLER_NOTIFY__SMTP__HOST=smtp.example.com
ZAEHLER_NOTIFY__SMTP__USERNAME=…
ZAEHLER_NOTIFY__SMTP__PASSWORD=…
ZAEHLER_NOTIFY__SMTP__FROM=zaehler@example.com
```

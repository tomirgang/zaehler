#!/usr/bin/env bash
# @impl SPEC_OPS_COMPOSE
# Smoke-Test der Auslieferung mit docker compose: Image bauen, starten,
# /readyz, Entwicklungs-Login, Messstelle anlegen, Stand erfassen, Anzeige
# prüfen, Foto hochladen und abrufen; mit compose.otel.yaml zusätzlich
# Traces im Collector. Weitere Compose-Dateien als Argumente, z.B.
#
#   deploy/smoke-test.sh                      # SQLite, Dateiablage
#   deploy/smoke-test.sh compose.postgres.yaml compose.s3.yaml
#
# Die HTTP-Aufrufe laufen in einem curl-Container im Compose-Netz; so
# funktioniert der Test auch in CI-Jobs, die Docker über den Socket nutzen.
set -euo pipefail

cd "$(dirname "$0")/.."
files=(-f compose.yaml)
for f in "$@"; do
  files+=(-f "$f")
done
project="${SMOKE_PROJECT:-zaehler-smoke}"
# ohne lokale .env (Listen-Adresse, AI-Schlüssel) und auf einem freien Port
export COMPOSE_APP_ENV_FILE="${COMPOSE_APP_ENV_FILE:-/dev/null}"
export COMPOSE_HTTP_PORT="${COMPOSE_HTTP_PORT:-18080}"
curl_image="curlimages/curl:8.16.0"
base="http://zaehler:8080"
cookie=""

compose() {
  # --env-file: keine Variablen aus der lokalen .env einsetzen
  docker compose --env-file /dev/null -p "$project" "${files[@]}" "$@"
}

cleanup() {
  local status=$?
  if [ "$status" -ne 0 ]; then
    echo "Smoke-Test fehlgeschlagen; Log des Servers:" >&2
    compose logs --tail 80 zaehler >&2 || true
  fi
  compose down -v --remove-orphans >/dev/null 2>&1 || true
  exit "$status"
}
trap cleanup EXIT

# Antwort mit Kopfzeilen in `res`; merkt sich das Sitzungscookie. Ohne
# Subshell aufrufen, sonst geht das Cookie verloren.
# `stdin`: Datei, die curl auf der Standardeingabe erhält (Uploads)
res=""
stdin=/dev/null
http() {
  res=$(docker run --rm -i --network "${project}_default" "$curl_image" \
    -sS -D - ${cookie:+-H "Cookie: $cookie"} "$@" < "$stdin" | tr -d '\r') || return 1
  local set
  set=$(printf '%s' "$res" | sed -n 's/^[Ss]et-[Cc]ookie: \(zaehler_session=[^;]*\).*/\1/p' | tail -1)
  if [ -n "$set" ]; then
    cookie="$set"
  fi
}

status_of() {
  printf '%s\n' "$1" | sed -n '1s/^HTTP\/[0-9.]* \([0-9]*\).*/\1/p'
}

location_of() {
  printf '%s\n' "$1" | sed -n 's/^[Ll]ocation: //p' | head -1
}

expect() {
  local got
  got=$(status_of "$1")
  if [ "$got" != "$2" ]; then
    echo "$3: HTTP $got statt $2" >&2
    printf '%s\n' "$1" | head -40 >&2
    exit 1
  fi
}

echo "Start: ${files[*]}"
compose up -d --build

for i in $(seq 1 60); do
  if http "$base/readyz" 2>/dev/null && [ "$(status_of "$res")" = "200" ]; then
    break
  fi
  if [ "$i" -eq 60 ]; then
    echo "/readyz nicht erreichbar" >&2
    exit 1
  fi
  sleep 2
done
echo "bereit"

http "$base/auth/dev-login?name=smoke"
expect "$res" 303 "Entwicklungs-Login"
[ -n "$cookie" ] || { echo "kein Sitzungscookie" >&2; exit 1; }

http "$base/settings"
expect "$res" 200 "Einstellungen"
csrf=$(printf '%s' "$res" | sed -n 's/.*name="_csrf" value="\([^"]*\)".*/\1/p' | head -1)
[ -n "$csrf" ] || { echo "kein CSRF-Token" >&2; exit 1; }

http "$base/meters" \
  --data-urlencode "_csrf=$csrf" --data-urlencode "name=Strom Smoke" \
  -d kind=electricity -d unit= -d serial_number= -d rollover_digits= \
  -d decimal_places=1 -d reading_interval_days=30 -d max_daily_rate= \
  -d billing_month= -d billing_day= --data-urlencode "start_at=2025-01-01T00:00" \
  -d register_count=1 -d register_0_id= -d register_0_code=main \
  -d register_0_direction=import -d register_0_start=1000
expect "$res" 303 "Messstelle anlegen"
meter=$(location_of "$res")
echo "Messstelle: $meter"

http "$base$meter/readings/new"
expect "$res" 200 "Erfassungsformular"
field=$(printf '%s' "$res" | grep -o 'name="value_[0-9a-f-]*"' | head -1 | sed 's/name="\(.*\)"/\1/')
[ -n "$field" ] || { echo "kein Eingabefeld" >&2; exit 1; }

http "$base$meter/readings" \
  --data-urlencode "_csrf=$csrf" --data-urlencode "at=2025-02-01T00:00" \
  --data-urlencode "$field=1234,5" -d note=
expect "$res" 303 "Stand erfassen"

http "$base$meter"
expect "$res" 200 "Detailseite"
printf '%s' "$res" | grep -q '1234,5' || { echo "erfasster Stand fehlt" >&2; exit 1; }

http "$base$meter/chart.json"
expect "$res" 200 "chart.json"

# Foto-Upload ohne AI: Ablage im Objektspeicher (Volume oder S3)
stdin=crates/zaehler-web/static/icon-192.png
http "$base/capture" -F "_csrf=$csrf" -F "photo=@-;filename=zaehler.png;type=image/png"
stdin=/dev/null
expect "$res" 303 "Foto hochladen"
draft=$(location_of "$res")
http "$base$draft/photo" -o /dev/null
expect "$res" 200 "Foto abrufen"
printf '%s' "$res" | grep -qi '^content-type: image/jpeg' || { echo "kein JPEG" >&2; exit 1; }

# mit compose.otel.yaml: Traces kommen im Collector an
if printf '%s\n' "$@" | grep -qx 'compose.otel.yaml'; then
  for i in $(seq 1 20); do
    if compose logs otel-collector 2>/dev/null | grep -q 'Traces.*spans'; then
      echo "Traces im Collector"
      break
    fi
    if [ "$i" -eq 20 ]; then
      echo "keine Traces im Collector" >&2
      compose logs --tail 30 otel-collector >&2 || true
      exit 1
    fi
    sleep 2
  done
fi

echo "Smoke-Test erfolgreich"

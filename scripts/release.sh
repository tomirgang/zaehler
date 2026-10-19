#!/usr/bin/env bash
# Baut das Container-Image und überträgt es in die Container-Registry von
# Forgejo, getaggt mit der Version aus Cargo.toml und mit `latest`.
#
# Aufruf:  scripts/release.sh [--dry-run] [--allow-dirty]
#   --dry-run baut das Image, überträgt es aber nicht.
#
# Umgebung (alle optional):
#   REGISTRY         Registry-Host            (Default: code.do9ita.de)
#   IMAGE            vollständiger Image-Name (Default: $REGISTRY/tom/zaehler)
#   PLATFORMS        Zielplattformen          (Default: linux/amd64)
#   FORGEJO_USER     Benutzer für docker login, zusammen mit FORGEJO_TOKEN
#   FORGEJO_TOKEN    Zugriffstoken mit Berechtigung write:package
#
# Ohne FORGEJO_USER/FORGEJO_TOKEN wird eine bestehende Anmeldung
# (`docker login $REGISTRY`) verwendet.
# @impl SPEC_OPS_IMAGE
set -euo pipefail

cd "$(dirname "$0")/.."

dry_run=false
allow_dirty=false
for arg in "$@"; do
  case "$arg" in
    --dry-run) dry_run=true ;;
    --allow-dirty) allow_dirty=true ;;
    -h | --help)
      sed -n '2,16p' scripts/release.sh | sed 's/^# \{0,1\}//'
      exit 0
      ;;
    *)
      echo "unbekannte Option: $arg" >&2
      exit 2
      ;;
  esac
done

REGISTRY=${REGISTRY:-code.do9ita.de}
IMAGE=${IMAGE:-$REGISTRY/tom/zaehler}
PLATFORMS=${PLATFORMS:-linux/amd64}

version=$(awk '
  /^\[workspace.package\]/ { package = 1; next }
  /^\[/ { package = 0 }
  package && /^version = "/ { sub(/^version = "/, ""); sub(/".*$/, ""); print }
' Cargo.toml)
chart_app=$(sed -n 's/^appVersion: "\(.*\)"/\1/p' deploy/helm/zaehler/Chart.yaml)
chart_version=$(sed -n 's/^version: //p' deploy/helm/zaehler/Chart.yaml)
if [[ ! "$version" =~ ^[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z.-]+)?$ ]]; then
  echo "Keine gültige Release-Version in Cargo.toml gefunden" >&2
  exit 1
fi
if [[ "$version" != "$chart_app" || "$version" != "$chart_version" ]]; then
  echo "Version in Cargo.toml ($version) weicht von version/appVersion im Chart ($chart_version/$chart_app) ab" >&2
  exit 1
fi
if [[ "$IMAGE" != "$REGISTRY/"* ]]; then
  echo "IMAGE muss mit REGISTRY/ beginnen ($REGISTRY/)" >&2
  exit 1
fi
if [[ -n "${FORGEJO_USER:-}" && -z "${FORGEJO_TOKEN:-}" || -z "${FORGEJO_USER:-}" && -n "${FORGEJO_TOKEN:-}" ]]; then
  echo "FORGEJO_USER und FORGEJO_TOKEN müssen gemeinsam gesetzt sein" >&2
  exit 1
fi
if ! command -v docker >/dev/null; then
  echo "Docker mit Buildx wird benötigt" >&2
  exit 1
fi

if ! $allow_dirty && [[ -n "$(git status --porcelain)" ]]; then
  echo "Arbeitsverzeichnis enthält nicht committete Änderungen (--allow-dirty übergeht die Prüfung)" >&2
  exit 1
fi

revision=$(git rev-parse HEAD)
created=$(date -u +%Y-%m-%dT%H:%M:%SZ)

echo "Image:      $IMAGE"
echo "Tags:       $version, latest"
echo "Plattform:  $PLATFORMS"
echo "Revision:   $revision"

build_args=(
  --platform "$PLATFORMS"
  --tag "$IMAGE:$version"
  --tag "$IMAGE:latest"
  --label "org.opencontainers.image.version=$version"
  --label "org.opencontainers.image.revision=$revision"
  --label "org.opencontainers.image.created=$created"
  --label "org.opencontainers.image.source=https://$REGISTRY/Tom/zaehler"
)

# Mehrere Plattformen benötigen einen Builder mit dem Treiber docker-container
if [[ "$PLATFORMS" == *,* ]]; then
  if ! docker buildx inspect zaehler-release >/dev/null 2>&1; then
    docker buildx create --name zaehler-release --driver docker-container >/dev/null
  fi
  build_args+=(--builder zaehler-release)
fi

if $dry_run; then
  echo "Probelauf: Image wird gebaut, aber nicht übertragen"
  docker buildx build "${build_args[@]}" .
  exit 0
fi

if [[ -n "${FORGEJO_USER:-}" && -n "${FORGEJO_TOKEN:-}" ]]; then
  printf '%s' "$FORGEJO_TOKEN" | docker login "$REGISTRY" --username "$FORGEJO_USER" --password-stdin
fi

docker buildx build "${build_args[@]}" --push .

echo "Übertragen: $IMAGE:$version und $IMAGE:latest"

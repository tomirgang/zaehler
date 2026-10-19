#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."

tmp=$(mktemp -d)
trap 'rm -f "$tmp/docker" "$tmp/git" "$tmp/calls" "$tmp/output"; rmdir "$tmp"' EXIT
export RELEASE_TEST_CALLS="$tmp/calls"
cat >"$tmp/docker" <<'MOCK'
#!/usr/bin/env bash
printf '%s\n' "$*" >>"$RELEASE_TEST_CALLS"
if [[ "$1" == login ]]; then
  read -r token || [[ -n "$token" ]]
  [[ "$token" == test-token ]]
fi
if [[ "${FAIL_BUILD:-}" == 1 && "$1 $2" == "buildx build" ]]; then
  exit 7
fi
if [[ "${MISSING_BUILDER:-}" == 1 && "$1 $2" == "buildx inspect" ]]; then
  exit 1
fi
MOCK
cat >"$tmp/git" <<'MOCK'
#!/usr/bin/env bash
case "$1" in
  status) printf '%s' "${DIRTY:-}" ;;
  rev-parse) printf '%s\n' 0123456789abcdef ;;
  *) exit 2 ;;
esac
MOCK
chmod +x "$tmp/docker" "$tmp/git"
export PATH="$tmp:$PATH"
unset FORGEJO_USER FORGEJO_TOKEN IMAGE REGISTRY PLATFORMS
(cd scripts && ./release.sh --help) >"$tmp/output"
grep -q 'Aufruf:' "$tmp/output"

scripts/release.sh >"$tmp/output"
grep -q -- '--tag code.do9ita.de/tom/zaehler:1.2.0 --tag code.do9ita.de/tom/zaehler:latest' "$RELEASE_TEST_CALLS"
grep -q -- '--push .' "$RELEASE_TEST_CALLS"

: >"$RELEASE_TEST_CALLS"
FORGEJO_USER=tom FORGEJO_TOKEN=test-token scripts/release.sh >"$tmp/output"
grep -q '^login code.do9ita.de --username tom --password-stdin$' "$RELEASE_TEST_CALLS"
! grep -q test-token "$RELEASE_TEST_CALLS"
! grep -q test-token "$tmp/output"

: >"$RELEASE_TEST_CALLS"
MISSING_BUILDER=1 PLATFORMS=linux/amd64,linux/arm64 scripts/release.sh --dry-run >"$tmp/output"
grep -q 'buildx create --name zaehler-release --driver docker-container' "$RELEASE_TEST_CALLS"
grep -q -- '--builder zaehler-release' "$RELEASE_TEST_CALLS"
! grep -q -- '--push' "$RELEASE_TEST_CALLS"
! grep -q '^login' "$RELEASE_TEST_CALLS"

if DIRTY=' M Cargo.toml' scripts/release.sh >"$tmp/output" 2>&1; then exit 1; fi
DIRTY=' M Cargo.toml' scripts/release.sh --allow-dirty >"$tmp/output"
if FORGEJO_USER=tom scripts/release.sh >"$tmp/output" 2>&1; then exit 1; fi
if IMAGE=other.example/tom/zaehler scripts/release.sh >"$tmp/output" 2>&1; then exit 1; fi
if scripts/release.sh --unknown >"$tmp/output" 2>&1; then exit 1; fi
if FAIL_BUILD=1 scripts/release.sh >"$tmp/output" 2>&1; then exit 1; fi
! grep -q '^Übertragen:' "$tmp/output"
printf '%s\n' 'Release-Skripttests erfolgreich.'

# Zähler

Cloud-/Kubernetes-native Web-App in Rust zum Erfassen und Auswerten von
Zählerständen: Strom, Gas, Wasser, Wärme, Fahrzeug-Kilometer und weitere Verbrauchsgrößen.

- Erfassung per Formular oder **Foto mit AI-Erkennung** (OpenRouter)
- Vergleiche mit Vormonat, Vorjahr und 3/6/12-Monats-Durchschnitt, Ampel für den Erwartungsbereich
- **Prognose** von Jahresverbrauch und Kosten, Abgleich mit dem Abschlag
- Tarife mit Grund-/Arbeitspreis, Staffeln, Boni, mehreren Kostenpositionen
- Fahrzeuge: Kostenbuch, Wertverlust, **Kosten pro km** je Jahr und über die Haltedauer
- Erinnerungen und Alarme per E-Mail
- PostgreSQL (CloudNativePG) oder SQLite, Helm-Chart und docker compose

Version **1.1.0**: siehe [Umsetzungsplan v1.1](docs/impl/V1.1.md).

## Schnellstart

```bash
# lokal (SQLite unter ./data, Entwicklungs-Login unter /auth/login)
ZAEHLER_AUTH__DEV_LOGIN=true ZAEHLER_LOG__FORMAT=pretty cargo run -p zaehler-server
# → http://localhost:8080, Metriken unter http://localhost:9090/metrics

# docker compose
cp .env.example .env
docker compose up -d --build
```

Kubernetes: siehe [docs/betrieb/kubernetes.md](docs/betrieb/kubernetes.md).

## Dokumentation

Anforderungen, Architektur, ADRs und Betrieb liegen in [`docs/`](docs/) und
werden mit Sphinx + sphinx-needs gerendert (inkl. Requirements-Tracing bis in
den Code):

```bash
uv run --project docs sphinx-autobuild docs docs/_build/html
```

## Entwicklung

```bash
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all
```

Konventionen zum Tracing (`// @impl SPEC_…`, `// @test SPEC_…`) stehen in
[docs/entwicklung/index.md](docs/entwicklung/index.md).

## Container-Release auf Forgejo

Docker mit Buildx wird benötigt. Zuerst bei der Registry anmelden:

```bash
docker login code.do9ita.de
scripts/release.sh
```

Das Skript baut mit dem vorhandenen Dockerfile und überträgt
`code.do9ita.de/tom/zaehler:1.1.0` und `code.do9ita.de/tom/zaehler:latest`.
Die Version wird aus `Cargo.toml` gelesen und mit `version` und `appVersion`
des Helm-Charts verglichen. Nicht committete Änderungen verhindern einen
Release; `--allow-dirty` erlaubt bewusst einen lokalen Entwicklungsstand.
`--dry-run` baut ohne Push und ohne Anmeldung.

Für CI können `FORGEJO_USER` und `FORGEJO_TOKEN` (Paket-Schreibrecht) gemeinsam
gesetzt werden. Das Token gelangt ausschließlich über Standardeingabe an
`docker login`, nicht als Kommandozeilenargument. Alternativ wird die bestehende
Docker-Anmeldung verwendet. `REGISTRY`, `IMAGE` (mit Registry-Präfix) und
`PLATFORMS` sind überschreibbar, beispielsweise:

```bash
PLATFORMS=linux/amd64,linux/arm64 scripts/release.sh
bash scripts/test-release.sh
```

Für mehrere Plattformen wird bei Bedarf der Buildx-Builder `zaehler-release`
mit dem Treiber `docker-container` angelegt. Image-Build und Push erfolgen
gemeinsam mit beiden Tags; das Skript legt weder Git-Tags noch Commits an.

## Lizenz

Dual lizenziert unter [MIT](LICENSE-MIT) oder [Apache-2.0](LICENSE-APACHE), nach Wahl.

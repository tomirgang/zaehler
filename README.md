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

> Status: Grundgerüst. Fachlogik-Kern, Datenbankschema, Server mit Health/Metrics,
> Deployment und Dokumentation stehen; Features folgen gemäß
> [Anforderungen](docs/anforderungen/index.md).

## Schnellstart

```bash
# lokal (SQLite unter ./data)
ZAEHLER_LOG__FORMAT=pretty cargo run -p zaehler-server
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

## Lizenz

Dual lizenziert unter [MIT](LICENSE-MIT) oder [Apache-2.0](LICENSE-APACHE), nach Wahl.

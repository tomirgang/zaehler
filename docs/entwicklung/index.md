# Entwicklung

## Voraussetzungen

- Rust stable (siehe `rust-toolchain.toml`)
- [uv](https://docs.astral.sh/uv/) für die Dokumentation
- optional Docker, Helm, Graphviz

## Häufige Befehle

```bash
ZAEHLER_AUTH__DEV_LOGIN=true ZAEHLER_LOG__FORMAT=pretty cargo run -p zaehler-server
                                              # Server mit SQLite unter ./data, Login unter /auth/login
cargo test --workspace                        # Tests (SQLite)
ZAEHLER_TEST_DATABASE_URL=postgres://… cargo test -p zaehler-db   # zusätzlich Postgres
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all

uv run --project docs sphinx-autobuild docs docs/_build/html      # Doku live
uv run --project docs sphinx-build -W --keep-going -b html docs docs/_build/html

# Mermaid-Diagramme prüfen (rendert erst im Browser, daher eigener Schritt)
docker run --rm -u "$(id -u):$(id -g)" -v "$PWD":/repo:ro -w /repo \
  -e MMDC=/home/mermaidcli/node_modules/.bin/mmdc -e MMDC_ARGS="-p /puppeteer-config.json" \
  --entrypoint node ghcr.io/mermaid-js/mermaid-cli/mermaid-cli:11.12.0 docs/tools/check-mermaid.mjs docs
```

## Verzeichnisstruktur

```text
crates/                 hexagonale Architektur (ADR-022)
  zaehler-domain/       Kern: Fachlogik (IO-frei)
  zaehler-app/          Kern: Anwendungsfälle, Port-Traits, Tenant, In-Memory-Adapter (Feature testing)
  zaehler-web/          Driving-Adapter: HTTP, Templates, Assets, i18n, Sitzung, OIDC
  zaehler-db/           Driven-Adapter: SeaORM, Migrationen, Festkomma
  zaehler-storage/      Driven-Adapter: Objektspeicher (ab WP_MVP_06)
  zaehler-media/        Driven-Adapter: Bildverarbeitung (ab WP_MVP_06)
  zaehler-ai/           Driven-Adapter: OpenRouter
  zaehler-notify/       Driven-Adapter: E-Mail
  zaehler-server/       Composition Root, Binary `zaehler`
config/             Beispielkonfiguration
deploy/helm/        Helm-Chart
docs/               Diese Dokumentation (Sphinx + sphinx-needs)
.forgejo/workflows/ CI
```

## Requirements-Tracing im Code

Jede Funktion, die eine Spezifikation umsetzt, und jeder Test, der sie prüft,
bekommt **direkt darüber** einen Tag-Kommentar:

```rust
// @impl SPEC_CONS_DELTA
pub fn delta(prev: Decimal, next: Decimal, rollover_digits: Option<u8>) -> Result<Decimal, ConsumptionError> {
    todo!()
}

// @test SPEC_CONS_DELTA
#[test]
fn delta_handles_rollover() {
    todo!()
}
```

- Mehrere IDs durch Komma trennen: `// @impl SPEC_A, SPEC_B`.
- Ziel sind in der Regel `SPEC_*`; `REQ_*` nur, wenn es keine Spezifikation gibt.
- Offene Arbeiten im Code als `TODO(REQ_…)` markieren.
- Der Doku-Build erzeugt daraus {doc}`../nachverfolgung/code`; unbekannte IDs
  lassen die CI fehlschlagen.

## Datenbankzugriffe

Nur Query-Builder oder `Statement::from_sql_and_values` mit Platzhaltern
verwenden ({need}`SPEC_SEC_SQL_PARAMS`). Roh-SQL-APIs sind in `clippy.toml`
gesperrt; clippy schlägt dann mit `use of a disallowed method` fehl.

```rust
// richtig: Wert als Bind-Parameter
db.execute_raw(Statement::from_sql_and_values(
    DbBackend::Postgres,
    "SELECT pg_advisory_xact_lock($1)",
    [key.into()],
))
.await?;
```

## Neue Anforderung umsetzen

1. Anforderung (`req`) prüfen bzw. ergänzen, ggf. ADR anlegen.
2. Spezifikation (`spec`) mit `:satisfies:` schreiben.
3. Implementieren und testen, Tags setzen.
4. Status der Spezifikation auf `implemented` setzen.
5. {doc}`../nachverfolgung/index` prüfen.

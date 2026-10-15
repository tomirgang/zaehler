# ADR-014: Cargo-Workspace mit fachlich getrennten Crates

```{adr} Workspace-Struktur
:id: ADR_014
:status: superseded
:decides: REQ_NFR_008, REQ_NFR_005

**Datum:** 2026-10-15

Ersetzt durch {need}`ADR_022`.
```

## Kontext
Fachlogik soll unabhängig von Web, Datenbank und externen Diensten testbar sein.

## Entscheidung
Crates `zaehler-domain` (IO-frei), `zaehler-db`, `zaehler-ai`, `zaehler-notify`
und `zaehler-server` (Binary `zaehler`). Gemeinsame Versionen und Lints im
Workspace (`clippy::pedantic`, `unsafe_code = forbid`).

## Alternativen
- *Ein Crate mit Modulen*: weniger Manifeste; Abhängigkeitsgrenzen nicht erzwungen,
  längere inkrementelle Builds.

## Konsequenzen
- `zaehler-domain` darf keine IO-Abhängigkeiten bekommen (Review-Kriterium).

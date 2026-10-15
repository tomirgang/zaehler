# ADR-002: Server-Side Rendering mit Askama und HTMX

```{adr} SSR mit Askama, HTMX und ECharts
:id: ADR_002
:status: accepted
:decides: REQ_NFR_001, REQ_ANA_007, REQ_NFR_003

**Datum:** 2026-10-15
```

## Kontext
Die Oberfläche besteht überwiegend aus Formularen, Listen und Diagrammen.
Ein separates SPA-Toolchain (Node, Bundler) erhöht Aufwand und Angriffsfläche.

## Entscheidung
- HTML wird serverseitig mit **Askama** (typgeprüfte Jinja-Templates) gerendert.
- Interaktivität über **HTMX** (Fragmente, `hx-boost`).
- Diagramme mit **Apache ECharts**; Daten als JSON-Endpunkte.
- Statische Assets werden ins Binary eingebettet, kein JS-Build.

## Alternativen
- *Leptos* (Rust/WASM, SSR + Hydration): erfordert zusätzlich ein WASM-Target und
  `cargo-leptos` im Build; höhere Compile-Zeiten.
- *REST-API + React/Svelte*: zwei Toolchains, doppelte Validierung.

## Konsequenzen
- Strikte CSP ohne Inline-Skripte möglich (HTMX-Attribute statt Inline-JS).
- Komplexe clientseitige Interaktionen (z.B. Foto-Zuschnitt) erfordern eigene
  JS-Module.

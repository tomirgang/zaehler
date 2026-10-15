# ADR-012: Dokumentation mit Sphinx und sphinx-needs

```{adr} Sphinx + sphinx-needs mit Code-Tags
:id: ADR_012
:status: accepted
:decides: REQ_NFR_008

**Datum:** 2026-10-15
```

## Kontext
Anforderungen, Entscheidungen, Spezifikationen, Code und Tests sollen
verknüpft, versioniert (Docs-as-Code) und als Website gerendert werden.

## Entscheidung
- **Sphinx** mit **MyST-Markdown**, Theme Furo, Diagramme mit Mermaid.
- **sphinx-needs** mit Need-Typen `req`, `spec`, `adr`, `impl`, `test` und
  Link-Typen `satisfies`, `implements`, `verifies`, `decides`; Felder mit
  JSON-Schema (Priorität, Meilenstein).
- **Code-Tracing:** Kommentare `// @impl SPEC_…` / `// @test SPEC_…` im Rust-Code
  werden von `docs/_ext/codetrace.py` zu `impl`/`test`-Needs.
- CI baut mit `-W`: tote Links und Schema-Verstöße brechen den Build.
- `needs.json` wird mitgebaut und kann von anderen Werkzeugen gelesen werden.

## Alternativen
- *OpenFastTrack*: Tags in Code und Markdown, Coverage-Prüfung; Ausgabe als
  Prüfbericht, keine Dokumentations-Website.
- *StrictDoc*: Web-UI zur Bearbeitung; eigenes SDoc-Format statt Markdown.
- *mdBook*: Markdown-Website ohne Requirements-Tracing.

## Konsequenzen
- Python-Toolchain (uv) nur für die Doku.
- Graphviz für `needflow`-Graphen (lokal optional).

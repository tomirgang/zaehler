# Bausteine

Hexagonale Architektur ({need}`ADR_022`): ein fachlicher Kern mit
Anwendungsfällen und Port-Traits, umgeben von Adaptern. Abhängigkeiten zeigen
ausschließlich nach innen.

```{mermaid}
flowchart LR
  subgraph driving[Driving-Adapter]
    web[zaehler-web<br/>HTTP: Web-UI, REST-API ab v1]
  end
  subgraph core[Kern]
    app[zaehler-app<br/>Anwendungsfälle, Ports, Tenant]
    domain[zaehler-domain<br/>Fachlogik, IO-frei]
  end
  subgraph driven[Driven-Adapter]
    db[zaehler-db<br/>SeaORM, PostgreSQL, SQLite]
    storage[zaehler-storage<br/>object_store]
    media[zaehler-media<br/>Bildverarbeitung]
    ai[zaehler-ai<br/>OpenRouter]
    notify[zaehler-notify<br/>SMTP]
  end
  server[zaehler-server<br/>Composition Root, Binary zaehler]

  web --> app
  app --> domain
  db --> app
  storage --> app
  media --> app
  ai --> app
  notify --> app
  server --> web
  server --> db
  server --> storage
  server --> media
  server --> ai
  server --> notify
```

| Crate | Rolle | Verantwortung | Externe Abhängigkeiten |
|---|---|---|---|
| `zaehler-domain` | Kern | Entitäten, Wertobjekte, Verbrauch, Serien, Vergleiche, Erwartungsbereich, Prognose, Tarife, Fahrzeugkosten, Kalender | `chrono`, `chrono-tz`, `rust_decimal` |
| `zaehler-app` | Kern | Anwendungsfälle (Driving Ports), Port-Traits (Driven Ports), `Tenant`, Unit of Work, In-Memory-Adapter für Tests (Feature `testing`) | `async-trait`, `thiserror`, `csv`, Fassaden `tracing` und `metrics` |
| `zaehler-web` | Driving-Adapter | Axum-Router, Handler, Extraktoren, Askama-Templates, HTMX-Fragmente, i18n, Sitzung, CSRF, OIDC-Ablauf | `axum`, `askama`, `tower-sessions`, `openidconnect`, `fluent-templates` |
| `zaehler-db` | Driven-Adapter | Repository- und Unit-of-Work-Ports, Migrationen, Festkomma, Sitzungsspeicher | `sea-orm`, `sea-orm-migration` |
| `zaehler-storage` | Driven-Adapter | Objektspeicher-Port | `object_store` |
| `zaehler-media` | Driven-Adapter | Bildverarbeitungs-Port | `image` |
| `zaehler-ai` | Driven-Adapter | Erkennungs-Port (Zählerstand, Beleg, Rechnung) | `reqwest` |
| `zaehler-notify` | Driven-Adapter | Benachrichtigungs-Port | `lettre` (ab v1) |
| `zaehler-server` | Composition Root | Konfiguration, Erzeugen und Verdrahten der Adapter, Telemetrie, Scheduler, CLI | `figment`, `clap`, `tracing-subscriber`, OpenTelemetry |

## Leitlinien

- **Fachlogik ist IO-frei** und wird mit Unit-Tests geprüft.
- **Anwendungsfälle** orchestrieren: Tenant prüfen, über Ports laden,
  Domain-Funktionen aufrufen, über Ports speichern. Sie kennen keine
  Frameworks und keine Adapter-Typen.
- **Haushalts-Scope**: Anwendungsfälle erhalten einen `Tenant`, den nur
  `zaehler-app` nach Prüfung der Mitgliedschaft erzeugt
  ({need}`SPEC_SEC_TENANT_SCOPE`).
- **Kein JavaScript-Build:** HTMX und ECharts werden als statische Dateien in
  `zaehler-web` eingebettet.

# ADR-022: Hexagonale Architektur

```{adr} Hexagonale Architektur mit Kern- und Adapter-Crates
:id: ADR_022
:status: accepted
:decides: SPEC_ARCH_LAYERS, SPEC_SEC_TENANT_SCOPE, REQ_TEN_001, REQ_NFR_008
:links: ADR_014, ADR_018

**Datum:** 2026-10-15
```

## Kontext
Fachlogik und Anwendungsfälle sollen unabhängig von Web-Framework, Datenbank,
Objektspeicher, AI-Anbieter und E-Mail-Versand entwickelt und getestet werden.
Mehrere Adapter je Schnittstelle sind vorgesehen (SQLite und PostgreSQL,
Dateisystem und S3, Web-UI und REST-API) bzw. für Tests nötig (In-Memory,
Stubs). Diese Entscheidung ersetzt {need}`ADR_014` und {need}`ADR_018`.

## Entscheidung
- **Kern** (innen):
  - `zaehler-domain`: Entitäten, Wertobjekte, fachliche Berechnungen; ohne
    Abhängigkeit auf andere Crates des Workspace und ohne IO.
  - `zaehler-app`: Anwendungsfälle (Driving Ports als öffentliche API) und
    Port-Traits für alles Externe (Driven Ports: Repositories, Unit of Work,
    Objektspeicher, Bildverarbeitung, Erkennung, Benachrichtigung, Uhr,
    ID-Erzeugung); hängt nur von `zaehler-domain` ab.
- **Adapter** (außen), je ein Crate, abhängig von `zaehler-app`, nie
  voneinander:
  - Driving: `zaehler-web` (HTTP: Web-UI, ab v1 REST-API).
  - Driven: `zaehler-db` (SeaORM, PostgreSQL und SQLite), `zaehler-storage`
    (`object_store`), `zaehler-media` (`image`), `zaehler-ai` (OpenRouter),
    `zaehler-notify` (SMTP).
- **Composition Root**: `zaehler-server` (Binary `zaehler`) liest die
  Konfiguration, erzeugt die Adapter und verdrahtet sie als
  `Arc<dyn Port>` mit den Anwendungsfällen; enthält Telemetrie und den
  Scheduler, keine Fach- oder Präsentationslogik.
- **Ports** als Traits mit `async-trait`, aufgerufen über `Arc<dyn Port>`.
- **Transaktionen** über einen Unit-of-Work-Port: ein Anwendungsfall öffnet
  eine Transaktion, erhält daraus die Repositories und schließt sie mit
  `commit` ab.
- **Berechtigung**: `Tenant` ist ein Typ in `zaehler-app`; er entsteht nur
  im Anwendungsfall-Kern nach Prüfung der Mitgliedschaft über einen Port.
- Die Abhängigkeitsregeln prüft die CI ({need}`SPEC_ARCH_LAYERS`).

## Alternativen
- *Adapter als Module im Server-Crate*: weniger Crates, Grenzen nur per Review.
- *Ein Crate mit Modulen*: keine durch Cargo erzwungenen Grenzen.
- *Generics statt `dyn`*: statische Dispatch ohne Allokation, aber generische
  Parameter in Anwendungsfällen und Zustand.
- *Atomare Repository-Operationen statt Unit of Work*: kein
  Transaktionskonzept im Kern, dafür je repositoryübergreifendem Fall eine
  eigene Port-Methode.

## Konsequenzen
- Eigene Typen an jeder Port-Grenze; Adapter bilden ihre Typen
  (SeaORM-Entities, OpenRouter-Antworten) auf Kerntypen ab.
- In-Memory-Implementierungen aller Driven Ports (Feature `testing` in
  `zaehler-app`) erlauben Tests der Anwendungsfälle ohne Datenbank.
- Eine Box-Allokation je Port-Aufruf durch `async-trait`.
- Umbau des vorhandenen Grundgerüsts in WP_MVP_01 und WP_MVP_02.

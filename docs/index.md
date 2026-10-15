# Zähler-Dokumentation

**Zähler** ist eine cloud-/Kubernetes-native Web-App in Rust zum Erfassen von
Zählerständen (Strom, Gas, Wasser, Wärme, Fahrzeug-Kilometer u.a.), zur Bewertung
des Verbrauchs, zur Prognose von Jahresverbrauch und Kosten sowie zum Vergleich
mit Abschlägen und Vorperioden.

Die Dokumentation nutzt [sphinx-needs](https://sphinx-needs.readthedocs.io/) für
Requirements-Tracing von der Anforderung bis zu Code und Tests:

```{mermaid}
flowchart LR
  REQ[Anforderung<br/>REQ_*] -->|erfüllt durch| SPEC[Spezifikation<br/>SPEC_*]
  SPEC -->|implementiert durch| IMPL[Code<br/>// @impl]
  SPEC -->|verifiziert durch| TEST[Test<br/>// @test]
  ADR[Entscheidung<br/>ADR_*] -.->|betrifft| REQ
  ADR -.->|betrifft| SPEC
```

Den aktuellen Abdeckungsgrad zeigt die Seite {doc}`nachverfolgung/index`.

```{toctree}
:maxdepth: 2
:caption: Einführung

einfuehrung/vision
einfuehrung/glossar
```

```{toctree}
:maxdepth: 2
:caption: Anforderungen

anforderungen/index
```

```{toctree}
:maxdepth: 2
:caption: Architektur

architektur/index
adr/index
```

```{toctree}
:maxdepth: 2
:caption: Umsetzung

impl/index
```

```{toctree}
:maxdepth: 2
:caption: Betrieb & Entwicklung

betrieb/index
entwicklung/index
```

```{toctree}
:maxdepth: 2
:caption: Nachverfolgung

nachverfolgung/index
```

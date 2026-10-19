# Zähler

**Zähler** ist eine Web-App für Haushalte, die ihren Energie- und
Wasserverbrauch, ihre Photovoltaikanlage und ihre Fahrzeuge im Blick behalten
wollen. Zählerstände werden per Formular, per Foto oder automatisch erfasst;
die App rechnet daraus Verbrauch, Kosten und Prognosen, vergleicht mit
Vorperioden und Abschlägen und warnt bei Auffälligkeiten. Sie läuft im
Browser auf Smartphone und PC, ist installierbar (PWA) und wird selbst
betrieben – als einzelner Container mit SQLite oder in Kubernetes mit
PostgreSQL.

```{figure} handbuch/bilder/uebersicht.png
:alt: Übersicht der Zähler-App mit Karten je Messstelle
:width: 100%

Die Übersicht: je Messstelle der letzte Stand, der Verbrauch des letzten
Monats mit Ampel und die Jahresprognose.
```

## Funktionen

**Erfassen**

- Zählerstände per Formular mit Plausibilitätsprüfung (fallende Stände,
  ungewöhnlich hoher Tagesverbrauch), mehrere Register je Zähler (z.B.
  HT/NT, Bezug/Einspeisung)
- Stand per Foto: Belegfoto zu jeder Ablesung, mit eingerichteter
  KI-Erkennung Vorschlag von Stand, Zählernummer und Zuordnung zum Zähler;
  mehrere Fotos auf einmal
- Sammelerfassung aller Zähler eines Hausanschlusses mit PV
- CSV-Import von Zählerständen, Rechnungs- und Belegimport (PDF oder Foto)
- Automatisch gelieferte Stände über die REST-API, z.B. aus Home Assistant
  oder SENEC-Exporten
- Korrektur mit Änderungshistorie, Zählerwechsel und Zählerüberlauf
- Ableseplan mit Fälligkeit und E-Mail-Erinnerung

**Auswerten**

- Verbrauch je Monat und Jahr, Vergleich mit Vormonat, Vorjahr und
  gleitenden Durchschnitten
- Erwartungsbereich und Ampel, Alarm bei auffälligem Verbrauch
- Jahresprognose mit Saisonprofil, auch bei Messbeginn mitten im Jahr
- Diagramme mit Tabellenalternative, Auswertung je Register
- Zwischenzähler (Wärmepumpe, Wallbox) mit und ohne Anteil an der
  übergeordneten Messstelle

**Kosten**

- Verträge mit Grund- und Arbeitspreis, Preisständen, Staffeln, mehreren
  Kostenpositionen, Boni und Gutschriften
- Abschläge mit erwarteter Nachzahlung bzw. Erstattung und Vorschlag für den
  neuen Abschlag, tatsächliche Abrechnungen
- Kostenübersicht des Haushalts mit Saldo aus Bezug und Einspeisevergütung
  und Fahrzeugkosten

**Photovoltaik**

- PV-Anlagen mit Hausanschluss, Erzeugungs- und Speicherzähler
- Eigenverbrauch, Autarkiegrad, Speicherkennzahlen
- Ertragserwartung und Ertragsüberwachung
- Wirtschaftlichkeit: Einspeisevergütung, Ersparnis, Amortisation, Rendite

**Fahrzeuge**

- Fahrzeug als Messstelle für den Kilometerstand, Verbrenner, Elektro und
  Plug-in-Hybrid
- Kostenbuch mit Tanken (Literpreis), Laden, Werkstatt, Versicherung, Steuer
  und mehr, wiederkehrende Einträge
- Heimladen über Wallbox-Zähler oder geschätzt aus km und Verbrauch, externes
  Laden, Ladepreis mit PV-Anteil
- Wertverlust nach Restwertkurve (Alter oder Laufleistung), Kosten pro km,
  Prognose von Fahrleistung und Jahreskosten

**Haushalt und Betrieb**

- Mehrere Haushalte, Mitglieder mit Rollen Lesen, Mitglied und Eigentümer,
  Einladungen
- Setup-Assistent für Hausstrom, Wärmepumpe, PV, E-Auto und weitere Medien
- Export als Archiv oder CSV, Import als neuer Haushalt, vollständige
  Sicherung der Installation
- REST-API mit OpenAPI und persönlichen Tokens
- Deutsch und Englisch, barrierearm, mobil bedienbar
- Anmeldung über OIDC (z.B. Keycloak), Betrieb mit docker compose oder
  Helm-Chart

Wie die App bedient wird, beschreibt das {doc}`handbuch/index`.

## Für Entwicklung und Betrieb

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
:caption: Benutzerhandbuch

handbuch/index
handbuch/erste-schritte
handbuch/erfassen
handbuch/auswerten
handbuch/kosten
handbuch/photovoltaik
handbuch/fahrzeuge
handbuch/einstellungen
```

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

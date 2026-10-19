# Photovoltaik

## PV-Anlage anlegen

Unter **PV → PV-Anlage anlegen** entsteht eine Anlage mit:

- **Anlagentyp**: Überschusseinspeisung, Volleinspeisung oder
  Steckersolargerät,
- Leistung in kWp, Inbetriebnahme, Ausrichtung und Neigung, Wechselrichter,
  optional Speicherkapazität,
- **zugeordneten Messstellen**: Hausanschluss (Zweirichtungszähler),
  Erzeugungszähler und optional Speicherzähler,
- Ertragserwartung: spezifischer Jahresertrag (kWh/kWp), Degradation und
  optional eigene Monatsanteile; Alarmschwelle für zu geringen Ertrag.

Mehrere Anlagen können sich einen Hausanschluss teilen.

```{figure} bilder/pv-uebersicht.png
:alt: Liste der PV-Anlagen
:width: 100%

PV-Anlagen mit Typ, Leistung und Hausanschluss.
```

## Anlage und Wirtschaftlichkeit

Die Seite einer Anlage zeigt die Stammdaten und die **Wirtschaftlichkeit**
je Jahr: Einspeisevergütung (aus dem Einspeisevertrag), Ersparnis durch
Eigenverbrauch (zum Arbeitspreis des Bezugsvertrags), Kosten aus dem
Kostenbuch der Anlage und die kumulierte Bilanz. Daraus ergeben sich
**Amortisation** und **Rendite**; das Diagramm schreibt die Bilanz in die
Zukunft fort. Investition, Wartung oder Versicherung trägt man im
**Kostenbuch** der Anlage ein.

```{figure} bilder/pv-anlage.png
:alt: PV-Anlage mit Wirtschaftlichkeit und kumulierter Bilanz
:width: 100%

Anlage „Dach Süd“: Wirtschaftlichkeit je Jahr und kumulierte Bilanz.
```

Weiter unten folgen **Ertrag und Erwartung**: der Ertrag je Monat gegen die
Erwartung aus kWp, spezifischem Ertrag, Degradation und Monatsverteilung.
Liegt ein Monat deutlich unter der Erwartung, meldet die App das.

## Hausanschluss mit PV

**Hausanschluss mit PV** zeigt die Energiebilanz aller Anlagen am
Hausanschluss:

- **Erzeugung**: Eigenverbrauch und Einspeisung je Monat,
- **Gesamtverbrauch**: Eigenverbrauch und Bezug je Monat,
- je Jahr Erzeugung, Eigenverbrauch, Einspeisung, Bezug, Gesamtverbrauch,
  **Eigenverbrauchsquote** und **Autarkiegrad**, mit Speicher zusätzlich
  Speicherkennzahlen.

```{figure} bilder/pv-hausanschluss.png
:alt: Energiebilanz eines Hausanschlusses mit PV
:width: 100%

Energiebilanz: Erzeugung, Eigenverbrauch, Einspeisung und Bezug je Monat.
```

Von hier aus erfasst die **Sammelerfassung** alle Zähler des Hausanschlusses
mit einem gemeinsamen Zeitpunkt (siehe {doc}`erfassen`).

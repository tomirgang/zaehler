# Verbrauch auswerten

## Übersicht

Die **Übersicht** zeigt je Messstelle eine Karte, gruppiert nach Art und
innerhalb der Gruppe nach Name sortiert:

- letzter Stand je Register mit Zeitpunkt,
- Verbrauch des letzten vollständigen Monats mit Veränderung zum Vormonat und
  **Ampel** (im Rahmen, leicht abweichend, deutlich abweichend),
- **Prognose** für das laufende Jahr mit dem gemessenen Anteil,
- nächste fällige Ablesung; überfällige Karten sind hervorgehoben.

PV-Anlagen erscheinen mit Erzeugung, Autarkiegrad und Eigenverbrauchsquote
des letzten Monats.

## Auswertung einer Messstelle

Die Seite einer Messstelle beginnt mit Stammdaten und Aktionen, darunter
folgt die Auswertung.

```{figure} bilder/messstelle-auswertung.png
:alt: Auswertung einer Messstelle mit Vergleichstabelle
:width: 100%

Hausstrom mit PV: Hinweis auf den Gesamtverbrauch, Zwischenzähler und
Vergleich des gewählten Monats.
```

**Vergleich**: Für den gewählten Bezugsmonat vergleicht die App den
Verbrauch mit Vormonat, Vorjahresmonat und den Durchschnitten der letzten 3,
6 und 12 Monate. Die Ampel bewertet den Monat gegen den Erwartungsbereich:
aus dem Vorjahr, sonst aus dem Jahresziel oder dem Abschlag des Vertrags,
verteilt nach dem Saisonprofil.

**Laufendes Jahr gegen Vorjahr** stellt den Verbrauch seit 1. Januar dem
Vorjahr bis zum gleichen Tag gegenüber.

**Prognose**: Die App rechnet den gemessenen Verbrauch mit dem
Saisonprofil auf das Kalenderjahr hoch, bei hinterlegtem Abrechnungsbeginn
zusätzlich auf das Abrechnungsjahr. Beginnt die Messung mitten im Jahr, wird
die Zeit davor mit derselben Rate geschätzt. Das Saisonprofil lernt die App
aus zwölf vollständigen Monaten; bis dahin gilt das Standardprofil der
Zählerart oder ein eigenes.

```{figure} bilder/messstelle-prognose.png
:alt: Monatswerte des laufenden Jahres und des Vorjahres
:width: 100%

Monatswerte: laufendes Jahr (grün) und Vorjahr (grau); schraffiert sind
prognostizierte Anteile.
```

```{figure} bilder/messstelle-jahresverlauf.png
:alt: Kumulierter Jahresverlauf mit Prognose
:width: 100%

Jahresverlauf: kumulierter Verbrauch und gestrichelt die Prognose bis
Jahresende.
```

Unter jedem Diagramm öffnet **Diagrammwerte als Tabelle** die Zahlen.
Zähler mit mehreren Registern haben zusätzlich eine **Auswertung je
Register**.

## Zwischenzähler

Hat eine Messstelle Zwischenzähler (z.B. Wärmepumpe, Wallbox), zeigt sie
diese oben an. Mit **Ohne Zwischenzähler anzeigen** wird ihr Verbrauch je
Monat abgezogen – etwa um den reinen Haushaltsstrom zu sehen. Die
Zwischenzähler haben eigene Auswertungen.

## Hausanschluss mit PV

Bei einem Hausanschluss mit PV-Anlage beziehen sich Vergleiche, Ampel und
Prognose auf den **Gesamtverbrauch** (Bezug plus selbst verbrauchter
PV-Strom); Kosten und Kostenprognose auf den Bezug. Die Energiebilanz zeigt
{doc}`photovoltaik`.

## Benachrichtigungen

Mit eingerichtetem E-Mail-Versand erinnert die App an fällige Ablesungen und
meldet Ablesungen, deren Verbrauch deutlich außerhalb der Erwartung liegt
(siehe {doc}`einstellungen`).

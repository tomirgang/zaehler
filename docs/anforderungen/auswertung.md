# Auswertung und Vergleiche

```{req} Verbrauch je Zeitraum
:id: REQ_ANA_001
:status: accepted
:priority: must
:release: MVP

Das System muss aus den Zählerständen den Verbrauch je Tag, Monat, Jahr und
Abrechnungszeitraum berechnen. Liegen Ablesungen nicht auf den
Zeitraumgrenzen, wird der Verbrauch zwischen zwei Ablesungen anteilig verteilt.
```

```{req} Saisonprofil
:id: REQ_ANA_002
:status: accepted
:priority: should
:release: v1

Das System soll je Zähler ein Saisonprofil verwenden, das bei mindestens einem
vollständigen Jahr aus der eigenen Historie gelernt wird. Davor gilt ein
Standardprofil je Art (gleichverteilt; für Gas/Wärme winterbetont, anpassbar).
```

```{req} Vergleich mit Vormonat
:id: REQ_ANA_003
:status: accepted
:priority: must
:release: MVP

Das System muss den Verbrauch eines Monats mit dem Vormonat vergleichen
(absolut und in Prozent).
```

```{req} Vergleich mit Vorjahr
:id: REQ_ANA_004
:status: accepted
:priority: must
:release: MVP

Das System muss, sobald Daten über mehrere Jahre vorliegen, Monate, Jahre und
das laufende Jahr bis heute mit dem gleichen Zeitraum des Vorjahres vergleichen.
```

```{req} Vergleich mit gleitenden Durchschnitten
:id: REQ_ANA_005
:status: accepted
:priority: must
:release: MVP

Das System muss den Verbrauch eines Monats mit dem Durchschnitt der letzten 3,
6 und 12 Monate vergleichen.
```

```{req} Erwartungsbereich und Ampel
:id: REQ_ANA_006
:status: accepted
:priority: must
:release: v1

Das System muss bewerten, ob der Verbrauch im erwarteten Bereich liegt:
- mit Historie: saisonbereinigter Erwartungswert aus Vorjahren mit Band (± k·σ),
- ohne Historie: manuelles Jahresziel bzw. Abschlagsbasis mit Toleranz.

Ergebnis ist eine Ampel grün/gelb/rot je Zähler und Monat. Bei einem
Hausanschluss mit PV-Erzeugungszähler bezieht sich die Bewertung auf den
Gesamtverbrauch ({need}`REQ_PV_006`).
```

```{req} Diagramme
:id: REQ_ANA_007
:status: accepted
:priority: must
:release: MVP

Das System muss Verbräuche visualisieren: Monatsbalken mit Vorjahresvergleich,
kumulierter Jahresverlauf gegen Prognose und Erwartungsband.
```

```{req} Dashboard
:id: REQ_ANA_008
:status: accepted
:priority: should
:release: MVP

Das System soll ein Dashboard mit allen aktiven Zählern zeigen: letzter Stand,
Ampel, Jahresprognose (Verbrauch, ab v1 auch Kosten) und fällige Ablesungen.
```

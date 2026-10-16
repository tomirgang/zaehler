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

```{req} Auswertung je Register
:id: REQ_ANA_009
:status: accepted
:priority: must
:release: MVP

Das System muss bei Messstellen mit mehreren Registern die Prognose und den
Verlauf zusätzlich für jedes Register berechnen und als Diagramme darstellen.

**Akzeptanzkriterien**
- Je Register: Prognose für Kalenderjahr und, falls hinterlegt,
  Abrechnungsjahr; Monatswerte des laufenden Jahres und des Vorjahres;
  kumulierter Jahresverlauf mit Prognose.
- Die Werte der Register ergeben in Summe die Werte der Messstelle je Richtung.
- Bei nur einem Register entfällt die Darstellung je Register.
- Liefert ein Register allein die Werte der Messstelle einer Richtung
  (z.B. Bezug 1.8.0 neben Einspeisung 2.8.0), entfällt seine Darstellung je
  Register; die Monatswerte der Messstelle nennen es als Quelle.
```

```{req} Teilweise abgedeckte Monate in den Monatswerten
:id: REQ_ANA_010
:status: accepted
:priority: must
:release: MVP

Das System muss in den Monatsdiagrammen auch Monate darstellen, die die
Ablesungen nur teilweise abdecken (Monat der ersten und der letzten
Ablesung). Der gemessene Anteil wird angezeigt, die fehlenden Tage werden
auf Basis der gemessenen Tage dieses Monats prognostiziert.

**Akzeptanzkriterien**
- Gemessener Anteil und Prognose stehen gestapelt in einem Balken; die
  Prognose ist als solche gekennzeichnet (eigene Legende, schraffiert) und im
  Tooltip und in der Tabellenalternative getrennt ausgewiesen.
- Prognose der fehlenden Tage = gemessener Anteil · fehlende Zeit /
  gemessene Zeit des Monats.
- Monate ohne Ablesung im oder um den Monat bleiben leer.
- Vergleiche, Bezugsmonate und die Monatstabelle verwenden weiterhin nur
  vollständig abgedeckte Monate.
```

```{req} Jahresverlauf mit geschätzten und prognostizierten Abschnitten
:id: REQ_ANA_011
:status: accepted
:priority: must
:release: MVP

Das System muss den kumulierten Jahresverlauf auch dann darstellen, wenn
keine Ablesung am oder vor dem Jahresbeginn vorliegt. Der Verbrauch vor der
ersten Ablesung des Jahres wird geschätzt, der Verbrauch nach der letzten
Ablesung prognostiziert.

**Akzeptanzkriterien**
- Zwischen zwei Ablesungen wird linear interpoliert (wie bisher).
- Vor der ersten Ablesung des Jahres und nach der letzten Ablesung gilt die
  durchschnittliche Rate des gemessenen Zeitraums im Jahr; Schätzung,
  Messung und Prognose ergeben einen durchgehenden Verlauf bis Jahresende.
- Geschätzter Abschnitt (gepunktet) und Prognose (gestrichelt) sind
  gekennzeichnet; ein Hinweis nennt den Beginn des gemessenen Zeitraums.
- Die Jahresprognose der Prognosetabelle stimmt mit dem Endwert des
  Jahresverlaufs überein und weist den geschätzten Beginn aus.
```

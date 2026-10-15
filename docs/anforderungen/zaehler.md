# Zähler

```{req} Messstellen verwalten
:id: REQ_MTR_001
:status: accepted
:priority: must
:release: MVP

Das System muss Messstellen mit Name, Art (Strom, Gas, Wasser, Wärme, Fahrzeug,
Sonstiges), Einheit, Zählernummer, Anzahl Vor- und Nachkommastellen und
optionalem Ableseturnus verwalten.

**Akzeptanzkriterien**
- Zur Art wird eine Standardeinheit vorgeschlagen (kWh, m³, km).
- Zählernummern sind je Haushalt eindeutig, wenn angegeben.
```

```{req} Benutzerdefinierte Zählerarten
:id: REQ_MTR_002
:status: accepted
:priority: should
:release: v1

Das System soll über die Art „Sonstiges“ beliebige Verbrauchsgrößen mit freier
Einheit unterstützen (z.B. Heizöl in l, Pellets in kg, Wärmemengenzähler in MWh).
```

```{req} Mehrere Register je Zähler
:id: REQ_MTR_003
:status: accepted
:priority: must
:release: MVP

Das System muss je Zähler mehrere Register unterstützen, insbesondere
Zweitarifzähler (HT/NT). Verbrauch und Kosten werden je Register und summiert
ausgewiesen.
```

```{req} Zweirichtungszähler und Einspeisung
:id: REQ_MTR_004
:status: accepted
:priority: should
:release: v1

Das System soll Register mit Richtung „Einspeisung“ unterstützen (z.B. PV).
Einspeisung wird getrennt vom Bezug ausgewertet; eine Einspeisevergütung wird
als Gutschrift in der Kostenrechnung berücksichtigt. Kennzahlen und
Wirtschaftlichkeit von PV-Anlagen beschreibt {doc}`photovoltaik`.
```

```{req} Zählerwechsel
:id: REQ_MTR_005
:status: accepted
:priority: must
:release: MVP

Das System muss einen Zählerwechsel abbilden: Endstand des alten und Startstand
des neuen Geräts zum selben Zeitpunkt. Verbrauchsreihen, Vergleiche und
Prognosen laufen über den Wechsel hinweg ohne Sprung weiter.

**Akzeptanzkriterien**
- Ein Wechsel ohne End-/Startstand zum selben Zeitpunkt wird abgelehnt bzw. als Lücke gemeldet.
- Historische Ablesungen bleiben dem alten Gerät zugeordnet.
```

```{req} Zählerüberlauf
:id: REQ_MTR_006
:status: accepted
:priority: should
:release: MVP

Das System soll den Überlauf eines Zählwerks (z.B. 99999 → 00012) anhand der
konfigurierten Vorkommastellen erkennen und korrekt als Verbrauch werten.
```

```{req} Zähler archivieren
:id: REQ_MTR_007
:status: accepted
:priority: could
:release: v1

Das System kann Messstellen archivieren. Archivierte Messstellen erscheinen nicht
mehr in Erfassung und Dashboard, bleiben aber in Auswertungen vergangener
Zeiträume enthalten.
```

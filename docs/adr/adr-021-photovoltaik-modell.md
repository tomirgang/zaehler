# ADR-021: PV-Anlage als Entität mit Messstellen-Zuordnung

```{adr} PV-Anlage mit zugeordneten Messstellen, Vergütung über das Tarifmodell
:id: ADR_021
:status: accepted
:decides: REQ_PV_001, REQ_PV_002, REQ_PV_009, SPEC_PV_ASSIGNMENT, SPEC_PV_FEED_IN

**Datum:** 2026-10-15
```

## Kontext
PV-Kennzahlen verknüpfen mehrere Zähler: Zweirichtungszähler des
Hausanschlusses, Erzeugungszähler und Speicher. Erfassung, Plausibilität und
Verbrauchsberechnung der einzelnen Zähler unterscheiden sich nicht von
anderen Stromzählern. Die Einspeisevergütung folgt denselben Mustern wie
Strompreise (Satz je kWh, Gültigkeit, Staffelung nach Leistungsanteil).

## Entscheidung
- Keine neuen Zählerarten. Erzeugungs- und Speicherzähler sind Messstellen der
  Art Strom; ein Speicher hat die Register `charge` (Bezug) und `discharge`
  (Einspeisung).
- Neue Entität `pv_systems` mit Anlagendaten und Zuordnungstabelle
  `pv_system_meters` mit Rolle `grid`, `generation` oder `battery`.
- PV-Kennzahlen entstehen in einer eigenen Auswertung über die Zuordnung aus
  den Verbrauchsreihen der Messstellen.
- Die Einspeisevergütung ist ein Vertrag des Tarifmodells ({need}`ADR_008`) am
  Einspeiseregister: Kostenpositionen mit negativem Arbeitspreis; anteilige
  Sätze als mehrere Positionen mit Umrechnungsfaktor = Anteil der Leistung.

## Alternativen
- *Zählerarten `pv_generation` und `battery`*: Sonderfälle in Erfassung,
  Plausibilität und Auswertung aller Messstellen.
- *Eigenes Vergütungsmodell an der Anlage*: zweite Preislogik neben dem
  Tarifmodell.

## Konsequenzen
- Ein Hausanschluss kann zu mehreren Anlagen gehören; Kennzahlen werden je
  Hausanschluss aggregiert, der spezifische Ertrag je Anlage.
- Zeiten negativer Börsenpreise sind ohne Stundenwerte nicht abbildbar.

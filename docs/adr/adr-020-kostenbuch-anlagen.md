# ADR-020: Gemeinsames Kostenbuch für Fahrzeuge und Anlagen

```{adr} Ein Kostenbuch für Fahrzeuge und PV-Anlagen
:id: ADR_020
:status: accepted
:decides: REQ_CAR_002, REQ_PV_011, SPEC_PV_ECONOMICS

**Datum:** 2026-10-15
```

## Kontext
Fahrzeuge ({need}`REQ_CAR_002`) und PV-Anlagen ({need}`REQ_PV_011`) benötigen
dieselbe Struktur: einmalige und wiederkehrende Kosten mit Kategorie, Beleg und
Auswertung je Jahr und über die Nutzungsdauer.

## Entscheidung
- Eine Tabelle `asset_costs` mit `asset_type` (`vehicle`, `pv_system`) und
  `asset_id`; Kategorien je Anlagentyp (Fahrzeug: wie {need}`REQ_CAR_002`;
  PV-Anlage: `investment`, `replacement`, `insurance`, `maintenance`,
  `meter_rent`, `other`).
- Gemeinsame Erfassungsseiten, Belegerkennung ({need}`REQ_AI_004`) und
  Jahresauswertung; anlagenspezifische Kennzahlen (Kosten pro km, Bilanz und
  Amortisation) bleiben getrennte Auswertungen.

## Alternativen
- *Getrennte Kostenbücher*: doppelte Tabellen, Seiten und Belegverarbeitung.

## Konsequenzen
- Die geplante Tabelle `vehicle_costs` entfällt zugunsten von `asset_costs`.
- `CostCategory` im Domain-Crate wird je Anlagentyp aufgeteilt.

# ADR-007: Lineare Verteilung plus gelernte Saisonprofile

```{adr} Verbrauchsverteilung linear mit Saisonprofil
:id: ADR_007
:status: accepted
:decides: REQ_ANA_001, REQ_ANA_002, REQ_FC_001, SPEC_CONS_INTERPOLATION, SPEC_FC_SEASON_PROFILE, SPEC_FC_PERIOD_FORECAST

**Datum:** 2026-10-15
```

## Kontext
Ablesungen erfolgen unregelmäßig. Für Monatsvergleiche und Jahresprognosen muss
Verbrauch auf Kalenderzeiträume verteilt werden. Der Heizenergieverbrauch hängt von der Jahreszeit ab.

## Entscheidung
- Innerhalb gemessener Intervalle: **lineare** Verteilung (sekundengenau).
- Für Hochrechnungen: **Saisonprofil** mit 12 Monatsgewichten, gelernt aus der
  eigenen Historie, sonst Standardprofil je Zählerart.
- Später: Saisonprofil auch innerhalb langer Intervalle (> 45 Tage) anwenden.

## Alternativen
- *Nur echte Ablesezeitpunkte*: keine Monatswerte, wenn Ablesungen nicht auf
  Monatsgrenzen fallen.
- *Gradtagzahlen (Wetterdaten)*: berücksichtigt die Außentemperatur, erfordert eine
  externe Datenquelle; als spätere Option offen.

## Konsequenzen
- Prognosen für Heizenergie haben im ersten Jahr eine höhere Abweichung; der
  Abdeckungsgrad wird mit angezeigt.

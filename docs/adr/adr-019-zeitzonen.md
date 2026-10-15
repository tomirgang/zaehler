# ADR-019: Zeiträume in der Zeitzone des Haushalts

```{adr} UTC-Speicherung, Kalenderzeiträume in der Zeitzone des Haushalts
:id: ADR_019
:status: accepted
:decides: REQ_ANA_001, REQ_RDG_001, SPEC_TZ

**Datum:** 2026-10-15
```

## Kontext
Monats- und Jahreswerte hängen davon ab, wann ein Tag beginnt. Server und
Container laufen in UTC; Haushalte liegen in einer lokalen Zeitzone mit
Sommerzeit.

## Entscheidung
- Zeitpunkte werden in UTC gespeichert (`timestamptz`).
- Jeder Haushalt hat eine IANA-Zeitzone (Standard `Europe/Berlin`).
- `zaehler_domain::calendar::Calendar` (mit `chrono-tz`) bildet Kalendertage
  auf UTC-Zeitpunkte ab; alle Zeiträume der Auswertung entstehen darüber.

## Alternativen
- *Alles in UTC*: Monatsgrenzen um eine bzw. zwei Stunden verschoben.
- *Zeitzone je Nutzer*: unterschiedliche Monatswerte für denselben Haushalt.

## Konsequenzen
- `zaehler-domain` erhält die Abhängigkeit `chrono-tz` (Zeitzonendaten, kein IO).
- Tests decken die Tage der Zeitumstellung ab.

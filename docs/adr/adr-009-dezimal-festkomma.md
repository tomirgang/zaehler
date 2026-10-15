# ADR-009: Dezimalarithmetik, Festkomma-Speicherung, Brutto-EUR

```{adr} Decimal im Code, Festkomma-Integer in der DB, Bruttopreise
:id: ADR_009
:status: accepted
:decides: REQ_NFR_005, REQ_TAR_010, SPEC_STORE_FIXED_POINT

**Datum:** 2026-10-15
```

## Kontext
Geld und Zählerstände dürfen keine Rundungsfehler durch Gleitkommazahlen
haben. SQLite kennt keinen exakten `NUMERIC`-Typ mit Skala; SeaORM bildet
`Decimal` dort nicht verlustfrei ab.

## Entscheidung
- Im Code: `rust_decimal::Decimal`.
- In der DB: `BIGINT` mit fester Skala: Mengen 10⁻³ (`_milli`), Geld/Preise
  10⁻⁶ (`_micro`). Zu genaue Werte werden abgewiesen.
- Preise werden **brutto in EUR** erfasst; MwSt nur optional zur Anzeige.

## Alternativen
- *`NUMERIC` in Postgres + `TEXT` in SQLite*: unterschiedliche Typen je Backend,
  keine Sortierung/Aggregation in SQLite.
- *Netto + MwSt-Sätze mit Gültigkeit*: bildet Steueränderungen ab, erfordert
  aber die Pflege von MwSt-Sätzen; Privatrechnungen weisen Bruttopreise aus.

## Konsequenzen
- Konvertierung an der Repository-Grenze (`zaehler_db::fixed`).
- Wertebereich: ±9,2·10¹² EUR bzw. ±9,2·10¹⁵ Einheiten.

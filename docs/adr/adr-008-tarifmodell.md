# ADR-008: Tarifmodell Vertrag → Kostenposition → Preisstand

```{adr} Tarifmodell mit Verträgen, Kostenpositionen und Preisständen
:id: ADR_008
:status: accepted
:decides: REQ_TAR_001, REQ_TAR_002, REQ_TAR_003, REQ_TAR_004, REQ_TAR_005, REQ_TAR_007, REQ_TAR_008, SPEC_TAR_COST_CALC

**Datum:** 2026-10-15
```

## Kontext
Reale Abrechnungen kombinieren Grundpreise in verschiedenen Intervallen,
Arbeitspreise, Staffeln, Boni, mehrere Positionen (Wasser/Abwasser) und
Preisänderungen mitten im Abrechnungsjahr.

## Entscheidung
- **Vertrag** (Messstelle, Anbieter, Abrechnungszeitraum) →
  **Kostenpositionen** (mit Umrechnungsfaktor) →
  **Preisstände** mit `valid_from`, Grundpreis + Intervall und
  `UnitPricing = Flat | Tiered | Dynamic`.
- **Anpassungen** (Bonus, Preisdeckel) und **Abschläge** hängen am Vertrag.
- Berechnung tagesgenau, Zeitraum wird an jedem Preiswechsel geteilt.

## Alternativen
- *Ein Grund- und Arbeitspreis je Zähler*: deckt Wasser/Abwasser und
  Preisänderungen nicht ab.
- *Freie Formeln*: beliebige Preisregeln; Validierung und Nachvollziehbarkeit
  der Berechnung nicht sichergestellt.

## Konsequenzen
- `UnitPricing` wird als JSON in `price_components` gespeichert (beide DBs).
- UI braucht Vorlagen (Strom, Gas, Wasser) zur Vorbelegung typischer Positionen.

# Berechnungen (Fachlogik)

Alle Berechnungen liegen im Crate `zaehler-domain` und arbeiten mit
`rust_decimal::Decimal` ({need}`ADR_009`).

## Zähler und Verbrauch

```{spec} Zählerarten und Standardeinheiten
:id: SPEC_MTR_KINDS
:status: implemented
:satisfies: REQ_MTR_001, REQ_MTR_002

Arten: `electricity` (kWh), `gas` (m³), `water` (m³), `heat` (kWh),
`vehicle` (km), `custom` (freie Einheit). Die Art steuert Standardeinheit,
Standard-Saisonprofil und verfügbare Auswertungen.
```

```{spec} Verbrauch zwischen zwei Ablesungen inkl. Überlauf
:id: SPEC_CONS_DELTA
:status: implemented
:satisfies: REQ_ANA_001, REQ_MTR_006

Für zwei Stände *a* < *b* (zeitlich) desselben Geräts gilt
`Δ = b - a`. Ist `b < a` und hat das Gerät *n* Vorkommastellen, gilt
`Δ = b + 10ⁿ - a` (Überlauf). Ohne Vorkommastellen ist `b < a` ein Fehler.
```

```{spec} Zählerwechsel in Verbrauchsreihen
:id: SPEC_CONS_DEVICE_SWAP
:status: implemented
:satisfies: REQ_MTR_005

Ablesungen einer Messstelle werden chronologisch zu Intervallen verbunden.
Wechselt zwischen zwei Ablesungen das Gerät, müssen beide denselben Zeitpunkt
haben (Endstand alt = Startstand neu); das Intervall hat die Länge 0 und
trägt keinen Verbrauch. Unterschiedliche Zeitpunkte sind ein Fehler
(`DeviceSwapGap`).
```

```{spec} Lineare Verteilung auf Zeiträume
:id: SPEC_CONS_INTERPOLATION
:status: implemented
:satisfies: REQ_ANA_001

Der Verbrauch eines Zeitraums `[von, bis)` ist die Summe der Intervallanteile:
`Δᵢ · |Überlappung| / |Intervall|` (sekundengenau). Ist der Zeitraum nicht
vollständig durch Intervalle abgedeckt, ist das Ergebnis *undefiniert* und es
wird stattdessen eine Prognose verwendet. Saisonale Verteilung innerhalb eines
Intervalls folgt mit {need}`SPEC_FC_SEASON_PROFILE` (v1).
```

```{spec} Plausibilitätsprüfung neuer Stände
:id: SPEC_RDG_PLAUSIBILITY
:status: implemented
:satisfies: REQ_RDG_002, REQ_AI_003

Prüfung gegen den letzten Stand desselben Geräts in dieser Reihenfolge:
`OutOfOrder` (Zeitpunkt früher), `Decreasing` (Wert kleiner, kein Überlauf
möglich), `RateTooHigh` (Tagesrate > `max_daily_rate`), sonst `Ok`. Das
Ergebnis ist ein Hinweis; Speichern nach Bestätigung bleibt möglich.
```

## Vergleiche und Erwartung

```{spec} Vergleichszeiträume
:id: SPEC_CMP_PERIODS
:status: implemented
:satisfies: REQ_ANA_003, REQ_ANA_004, REQ_ANA_005

Für einen Kalendermonat *M* gilt: Vormonat = *M*-1; Vorjahr = *M*-12;
gleitender Durchschnitt *n* ∈ {3, 6, 12} = Monate *M*-1 … *M*-*n*.
Abrechnungsjahre beginnen an einem beliebigen Tag und dauern 12 Monate.
```

```{spec} Abweichung gegenüber Referenz
:id: SPEC_CMP_DIFF
:status: implemented
:satisfies: REQ_ANA_003, REQ_ANA_004, REQ_ANA_005

Referenz = arithmetisches Mittel der Referenzwerte. Ausgegeben werden
absolute Differenz und relative Differenz in Prozent (1 Nachkommastelle);
bei Referenz 0 entfällt die relative Angabe.
```

```{spec} Erwartungsbereich und Ampel
:id: SPEC_EXP_BAND
:status: implemented
:satisfies: REQ_ANA_006

*Historie* (≥ 2 Werte, z.B. gleicher Monat der Vorjahre): Erwartung = Mittelwert,
Band = Mittelwert ± k·σ (Stichproben-σ, Standard k = 1,5, untere Grenze ≥ 0).
*Ohne Historie*: Ziel ± Toleranz in %. Ampel: grün im Band, gelb bis zur
halben Halbbreite außerhalb, sonst rot.
```

## Prognose

```{spec} Saisonprofil
:id: SPEC_FC_SEASON_PROFILE
:status: implemented
:satisfies: REQ_ANA_002

Ein Profil besteht aus 12 Monatsgewichten; innerhalb eines Monats wird
gleichmäßig auf die Tage verteilt. Standard = Tage je Monat (Gleichverteilung).
Gelernt wird aus Monatsverbräuchen: Mittel je Kalendermonat über alle Jahre,
normiert. Fehlt ein Monat in der Historie, gilt die Gleichverteilung.
```

```{spec} Hochrechnung auf einen Zeitraum
:id: SPEC_FC_PERIOD_FORECAST
:status: implemented
:satisfies: REQ_FC_001, REQ_CAR_006

`Prognose = gemessen · Gewicht(Zeitraum) / Gewicht(gemessener Teil)`. Der
Abdeckungsgrad `Gewicht(gemessen) / Gewicht(Zeitraum)` wird mit ausgegeben.
Ohne gemessenen Anteil gibt es keine Prognose.
```

## Tarife und Kosten

```{spec} Tagesgenaue Umlage des Grundpreises
:id: SPEC_TAR_BASE_PRORATION
:status: implemented
:satisfies: REQ_TAR_003

Grundpreis für *d* Tage: Tagespreis · d; Monatspreis · 12 · d / 365;
Jahrespreis · d / 365. Erst multiplizieren, dann dividieren.
```

```{spec} Staffelpreise
:id: SPEC_TAR_TIERED
:status: implemented
:satisfies: REQ_TAR_004

Stufen sind durch exklusive Obergrenzen der kumulierten Menge im
Abrechnungszeitraum definiert. Für eine Teilmenge wird der bereits verbrauchte
Anteil berücksichtigt, sodass unterjährige Teilrechnungen konsistent summieren.
```

```{spec} Gesamtkosten eines Zeitraums
:id: SPEC_TAR_COST_CALC
:status: draft
:satisfies: REQ_TAR_002, REQ_TAR_005, REQ_TAR_010, REQ_FC_002

Kosten = Σ Positionen [ Σ Preisstände (Grundpreis anteilig + Menge · Faktor ·
Arbeitspreis) ] - Boni. Ein Zeitraum wird an jedem `valid_from` einer
Preiskomponente geteilt; die Menge je Teilstück stammt aus
{need}`SPEC_CONS_INTERPOLATION` bzw. für die Zukunft aus der Prognose.
```

```{spec} Abschläge und erwartete Abrechnung
:id: SPEC_TAR_INSTALLMENT_SETTLEMENT
:status: implemented
:satisfies: REQ_TAR_008, REQ_TAR_009

Fälligkeiten: `valid_from + k · Turnus`, bis zum nächsten Abschlagsstand.
Saldo = prognostizierte Kosten - gezahlte Abschläge (positiv = Nachzahlung).
Vorschlag neuer Monatsabschlag = prognostizierte Jahreskosten / 12, auf volle
Euro aufgerundet.
```

## Fahrzeug

```{spec} Kosten pro Kilometer
:id: SPEC_CAR_COST_PER_KM
:status: implemented
:satisfies: REQ_CAR_005, REQ_CAR_002

Variable Kategorien: Kraftstoff, Laden, Werkstatt, Reifen, Pflege, Parken.
Fix: Versicherung, Steuer, HU/AU, Finanzierung, Sonstiges.
`variabel/km = Σ variabel / km`, `Vollkosten/km = (Σ variabel + Σ fix +
Wertverlust) / km`, gerundet auf 4 Nachkommastellen. Lebenszeit = Zeitraum
Kauf bis heute bzw. Verkauf.
```

```{spec} Energiekosten von Elektrofahrzeugen und Plug-in-Hybriden
:id: SPEC_CAR_ENERGY_COST
:status: draft
:satisfies: REQ_CAR_007, REQ_CAR_008, REQ_CAR_009, REQ_CAR_010, REQ_CAR_011, REQ_CAR_003

Für einen Zeitraum *P*:

- *Extern:* `E_ext` = Σ kWh, `K_ext` = Σ Beträge der externen Ladevorgänge in *P*.
- *Arbeitspreis Hausstrom:* `p_haus` = Arbeitspreiskosten / Menge des
  Hausstromvertrags in *P* (Durchschnitt, berücksichtigt Preisstände und
  Staffeln; ohne Grundpreis, Boni und Einspeisung).
- *Ladepreis:* ohne PV am Hausanschluss `p_lade = p_haus`. Mit PV
  `p_lade = s · v_ein + (1 - s) · p_haus` mit PV-Anteil *s* (Standard:
  Autarkiegrad des Hausanschlusses in *P*, {need}`SPEC_PV_KPI`; überschreibbar
  durch einen am Fahrzeug hinterlegten PV-Anteil) und durchschnittlicher
  Einspeisevergütung `v_ein` in *P*. Ein Ladepreis mit PV ist als Schätzung
  gekennzeichnet.
- *Heimladen mit Zwischenzähler:* `E_heim` = Verbrauch des Zwischenzählers in
  *P* ({need}`SPEC_CONS_INTERPOLATION`), `K_heim = E_heim · p_lade`.
- *Heimladen ohne Zwischenzähler (Schätzung):*
  `E_bedarf = km(P) · a · v / 100 · (1 + l / 100)` mit Durchschnittsverbrauch
  *v* (kWh/100 km, gültig nach `valid_from`, bei Wechsel in *P* tagesanteilig),
  Ladeverlust *l* (%) und elektrischem Fahranteil *a* (Elektro: 1,
  Plug-in-Hybrid: hinterlegter Anteil, ohne Angabe keine Schätzung).
  `E_heim = max(0, E_bedarf - E_ext)`, `K_heim = E_heim · p_lade`; Ergebnis
  als Schätzung gekennzeichnet.
- `K_heim` und `K_ext` gehen als variable Kosten der Kategorie `charging` in
  {need}`SPEC_CAR_COST_PER_KM` ein. kWh/100 km = `(E_heim + E_ext) / km(P) · 100`,
  nur ausgewiesen, wenn `E_heim` gemessen ist.
- Hausstrom ohne Fahrzeuganteil: Verbrauch der Hausstrom-Messstelle minus
  Verbrauch aller zugeordneten Zwischenzähler im selben Zeitraum.
```

```{spec} Linearer Wertverlust
:id: SPEC_CAR_DEPRECIATION
:status: implemented
:satisfies: REQ_CAR_004

`(Kaufpreis - Restwert) · Tage(Zeitraum ∩ Haltedauer) / Tage(Haltedauer)`.
Bei Verkauf ersetzt der Verkaufserlös den geschätzten Restwert.
```

## Photovoltaik

Bezeichner je Hausanschluss und Zeitraum *P*: Erzeugung *G* (Summe der
Messstellen mit Rolle `generation` aller Anlagen am Hausanschluss),
Einspeisung *E* und Bezug *B* (Register der Messstelle mit Rolle `grid`),
Speicherladung *L* und Speicherentladung *U* (Register `charge` und
`discharge` der Messstellen mit Rolle `battery`). Alle Größen über
{need}`SPEC_ANA_MONTHLY_SERIES` bzw. {need}`SPEC_CONS_INTERPOLATION`; eine
Kennzahl ist undefiniert, sobald eine beteiligte Größe undefiniert ist.

```{spec} Energiekennzahlen
:id: SPEC_PV_KPI
:status: draft
:satisfies: REQ_PV_004, REQ_PV_006, REQ_MTR_004

- Eigenverbrauch `EV = G - E - (L - U)`, bei Volleinspeisung `EV = 0`.
  Ohne Einspeiseregister gilt `E = 0` (Annahme, gekennzeichnet).
- Gesamtverbrauch `V = B + EV`.
- Eigenverbrauchsquote `EV / G`, Autarkiegrad `EV / V` (bei Nenner 0 undefiniert).
- Spezifischer Ertrag je Anlage `G_anlage / kWp`.
- Ein negatives `EV` (Ablese- oder Zuordnungsfehler) wird nicht auf 0
  begrenzt, sondern als Plausibilitätswarnung angezeigt.
- Ampel und Vergleiche des Hausanschlusses verwenden *V*, Kosten *B*
  ({need}`REQ_PV_006`).
```

```{spec} Speicherkennzahlen
:id: SPEC_PV_BATTERY
:status: draft
:satisfies: REQ_PV_005

Speicherverluste `L - U`, Wirkungsgrad `U / L` (bei `L = 0` undefiniert),
Vollzyklen `U / nutzbare Kapazität`. Netzladung wird nicht getrennt; der
Hinweis darauf erscheint bei jeder Speicherkennzahl.
```

```{spec} Ertragserwartung
:id: SPEC_PV_YIELD_EXPECTATION
:status: draft
:satisfies: REQ_PV_007
:links: SPEC_FC_SEASON_PROFILE, SPEC_FC_PERIOD_FORECAST

*Startwert* (weniger als ein vollständiges Kalenderjahr Messwerte):
`Erwartung(Monat m, Jahr j) = kWp · spezifischer Jahresertrag · w_m · (1 - d / 100)^(j - j0)`
mit Inbetriebnahmejahr *j0*, Degradation *d* und Monatsanteilen *w_m*.
Standard-Monatsanteile (Richtwerte für Deutschland, je Anlage überschreibbar,
Summe 100 %): Jan 3,0; Feb 4,5; Mär 8,0; Apr 11,5; Mai 13,0; Jun 13,5;
Jul 13,5; Aug 12,0; Sep 9,0; Okt 6,0; Nov 3,5; Dez 2,5.
*Historie* (ab einem vollständigen Kalenderjahr): Saisonprofil und
Jahresniveau aus den eigenen Monatswerten ({need}`SPEC_FC_SEASON_PROFILE`);
die Degradation ist darin enthalten und wird nicht zusätzlich angewendet.
Jahresprognose der Erzeugung über {need}`SPEC_FC_PERIOD_FORECAST` mit diesem
Profil.
```

```{spec} Ertragsüberwachung
:id: SPEC_PV_MONITORING
:status: draft
:satisfies: REQ_PV_008
:links: SPEC_EXP_BAND

Ampel je Anlage und Monat: mit Startwert grün bei Abweichung bis ±30 % von der
Monatserwartung, gelb bis ±45 %, sonst rot; mit Historie Band gemäß
{need}`SPEC_EXP_BAND`. Alarm bei einer neuen Ablesung des Erzeugungszählers,
wenn die Tagesrate seit der vorherigen Ablesung kleiner als
`Schwellwert · Erwartung des Zeitraums / Tage` ist (Schwellwert Standard 50 %);
höchstens ein Alarm je Anlage und Ablesung.
```

```{spec} Einspeisevergütung
:id: SPEC_PV_FEED_IN
:status: draft
:satisfies: REQ_PV_009
:links: SPEC_TAR_COST_CALC, ADR_021

Vertrag am Einspeiseregister der Messstelle mit Rolle `grid`. Jede
Kostenposition hat einen negativen Arbeitspreis (Vergütung je kWh) mit
`valid_from`; anteilige Sätze bilden mehrere Positionen mit Umrechnungsfaktor
gleich dem Leistungsanteil (z.B. 10 kWp von 15 kWp: Faktor 0,6667 und 0,3333).
Die Vergütung eines Zeitraums ist `-Kosten` gemäß {need}`SPEC_TAR_COST_CALC`.
Die durchschnittliche Vergütung `v_ein = Vergütung / E`.
```

```{spec} Wirtschaftlichkeit von PV-Anlagen
:id: SPEC_PV_ECONOMICS
:status: draft
:satisfies: REQ_PV_010, REQ_PV_011
:links: ADR_020

- Ersparnis `S = EV · p_haus` mit dem durchschnittlichen Arbeitspreis des
  Bezugsvertrags `p_haus` ({need}`SPEC_CAR_ENERGY_COST`).
- Vorteil Eigenverbrauch `EV · (p_haus - v_ein)`.
- Bei mehreren Anlagen an einem Hausanschluss werden *S* und Vergütung nach
  dem Anteil der Erzeugung `G_anlage / G` auf die Anlagen verteilt.
- Bilanz je Jahr = Vergütung + *S* - Kosten des Jahres aus `asset_costs`;
  kumulierte Bilanz seit Inbetriebnahme.
- Amortisationszeitpunkt: erster Tag, an dem die kumulierte Bilanz ≥ 0 ist;
  liegt er in der Zukunft, Fortschreibung je Monat mit Ertragserwartung
  ({need}`SPEC_PV_YIELD_EXPECTATION`), Eigenverbrauchsquote der letzten
  12 Monate und den aktuell gültigen Preisen.
- Einfache Rendite p.a. = durchschnittliche jährliche Bilanz ohne Investition
  / Summe der Investitionen (Kategorien `investment` und `replacement`).
```

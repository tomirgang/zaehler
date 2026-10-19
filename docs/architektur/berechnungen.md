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

Prüfung gegen die zeitlichen Nachbarn desselben Geräts zum Zeitpunkt des
neuen Stands: zuerst der neue Stand gegen den Vorgänger, danach der
Nachfolger gegen den neuen Stand. Nachgetragene Zwischenstände werden so wie
beim CSV-Import geprüft. Je Paar gilt diese Reihenfolge:
`OutOfOrder` (Zeitpunkt vor dem Einbau des Geräts),
`Decreasing` (Wert kleiner und kein Überlauf möglich; mit Vorkommastellen *n*
gilt ein Überlauf nur als plausibel, wenn er weniger als 10ⁿ/2 Einheiten
ergibt), `RateTooHigh` (Tagesrate des Verbrauchs, bei Überlauf inklusive
Überlauf, größer als `max_daily_rate`), sonst `Ok`. Das Ergebnis ist ein
Hinweis; Speichern nach Bestätigung bleibt möglich. Liegt ein bestätigter
Stand vor dem Einbau des ersten Geräts, wird dessen Einbau auf diesen
Zeitpunkt vorverlegt (wie beim CSV-Import); vor dem Einbau eines späteren
Geräts, also vor einem Zählerwechsel, ist das Speichern unzulässig.
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
gemessene Teil ist ein Abschnitt des Zeitraums; fehlende Abschnitte davor
und danach erhalten dieselbe Rate. Der Abdeckungsgrad
`Gewicht(gemessen) / Gewicht(Zeitraum)` wird mit ausgegeben. Ohne
gemessenen Anteil gibt es keine Prognose.
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
:status: implemented
:satisfies: REQ_TAR_002, REQ_TAR_005, REQ_TAR_010, REQ_FC_002

Kosten = Σ Positionen [ Σ Preisstände (Grundpreis anteilig + Menge · Faktor ·
Arbeitspreis) ] - Boni. Ein Zeitraum wird an jedem `valid_from` einer
Preiskomponente geteilt; die Menge je Teilstück stammt aus
{need}`SPEC_CONS_INTERPOLATION` bzw. für die Zukunft aus der Prognose.

Umsetzung (`zaehler_domain::cost`): Geteilt wird zusätzlich an den Grenzen
der Abrechnungszeiträume, an Vertragsbeginn und -ende und am Ende des
gemessenen Abschnitts. Die Interpolation rechnet in lokaler Kalenderzeit
(jeder Tag gleich lang). Staffeln beziehen sich auf die kumulierte Menge ab
Beginn des Abrechnungszeitraums. Golden-Tests für Strom, Gas und Wasser
vergleichen je Position auf 1 Cent; Property-Test zur Additivität
angrenzender Zeiträume.
```

```{spec} Abschläge und erwartete Abrechnung
:id: SPEC_TAR_INSTALLMENT_SETTLEMENT
:status: implemented
:satisfies: REQ_TAR_008, REQ_TAR_009

Fälligkeiten: `valid_from + k · Turnus`, bis zum nächsten Abschlagsstand.
Saldo = prognostizierte Kosten - gezahlte Abschläge (positiv = Nachzahlung).
Vorschlag neuer Monatsabschlag = prognostizierte Jahreskosten / 12, auf volle
Euro aufgerundet.

Umsetzung: Der Vorschlag gilt je Turnus des aktuellen Abschlags
(prognostizierte Jahreskosten · Turnus in Monaten / 12, aufgerundet); ohne
Abschlag monatlich. Abschläge, Boni und Gutschriften stehen in
`installments` und `adjustments` des Vertrags.
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
:status: implemented
:satisfies: REQ_CAR_007, REQ_CAR_008, REQ_CAR_009, REQ_CAR_010, REQ_CAR_011, REQ_CAR_003

Für einen Zeitraum *P*:

- *Extern:* `E_ext` = Σ kWh, `K_ext` = Σ Beträge der externen Ladevorgänge in *P*.
- *Arbeitspreis Hausstrom:* `p_haus` = Arbeitspreiskosten / Menge des
  Hausstromvertrags in *P* (Durchschnitt, berücksichtigt Preisstände und
  Staffeln; ohne Grundpreis, Boni und Einspeisung). Liefert die
  Preis-Messstelle keinen Arbeitspreis (kein Vertrag oder kein Verbrauch in
  *P*), gilt der am Fahrzeug hinterlegte Strompreis fürs Heimladen (€/kWh,
  optional); ohne beides bleiben die Kosten des Heimladens offen.
- *Ladepreis:* ohne PV am Hausanschluss `p_lade = p_haus`. Mit PV
  `p_lade = s · v_ein + (1 - s) · p_haus` mit PV-Anteil *s* (Standard:
  Autarkiegrad des Hausanschlusses in *P*, {need}`SPEC_PV_KPI`; überschreibbar
  durch einen am Fahrzeug hinterlegten PV-Anteil) und durchschnittlicher
  Einspeisevergütung `v_ein` in *P*. Ein Ladepreis mit PV ist als Schätzung
  gekennzeichnet.
- *Heimladen mit Zwischenzähler:* `E_heim` = Verbrauch des Zwischenzählers in
  *P* ({need}`SPEC_CONS_INTERPOLATION`), `K_heim = E_heim · p_lade`.
- *Heimladen ohne Zwischenzähler (Laden am Hausstrom, Schätzung aus den
  erfassten km):*
  `E_bedarf = km(P) · a · v / 100 · (1 + l / 100)` mit Durchschnittsverbrauch
  *v* (kWh/100 km, gültig nach `valid_from`, bei Wechsel in *P* tagesanteilig),
  Ladeverlust *l* (%) und elektrischem Fahranteil *a* (Elektro: 1,
  Plug-in-Hybrid: hinterlegter Anteil, ohne Angabe keine Schätzung).
  `E_heim = max(0, E_bedarf - E_ext)`: separat erfasste kWh externer
  Ladevorgänge werden abgezogen. `K_heim = E_heim · p_lade`; Ergebnis als
  Schätzung gekennzeichnet.
- `K_heim` und `K_ext` gehen als variable Kosten der Kategorie `charging` in
  {need}`SPEC_CAR_COST_PER_KM` ein. kWh/100 km = `(E_heim + E_ext) / km(P) · 100`,
  nur ausgewiesen, wenn `E_heim` gemessen ist.
- Hausstrom ohne Fahrzeuganteil: Verbrauch der Hausstrom-Messstelle minus
  Verbrauch aller zugeordneten Zwischenzähler im selben Zeitraum.

Umsetzung (`zaehler_domain::vehicle`): Die Heimlade-Messstelle ist ein
Wallbox-Zwischenzähler oder die Hausstrom-Messstelle selbst (dann
Schätzung). `p_haus` ist der durchschnittliche Arbeitspreis der
Bezugsverträge im Zeitraum. Der Mischpreis mit PV folgt mit WP_V1_18; bis
dahin gilt `p_lade = p_haus`. Profile vor dem ersten `valid_from` gelten
wie das erste Profil.
```

```{spec} Wertverlust nach Fahrzeugalter
:id: SPEC_CAR_DEPRECIATION
:status: implemented
:satisfies: REQ_CAR_004

Restwertkurve *r(a)* als Anteil des Neupreises nach Fahrzeugalter *a* in
Jahren (365,25 Tage), stückweise linear zwischen den Stützstellen
0 → 100 %, 1 → 77,5 %, 3 → 50 %, 5 → 42,5 %, 10 → 25 %; danach 2
Prozentpunkte je Jahr weniger, mindestens 10 %. Richtwerte: im ersten Jahr
20–25 % Wertverlust, nach drei Jahren rund 50 %, nach fünf Jahren 40–50 %
Restwert, nach zehn Jahren 20–30 %. Das Alter zählt ab der Erstzulassung
*z* (ohne Angabe der Kauftag *k*, also Neuwagen).

Kurvenwert relativ zum Kaufpreis *K*: `V(t) = K · r(a(t)) / r(a(k))`, damit
auch Gebrauchtwagen ab ihrem Alter beim Kauf weiter der Kurve folgen.
Kurvenverlust bis *t*: `L(t) = K - V(t)`.

Zeitmodus: Sind geschätzter Restwert *R* und Halteende *e* angegeben, wird die
Kurve so skaliert, dass sie *R* am Halteende genau trifft:
`Wertverlust[k, t) = (K - R) · L(min(t, e)) / L(e)`. Ohne beide Angaben gilt
die Kurve allein (`L(t)`); nur eine der beiden Angaben ist ungültig. Bei
Verkauf ersetzt der Verkaufserlös den Restwert, die Haltedauer endet mit dem
Verkauf. Wertverlust eines Zeitraums [*a*, *b*) ist die Differenz der
kumulierten Werte an *b* und *a*.
```

```{spec} Wertverlust nach Laufleistung
:id: SPEC_CAR_MILEAGE_DEPRECIATION
:status: implemented
:satisfies: REQ_CAR_012
:links: SPEC_CAR_DEPRECIATION, SPEC_CAR_COST_PER_KM, SPEC_FC_PERIOD_FORECAST

Bezeichner: Kaufpreis *K*, Kauftag *k*, Kilometerstand bei Kauf *s₀*,
Kilometerstand *s(t)* der Fahrzeug-Messstelle
({need}`SPEC_CONS_INTERPOLATION`), Kurvenwert *V(t)* aus
{need}`SPEC_CAR_DEPRECIATION`. Zur Restwertkurve kommt ein Zu- bzw. Abschlag
für die Laufleistung: je 10.000 km über bzw. unter der durchschnittlichen
Fahrleistung von 13.500 km je Jahr 1,5 % des Kaufpreises:

`Wert(t) = V(t) - K · 0,015 · ((s(t) - s₀) - 13.500 · Jahre(k, t)) / 10.000`,
begrenzt auf [0, *K*]; `Wertverlust[k, t) = K - Wert(t)`. Wer genau
durchschnittlich fährt, folgt der Kurve; Vielfahrer verlieren mehr,
Wenigfahrer weniger. Ein Zeitraum ohne Verkauf hat keinen negativen
Wertverlust (Monate mit wenig Fahrt).

Kilometerstand vor der ersten Ablesung: gleichmäßig zwischen *s₀* am Kauftag
und der ersten Ablesung verteilt, damit der Wertverlust lange vor der
Messung gekaufter Fahrzeuge auf die Jahre seit dem Kauf fällt.

Bei Verkauf am Tag *v* ist der Wertverlust ab dem Tag nach *v* `K - Erlös`;
der Unterschied zum Modellwert fällt am Verkaufstag an, damit die Summe über
die Haltedauer `K - Erlös` ergibt.

Restwert am Tag *t*: `K - Wertverlust[Kauf, t)`. Restlebensdauer, nur mit
optionaler Lebenslaufleistung *S* (absoluter Kilometerstand):
`(S - s(t)) / prognostizierte Jahresfahrleistung` in Jahren, Datum = *t* +
Restlebensdauer. Die Jahresfahrleistung stammt aus
{need}`SPEC_FC_PERIOD_FORECAST` über die letzten 12 Monate; ist sie
undefiniert (weniger als 90 Tage gemessener Zeitraum) oder 0, ist die
Restlebensdauer undefiniert. Im Zeitmodus ({need}`SPEC_CAR_DEPRECIATION`)
ist die Restlaufzeit `geplantes Ende - t`, höchstens 0. Geldbeträge werden
erst bei der Ausgabe gerundet ({need}`SPEC_STORE_FIXED_POINT`).
```

```{spec} Preis pro Liter
:id: SPEC_CAR_FUEL_PRICE
:status: implemented
:satisfies: REQ_CAR_013, REQ_CAR_014
:links: SPEC_CAR_FUEL_CONSUMPTION, SPEC_CAR_REPORT, SPEC_ASSET_COSTS

Erfassung: Von Betrag *B*, Menge *M* und Preis pro Liter *p* genügen zwei
Angaben. Ein Betrag ohne Menge und Literpreis bleibt zulässig; dann ist
kein einzelner Literpreis bekannt und nur die folgende Durchschnittsschätzung
möglich. Fehlt *B*, gilt `B = round(M · p, 2)`; fehlt *M*, gilt
`M = round(B / p, 3)`; fehlt *p*, gilt `p = round(B / M, 3)`. Sind alle drei
angegeben und ist `|B - M · p| > 0,01`, wird die Eingabe abgelehnt
(`asset_cost.price_mismatch`). Gespeichert wird *p* wie eingegeben bzw.
berechnet, damit ein eingegebener Literpreis nicht durch die Rundung von *B*
verfälscht wird.

Bekannter Literpreis eines Tankvorgangs: das gespeicherte *p*. Letzter
bekannter Literpreis: *p* des jüngsten Tankvorgangs mit *p*.
Bei v1-Bestandsdaten ohne gespeichertes *p*, aber mit Menge, wird
`round(B / M, 3)` verwendet; ein eingegebener Preis wird nie ersetzt.

Geschätzter Durchschnittspreis, falls kein Tankvorgang einen bekannten
Literpreis hat, über die letzten 3 Monate [*t* - 3 Monate, *t*):
`p̄ = Σ B(Tankvorgänge im Zeitraum) / (km(Zeitraum) · v / 100)` mit dem
Ø-Verbrauch *v* aus {need}`SPEC_CAR_FUEL_CONSUMPTION` über die letzten 12
Monate, ersatzweise dem gültigen Verbrauchsprofil für Kraftstoff (l/100 km,
neu neben dem Profil in kWh/100 km aus {need}`SPEC_CAR_ENERGY_COST`). Ohne *v*,
ohne Tankvorgang oder ohne gefahrene km im Zeitraum ist *p̄* undefiniert.

Prognose: `Kraftstoffkosten Restjahr = prognostizierte km Restjahr · v / 100 ·
p_prog` mit `p_prog` = letzter bekannter Literpreis, sonst *p̄*. Ist
`p_prog` geschätzt, ist die Prognose gekennzeichnet; ist *v* oder `p_prog`
undefiniert, entfällt die Kraftstoffkostenprognose. Die Kraftstoffkosten
ersetzen in den erwarteten Jahreskosten ({need}`SPEC_CAR_REPORT`) den
Kraftstoffanteil der variablen Kosten je km.

Verlauf: je Tankvorgang mit *p* ein Punkt (Datum, *p*); geschätzte Werte
werden nicht eingezeichnet.
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
:status: implemented
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
:status: implemented
:satisfies: REQ_PV_005

Speicherverluste `L - U`, Wirkungsgrad `U / L` (bei `L = 0` undefiniert),
Vollzyklen `U / nutzbare Kapazität`. Netzladung wird nicht getrennt; der
Hinweis darauf erscheint bei jeder Speicherkennzahl.
```

```{spec} Ertragserwartung
:id: SPEC_PV_YIELD_EXPECTATION
:status: implemented
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

Umsetzung: Domain `pv::start_expectation`, `pv::expectation` und
`pv::yield_months`. Die Jahresprognose ist die gemessene Erzeugung der
abgeschlossenen Monate plus die Erwartung der übrigen Monate des Jahres
(`pv::expectation_in`).
```

```{spec} Ertragsüberwachung
:id: SPEC_PV_MONITORING
:status: implemented
:satisfies: REQ_PV_008
:links: SPEC_EXP_BAND

Ampel je Anlage und Monat: mit Startwert grün bei Abweichung bis ±30 % von der
Monatserwartung, gelb bis ±45 %, sonst rot; mit Historie Band gemäß
{need}`SPEC_EXP_BAND`. Alarm bei einer neuen Ablesung des Erzeugungszählers,
wenn die Tagesrate seit der vorherigen Ablesung kleiner als
`Schwellwert · Erwartung des Zeitraums / Tage` ist (Schwellwert Standard 50 %);
höchstens ein Alarm je Anlage und Ablesung.

Umsetzung: `DispatchOutbox` prüft nach dem Abweichungsalarm die Anlagen des
Erzeugungszählers (`pv::yield_alarm`), Schlüssel
`pv_alarm:{system}:{reading}` in `notification_log`. Empfänger sind
Mitglieder mit dem Ereignistyp `alarm`; ein eigener Typ `pv_alarm` besteht
nicht.
```

```{spec} Einspeisevergütung
:id: SPEC_PV_FEED_IN
:status: implemented
:satisfies: REQ_PV_009
:links: SPEC_TAR_COST_CALC, ADR_021

Vertrag am Einspeiseregister der Messstelle mit Rolle `grid`. Jede
Kostenposition hat einen Arbeitspreis (Vergütung je kWh) mit `valid_from`; er
gilt immer als Gutschrift (`-|Preis|`), gleich ob er wie im Vertrag üblich
positiv oder negativ erfasst wurde. Ein Grundpreis bleibt Kosten. Anteilige
Sätze bilden mehrere Positionen mit Umrechnungsfaktor
gleich dem Leistungsanteil (z.B. 10 kWp von 15 kWp: Faktor 0,6667 und 0,3333).
Die Vergütung eines Zeitraums ist `-Kosten` gemäß {need}`SPEC_TAR_COST_CALC`.
Die durchschnittliche Vergütung `v_ein = Vergütung / E`. In der
Kostenübersicht erscheint die Einspeisung daher negativ und mindert den Saldo
(`Saldo = Bezug + Einspeisung`).
```

```{spec} Wirtschaftlichkeit von PV-Anlagen
:id: SPEC_PV_ECONOMICS
:status: implemented
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

Umsetzung: `PvSystemReport` rechnet je Monat; Kosten zählen am Monatsanfang,
Vergütung und Ersparnis verteilen sich linear über die Tage des Monats, daraus
ergibt sich der Amortisationstag (`pv::break_even`). Die Fortschreibung nutzt
die letzten 12 gemessenen Monate, frühestens ab Inbetriebnahme. Die Rendite
wird aus den gemessenen Monaten auf ein Jahr hochgerechnet. Der Ladepreis mit
PV ({need}`REQ_CAR_011`) ist `s · v_ein + (1 - s) · p_haus` mit Autarkiegrad
*s* und als Schätzung gekennzeichnet (`pv::charging_price`).
```

# Tarife, Kosten und Abschläge

```{req} Verträge
:id: REQ_TAR_001
:status: accepted
:priority: must
:release: v1

Das System muss je Messstelle Verträge verwalten: Anbieter, Vertragsnummer,
Beginn/Ende und Abrechnungszeitraum (beliebiger Starttag). Ein Anbieterwechsel
wird als neuer Vertrag abgebildet; Verträge einer Messstelle überlappen nicht.
```

```{req} Mehrere Kostenpositionen
:id: REQ_TAR_002
:status: accepted
:priority: must
:release: v1

Das System muss je Vertrag mehrere Kostenpositionen unterstützen (z.B.
Frischwasser, Schmutzwasser, Zählermiete, Messstellenbetrieb). Jede Position
hat einen Umrechnungsfaktor von Mess- auf Abrechnungseinheit (z.B. Gas m³ → kWh
über Brennwert × Zustandszahl).
```

```{req} Grund- und Arbeitspreis mit Gültigkeit
:id: REQ_TAR_003
:status: accepted
:priority: must
:release: v1

Das System muss Grundpreise (pro Tag, Monat oder Jahr) und Arbeitspreise (pro
Einheit) mit Gültigkeitsbeginn verwalten. Preisänderungen innerhalb eines
Abrechnungszeitraums werden tagesgenau berücksichtigt.
```

```{req} Staffelpreise
:id: REQ_TAR_004
:status: accepted
:priority: should
:release: v1

Das System soll Arbeitspreise in Stufen abhängig von der kumulierten Menge im
Abrechnungszeitraum unterstützen.
```

```{req} Boni und Gutschriften
:id: REQ_TAR_005
:status: accepted
:priority: should
:release: v1

Das System soll einmalige Boni (Neukunden-, Sofortbonus) und Gutschriften
erfassen und in Kostenprognose und Abrechnungsvergleich berücksichtigen.
```

```{req} Preisdeckel und Kontingente
:id: REQ_TAR_006
:status: accepted
:priority: could
:release: später

Das System kann Kontingente abbilden, für die ein abweichender Preis gilt
(z.B. Preisbremse: 80 % des Vorjahresverbrauchs zu gedeckeltem Preis).
```

```{req} Dynamische Tarife
:id: REQ_TAR_007
:status: accepted
:priority: could
:release: später

Das System kann dynamische Tarife über monatliche Durchschnittspreise plus
Aufschlag abbilden; für Prognosen zukünftiger Monate gilt ein Ersatzpreis.
```

```{req} Abschläge
:id: REQ_TAR_008
:status: accepted
:priority: must
:release: v1

Das System muss Abschläge mit Betrag, Turnus (monatlich, zweimonatlich,
quartalsweise …) und Gültigkeitsbeginn verwalten; Änderungen der Abschlagshöhe
werden historisiert.
```

```{req} Abschlag gegen Verbrauch und Prognose
:id: REQ_TAR_009
:status: accepted
:priority: must
:release: v1

Das System muss die Summe der Abschläge im Abrechnungszeitraum mit den
bisherigen und den prognostizierten Kosten vergleichen, die erwartete
Nachzahlung bzw. Erstattung ausweisen und einen neuen Abschlag
vorschlagen, der die prognostizierten Kosten deckt.
```

```{req} Bruttopreise in EUR
:id: REQ_TAR_010
:status: accepted
:priority: must
:release: v1

Das System muss Preise brutto in EUR erfassen und rechnen. Ein MwSt-Satz je
Position kann optional zur Netto-Anzeige hinterlegt werden.
```

```{req} Tatsächliche Abrechnung erfassen
:id: REQ_TAR_011
:status: accepted
:priority: should
:release: v1

Das System soll den Betrag der tatsächlichen Jahresabrechnung erfassen und mit
der Prognose vergleichen, um die Prognosegüte sichtbar zu machen.
```

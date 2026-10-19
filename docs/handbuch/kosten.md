# Kosten

Kosten berechnet die App aus den Zählerständen und den **Verträgen** einer
Messstelle. Ohne Vertrag zeigt sie nur Verbrauch.

## Vertrag anlegen

Auf der Seite einer Messstelle bietet der Abschnitt **Verträge** Vorlagen an,
z.B. „Vertrag anlegen: Strom“, „Vertrag anlegen: Einspeisung“ oder für Gas
mit Umrechnungsfaktor. Ein Vertrag hat:

- **Anbieter**, Vertragsnummer, Beginn und optional Ende,
- **Richtung**: Bezug oder Einspeisung (Einspeisevergütung),
- **Beginn des Abrechnungsjahres**, falls abweichend vom Vertragsbeginn,
- eine oder mehrere **Kostenpositionen** (z.B. Frischwasser und Abwasser),
  jeweils für alle Register oder ein bestimmtes Register (z.B. HT/NT), mit
  Umrechnungsfaktor (z.B. Gas: Brennwert × Zustandszahl).

Je Position gibt es **Preisstände** mit Gültigkeitsdatum: Grundpreis je Tag,
Monat oder Jahr und Arbeitspreis je Einheit, einheitlich oder als Staffel.
Preisänderungen trägt man als neuen Preisstand ein; die App rechnet jeden
Zeitraum mit dem jeweils gültigen Preis. Alle Preise sind Bruttopreise in
Euro. Bei der Einspeisevergütung kann der Preis positiv eingetragen werden –
er gilt immer als Gutschrift.

```{figure} bilder/vertrag.png
:alt: Vertragsseite mit Kostenposition, Preisstand und Abschlag
:width: 100%

Vertrag mit einer Kostenposition, Preisstand und Abschlägen.
```

Auf der Vertragsseite werden außerdem gepflegt:

- **Abschläge** mit Betrag, Turnus und Gültigkeit; eine neue Höhe gilt ab dem
  angegebenen Tag, frühere bleiben als Verlauf erhalten,
- **Boni und Gutschriften** (z.B. Neukundenbonus) mit Buchungsdatum,
- **Abrechnungen** des Versorgers, um die Prognose mit der tatsächlichen
  Abrechnung zu vergleichen.

## Kosten einer Messstelle

**Kosten** auf der Messstelle zeigt je Vertrag für das laufende
Abrechnungsjahr:

- gemessene und prognostizierte Kosten, Summe und Abdeckungsgrad,
- Menge und durchschnittlichen Arbeitspreis,
- Abschläge: fällig bis heute, noch fällig, **erwartete Nachzahlung bzw.
  Erstattung** und einen Vorschlag für den neuen Abschlag,
- aufgeklappt die Aufteilung nach Position und Preisstand.

```{figure} bilder/kosten-messstelle.png
:alt: Kosten einer Messstelle mit Bezug und Einspeisung
:width: 100%

Hausstrom: Bezug mit erwarteter Nachzahlung und Einspeisevergütung als
Gutschrift.
```

Zwischenzähler ohne eigenen Vertrag (Wärmepumpe, Wallbox) erhalten ihren
Anteil an den Arbeitspreiskosten der übergeordneten Messstelle, ohne
Grundpreis.

## Kostenübersicht des Haushalts

**Kosten** in der Navigation fasst alle Messstellen eines Kalenderjahres
zusammen. Die Einspeisevergütung erscheint negativ und mindert den
**Saldo**. Zwischenzähler sind in der übergeordneten Messstelle enthalten
und zählen nicht doppelt. Das Diagramm zeigt je Monat gemessene und
prognostizierte Kosten sowie die Einspeisung.

Darunter stehen die **Fahrzeugkosten** des Jahres bis heute je Fahrzeug:
km, variable und fixe Kosten, Wertverlust, Summe und Vollkosten je km. Sie
werden nicht zum Saldo addiert, weil das Laden zu Hause bereits in den
Stromkosten enthalten ist.

```{figure} bilder/kostenuebersicht.png
:alt: Kostenübersicht mit Messstellen, Monatsdiagramm und Fahrzeugen
:width: 100%

Kostenübersicht 2026 mit Saldo, Kosten je Monat und Fahrzeugkosten.
```

Mit den Schaltflächen oben wechselt man das Jahr.

## Rechnungen importieren

**Rechnung importieren** nimmt eine Rechnung des Versorgers oder einen Beleg
(Tanken, Laden, Werkstatt) als PDF oder Foto entgegen. Mit eingerichteter
KI-Erkennung schlägt die App Ablesungen, Preise, Abschlag und
Rechnungsbetrag bzw. die Daten des Belegs vor; übernommen wird erst nach
Prüfung. Ohne Erkennung trägt man die Werte von Hand ein, das Dokument bleibt
als Beleg gespeichert.

```{figure} bilder/dokumente.png
:alt: Seite Rechnung importieren
:width: 100%

Rechnung oder Beleg hochladen; offene Dokumente warten auf die Prüfung.
```

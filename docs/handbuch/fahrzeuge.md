# Fahrzeuge

Ein Fahrzeug ist eine Messstelle für den **Kilometerstand** mit zusätzlichen
Stammdaten, Kostenbuch und Auswertung.

## Fahrzeug anlegen

**Fahrzeuge → Fahrzeug anlegen** fragt Name, Kennzeichen, Modell,
**Antriebsart** (Verbrenner, Elektro, Plug-in-Hybrid), Kilometerstand und
Beginn der Erfassung ab. Für Elektro und Plug-in-Hybrid kommen hinzu:

- **Heimlade-Messstelle**: ein Wallbox-Zwischenzähler (gemessen) oder die
  Hausstrom-Messstelle (geschätzt aus km und Verbrauch),
- **PV-Anteil** beim Heimladen, falls bekannt,
- **Strompreis Heimladen** (€/kWh), der gilt, wenn für die Heimlade-Messstelle
  kein Vertrag mit Arbeitspreis vorliegt.

```{figure} bilder/fahrzeuge.png
:alt: Liste der Fahrzeuge
:width: 100%

Fahrzeuge mit Kennzeichen und Antriebsart.
```

Kilometerstände erfasst man wie andere Zählerstände: über **Stand erfassen**
auf der Übersicht, beim Tanken oder Laden mit „Kilometerstand als Ablesung
speichern“ oder per Foto des Tachos.

## Auswertung

Die Fahrzeugseite zeigt Stammdaten, Restwert und Restlebensdauer sowie die
**Auswertung** je Kalenderjahr und über die Haltedauer:

- gefahrene km (vor der ersten Ablesung gleichmäßig zwischen Kauf und erster
  Ablesung verteilt und als geschätzt gekennzeichnet),
- variable Kosten (Kraftstoff, Laden, Werkstatt, Reifen, Pflege, Parken),
  fixe Kosten (Versicherung, Steuer, HU/AU, Finanzierung, Sonstiges) und
  Wertverlust,
- **Kosten je km** variabel und als Vollkosten,
- Verbrauch in l/100 km (Vollbetankungsmethode) bzw. kWh/100 km,
- Energie und Kosten beim Laden extern und zu Hause.

Die **Prognose** rechnet die Fahrleistung des laufenden Jahres wie bei
anderen Zählern hoch und leitet daraus die erwarteten Jahreskosten ab.

```{figure} bilder/fahrzeug.png
:alt: Fahrzeugseite mit Wertverlust, Auswertung und Kosten je Jahr
:width: 100%

Elektroauto: Restwert, Auswertung je Jahr, Kosten je Jahr und Prognose.
```

## Kostenbuch

**Kostenbuch** listet alle Kosten eines Jahres. Ein Eintrag hat Datum,
Kategorie, Betrag und Notiz, optional einen Kilometerstand. Wiederkehrende
Kosten (z.B. Versicherung alle 12 Monate) trägt man einmal mit Turnus ein.
Beim **Tanken** genügen zwei von Betrag, Menge und Literpreis; die App
ergänzt den dritten Wert und zeigt den Verlauf des Literpreises. Mit
„vollgetankt“ berechnet sie den Verbrauch nach der Vollbetankungsmethode.

```{figure} bilder/kostenbuch.png
:alt: Kostenbuch eines Fahrzeugs
:width: 100%

Kostenbuch mit Tankvorgängen (Menge, Literpreis), Werkstatt, Steuer und
Versicherung.
```

Belege lassen sich auch als Foto oder PDF über **Beleg importieren**
erfassen (siehe {doc}`kosten`).

## Laden

Externe Ladevorgänge (Ladesäule, Arbeitgeber) werden mit Zeitpunkt, kWh,
Betrag, Ladeart und Anbieter erfasst; kostenloses Laden mit Betrag 0.

```{figure} bilder/fahrzeug-laden.png
:alt: Liste der Ladevorgänge
:width: 100%

Externe Ladevorgänge eines Elektroautos.
```

Das **Heimladen** ergibt sich:

- mit Wallbox-Zähler aus dessen Verbrauch,
- ohne Wallbox-Zähler aus den gefahrenen km und dem hinterlegten
  **Verbrauchsprofil** (kWh/100 km, Ladeverlust, beim Plug-in-Hybrid
  elektrischer Fahranteil) abzüglich der extern geladenen kWh.

Bewertet wird es mit dem Arbeitspreis des Hausstromvertrags, sonst mit dem
Strompreis am Fahrzeug; mit PV-Anlage als Mischpreis aus Einspeisevergütung
und Arbeitspreis.

## Anschaffung und Wertverlust

Im Abschnitt **Anschaffung** stehen Kaufdatum, Kaufpreis, **Erstzulassung**
(leer bedeutet Neuwagen beim Kauf) und optional Verkaufsdatum und -erlös.

```{figure} bilder/fahrzeug-anschaffung.png
:alt: Abschnitt Anschaffung mit Checkbox für Laufleistung
:width: 100%

Anschaffung eines Gebrauchtwagens mit Wertverlust nach Laufleistung.
```

Der Wertverlust folgt einer typischen **Restwertkurve** nach Fahrzeugalter:
nach einem Jahr 77,5 %, nach drei Jahren 50 %, nach fünf Jahren 42,5 %,
nach zehn Jahren 25 % des Neupreises, danach langsamer bis mindestens 10 %.
Gebrauchtwagen folgen der Kurve ab ihrem Alter beim Kauf.

- **Ohne Haken** bei „Wertverlust nach Laufleistung berechnen“ zählt nur das
  Alter. Sind geschätzter Restwert und geplantes Halteende angegeben, wird die
  Kurve so angepasst, dass sie diesen Restwert genau trifft.
- **Mit Haken** kommt die Laufleistung hinzu: je 10.000 km über bzw. unter
  13.500 km im Jahr 1,5 % des Kaufpreises mehr bzw. weniger Wertverlust.
  Dafür ist der Kilometerstand bei Kauf nötig; eine optionale
  Lebenslaufleistung ergibt die Restlebensdauer.

Nach einem Verkauf ersetzt der Verkaufserlös den geschätzten Restwert.

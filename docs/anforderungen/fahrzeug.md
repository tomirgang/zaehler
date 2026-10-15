# Fahrzeuge

```{req} Fahrzeug als Messstelle
:id: REQ_CAR_001
:status: accepted
:priority: must
:release: v1

Das System muss Fahrzeuge als Messstelle der Art „Fahrzeug“ mit Kilometerstand
führen (Kennzeichen, Modell, Antriebsart).
```

```{req} Antriebsart Verbrenner und Elektro
:id: REQ_CAR_007
:status: accepted
:priority: must
:release: v1

Das System muss je Fahrzeug die Antriebsart `Verbrenner`, `Elektro` oder
`Plug-in-Hybrid` führen. Die Antriebsart bestimmt die verfügbaren
Energiequellen: Verbrenner tanken, Elektrofahrzeuge laden, Plug-in-Hybride
tanken und laden. Verbrauchskennzahlen und Kosten werden je Energieart
(l/100 km, kWh/100 km) getrennt ausgewiesen.

**Akzeptanzkriterien**
- Für Verbrenner sind keine Ladevorgänge erfassbar, für Elektrofahrzeuge keine Tankvorgänge.
- Ein Wechsel der Antriebsart ist nach der ersten Kostenbuchung nicht mehr möglich.
```

```{req} Heimladen über Zwischenzähler
:id: REQ_CAR_008
:status: accepted
:priority: should
:release: v1

Das System soll einem Elektrofahrzeug oder Plug-in-Hybrid optional einen
Zwischenzähler (z.B. Wallbox-Zähler) zuordnen. Die Energie für Heimladen
ergibt sich aus dem Verbrauch dieses Zählers, die Kosten aus dem Ladepreis
({need}`REQ_CAR_011`) des Hausanschlusses, hinter dem der Zwischenzähler liegt.

**Akzeptanzkriterien**
- Der Zwischenzähler ist eine Messstelle der Art Strom mit Verweis auf die übergeordnete Hausstrom-Messstelle.
- Der Grundpreis des Hausstromvertrags wird dem Fahrzeug nicht zugerechnet.
- Die Auswertung der Hausstrom-Messstelle kann wahlweise mit oder ohne den Anteil des Zwischenzählers angezeigt werden.
```

```{req} Geschätzte Heimladekosten
:id: REQ_CAR_009
:status: accepted
:priority: should
:release: v1

Das System soll für Elektrofahrzeuge und Plug-in-Hybride ohne Zwischenzähler
die Heimladekosten schätzen: aus gefahrenen Kilometern, einem hinterlegten
Durchschnittsverbrauch (kWh/100 km, optional mit Ladeverlust in %), der extern
geladenen Energie ({need}`REQ_CAR_010`) und dem Arbeitspreis des
Hausstromvertrags.

**Akzeptanzkriterien**
- Geschätzte Werte sind in Auswertungen als Schätzung gekennzeichnet.
- Der Durchschnittsverbrauch ist mit Gültigkeitsbeginn änderbar (z.B. Winter-/Sommerwerte).
```

```{req} Ladepreis beim Heimladen mit PV
:id: REQ_CAR_011
:status: accepted
:priority: should
:release: v1

Das System soll den Preis für Heimladen ohne PV aus dem Arbeitspreis des
Hausstromvertrags bilden. Mit PV am Hausanschluss gilt ein Mischpreis aus
Einspeisevergütung und Arbeitspreis, gewichtet mit dem PV-Anteil; Standard
für den PV-Anteil ist der Autarkiegrad des Hausanschlusses im Zeitraum. Ein
bekannter PV-Anteil (z.B. aus der Wallbox) kann je Fahrzeug hinterlegt werden
und ersetzt den Standard.

**Akzeptanzkriterien**
- Ein Ladepreis mit PV ist als Schätzung gekennzeichnet.
- Ohne Einspeisevergütung (z.B. Steckersolargerät) geht der PV-Anteil mit Preis 0 ein.
```

```{req} Externes Laden erfassen
:id: REQ_CAR_010
:status: accepted
:priority: must
:release: v1

Das System muss externe Ladevorgänge (öffentliche Ladesäule, Arbeitgeber,
Fremdwallbox) mit Datum, Energie in kWh, Betrag, optional Kilometerstand,
Anbieter und Ladeart (AC/DC) erfassen; per Formular oder als Beleg per Foto
({need}`REQ_AI_004`). Kostenlose Ladevorgänge werden mit Betrag 0 erfasst.
```

```{req} Kostenbuch
:id: REQ_CAR_002
:status: accepted
:priority: must
:release: v1

Das System muss je Fahrzeug Kosten erfassen: Kraftstoff, Laden, Versicherung,
Steuer, Werkstatt, Reifen, HU/AU, Pflege, Parken, Finanzierung, Sonstiges;
jeweils einmalig oder wiederkehrend (z.B. Versicherung jährlich). Das
Kostenbuch wird mit den PV-Anlagen geteilt ({need}`ADR_020`).
```

```{req} Kraftstoff- und Energieverbrauch
:id: REQ_CAR_003
:status: accepted
:priority: should
:release: v1

Das System soll aus Tank-/Ladevorgängen mit Menge den Verbrauch in l/100 km
bzw. kWh/100 km berechnen (Vollbetankungsmethode). Bei Elektrofahrzeugen
zählen Heimladen (Zwischenzähler oder Schätzung) und externes Laden zusammen;
ohne Zwischenzähler wird kein gemessener kWh-Verbrauch ausgewiesen, sondern der
hinterlegte Durchschnittsverbrauch.
```

```{req} Wertverlust und Finanzierung
:id: REQ_CAR_004
:status: accepted
:priority: should
:release: v1

Das System soll Kaufpreis, geschätzten Restwert bzw. Verkaufserlös und
geplante Haltedauer erfassen und den Wertverlust linear auf die Haltedauer
verteilen. Leasing- und Kreditraten werden als Finanzierungskosten geführt.
```

```{req} Kosten pro Kilometer
:id: REQ_CAR_005
:status: accepted
:priority: must
:release: v1

Das System muss Kosten pro km je Kalenderjahr und über die gesamte Haltedauer
berechnen und anzeigen, getrennt nach variablen Kosten und Vollkosten
(inkl. Fixkosten und Wertverlust).
```

```{req} Fahrleistungsprognose
:id: REQ_CAR_006
:status: accepted
:priority: should
:release: v1

Das System soll die Jahresfahrleistung analog zur Verbrauchsprognose
hochrechnen und daraus die erwarteten Jahreskosten ableiten.
```

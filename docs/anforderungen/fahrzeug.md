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
Alternativ zur Haltedauer kann der Wertverlust nach Laufleistung verteilt
werden ({need}`REQ_CAR_012`).
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

```{req} Wertverlust nach Lebenslaufleistung
:id: REQ_CAR_012
:status: accepted
:priority: should
:release: v1.1

Das System soll je Fahrzeug wahlweise den Zeitmodus (geplantes Halteende und
geschätzter Restwert, {need}`REQ_CAR_004`) oder den Laufleistungsmodus führen.
Im Laufleistungsmodus werden Kaufpreis, Kilometerstand bei Kauf, eine
Lebenslaufleistung als absoluter Kilometerstand (z.B. 250.000 km) und ein
optionaler Endwert (Standard 0) erfasst. Der Wertverlust eines Zeitraums ist
`km im Zeitraum × (Kaufpreis - Endwert) / (Lebenslaufleistung - km bei Kauf)`,
gedeckelt auf die restliche Lebenslaufleistung.

Das System soll in beiden Modi den aktuellen Restwert (`Kaufpreis - Wertverlust
bis heute`) und die Restlebensdauer anzeigen. Im Laufleistungsmodus ist die
Restlebensdauer `(Lebenslaufleistung - aktueller km-Stand) / prognostizierte
Jahresfahrleistung` ({need}`REQ_CAR_006`), angezeigt in Jahren und als
voraussichtliches Datum; im Zeitmodus die Restlaufzeit bis zum Halteende.

**Akzeptanzkriterien**
- Der Kilometerstand bei Kauf ist im Laufleistungsmodus Pflicht und mit dem Startstand des Fahrzeugs vorbelegt; die Lebenslaufleistung muss größer sein.
- Der Modus ist jederzeit wechselbar; Wertverlust, Restwert und Kosten pro km ({need}`REQ_CAR_005`) werden danach neu berechnet.
- Bei Verkauf ersetzt der Erlös in beiden Modi den Rest- bzw. Endwert; im Laufleistungsmodus wird die Differenz zum Kaufpreis nach den bis zum Verkauf gefahrenen km verteilt.
- Ohne belastbare Fahrleistungsprognose wird keine Restlebensdauer angezeigt, sondern der Hinweis „zu wenig Daten“; der Restwert wird weiterhin angezeigt.
```

```{req} Preis pro Liter beim Tanken
:id: REQ_CAR_013
:status: accepted
:priority: should
:release: v1.1

Das System soll bei Tankvorgängen von Fahrzeugen, die tanken
({need}`REQ_CAR_007`), die Felder Betrag, Menge (l) und Preis pro Liter
anbieten. Zwei der drei Angaben genügen, die dritte wird berechnet. Der Preis
pro Liter wird mit drei Nachkommastellen gespeichert.

Das System soll den zuletzt bekannten Preis pro Liter mit Datum auf der
Fahrzeugdetailseite anzeigen und für die prognostizierten Kraftstoffkosten
verwenden: `prognostizierte km × Ø-Verbrauch / 100 × Preis pro Liter`
({need}`REQ_CAR_006`, {need}`REQ_CAR_003`).

Ist kein Preis pro Liter bekannt (Tankvorgänge ohne Menge), wird ein
Durchschnittspreis über die letzten 3 Monate geschätzt: `Tankkosten / (gefahrene
km × Ø-Verbrauch / 100)`. Der Ø-Verbrauch stammt aus der Vollbetankungsmethode,
ersatzweise aus einem hinterlegten Verbrauchsprofil.

**Akzeptanzkriterien**
- Sind alle drei Werte angegeben und weicht der Betrag um mehr als 1 Cent von `Menge × Preis pro Liter` ab, wird die Eingabe mit Validierungsfehler abgelehnt.
- Ein geschätzter Durchschnittspreis ist in Anzeige und Prognose als Schätzung gekennzeichnet.
- Ohne Verbrauchswert wird kein Durchschnittspreis geschätzt und keine Kraftstoffkostenprognose ausgewiesen.
```

```{req} Verlauf des Literpreises
:id: REQ_CAR_014
:status: accepted
:priority: should
:release: v1.1

Das System soll auf der Fahrzeugdetailseite den Verlauf des Preises pro Liter
als Diagramm zeigen: ein Punkt je Tankvorgang mit bekanntem Preis
({need}`REQ_CAR_013`) über die gesamte Haltedauer, mit wählbarem
Ausschnitt (Zoom) ({need}`REQ_ANA_007`).

**Akzeptanzkriterien**
- Geschätzte Durchschnittspreise werden nicht eingezeichnet.
- Das Diagramm hat eine Tabellenalternative ({need}`REQ_NFR_007`).
- Ein Verlauf des kWh-Preises beim Laden ist nicht Teil dieser Anforderung.
```

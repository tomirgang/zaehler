# Photovoltaik

Begriffe: {doc}`../einfuehrung/glossar`. Rechenvorschriften:
{doc}`../architektur/berechnungen` (Abschnitt Photovoltaik).

```{req} PV-Anlagen verwalten
:id: REQ_PV_001
:status: accepted
:priority: must
:release: v1

Das System muss je Haushalt beliebig viele PV-Anlagen verwalten: Name,
Anlagentyp (`Überschusseinspeisung`, `Volleinspeisung`, `Steckersolargerät`),
Leistung in kWp, Inbetriebnahmedatum, optional Ausrichtung (Azimut) und
Neigung, Wechselrichter, nutzbare Speicherkapazität in kWh, spezifischer
Jahresertrag in kWh/kWp und Degradation in % pro Jahr (Standard 0,5).
```

```{req} Messstellen einer PV-Anlage zuordnen
:id: REQ_PV_002
:status: accepted
:priority: must
:release: v1

Das System muss einer PV-Anlage Messstellen der Art Strom mit einer Rolle
zuordnen: `grid` (Zweirichtungszähler des Hausanschlusses, Pflicht),
`generation` (Erzeugungszähler oder Gesamtertrag des Wechselrichters, Pflicht)
und `battery` (Speicher, optional, Register `charge` mit Richtung Bezug und
`discharge` mit Richtung Einspeisung). Mehrere Anlagen dürfen denselben
Hausanschluss verwenden.

**Akzeptanzkriterien**
- Eine Messstelle mit Rolle `generation` oder `battery` gehört zu genau einer Anlage.
- Ohne Register mit Richtung Einspeisung am Hausanschluss sind nur Anlagen vom Typ `Steckersolargerät` zulässig.
```

```{req} Sammelerfassung je Hausanschluss
:id: REQ_PV_003
:status: accepted
:priority: should
:release: v1

Das System soll die Stände aller einem Hausanschluss zugeordneten Messstellen
(Zweirichtungszähler, Erzeugungszähler, Speicher) in einem Vorgang mit
gemeinsamem Zeitpunkt erfassen: per Formular oder durch Hochladen mehrerer
Fotos mit gemeinsamem Bestätigungsformular. Die Einzelerfassung je Messstelle
bleibt möglich.
```

```{req} Energiekennzahlen
:id: REQ_PV_004
:status: accepted
:priority: must
:release: v1

Das System muss je Hausanschluss und Zeitraum (Monat, Jahr, Abrechnungsjahr)
Erzeugung, Einspeisung, Bezug, Eigenverbrauch, Gesamtverbrauch,
Eigenverbrauchsquote und Autarkiegrad sowie je Anlage den spezifischen Ertrag
(kWh/kWp) ausweisen.

**Akzeptanzkriterien**
- Ohne Einspeiseregister gilt Eigenverbrauch = Erzeugung; die Kennzahlen sind als Annahme gekennzeichnet.
- Bei Volleinspeisung ist der Eigenverbrauch 0.
- Ohne Erzeugungszähler werden nur Bezug und Einspeisung ausgewiesen.
```

```{req} Speicherkennzahlen
:id: REQ_PV_005
:status: accepted
:priority: should
:release: v1

Das System soll für Speicher Ladung, Entladung, Speicherverluste,
Wirkungsgrad und Vollzyklen je Zeitraum ausweisen. Speicher, die aus dem Netz
laden, werden nicht gesondert behandelt; diese Einschränkung ist in der
Oberfläche vermerkt.
```

```{req} Hausstrom-Auswertung mit PV
:id: REQ_PV_006
:status: accepted
:priority: must
:release: v1

Das System muss bei einem Hausanschluss mit Erzeugungszähler den
Gesamtverbrauch neben dem Bezug auswerten. Vergleiche, Erwartungsbereich und
Ampel ({need}`REQ_ANA_006`) beziehen sich dann auf den Gesamtverbrauch, Kosten
und Kostenprognose auf den Bezug.
```

```{req} Ertragserwartung
:id: REQ_PV_007
:status: accepted
:priority: must
:release: v1

Das System muss je Anlage eine monatliche Ertragserwartung bilden: im ersten
vollständigen Jahr aus einer Standard-Monatsverteilung, dem spezifischen
Jahresertrag, der Leistung und der Degradation; ab einem vollständigen Jahr
eigener Messwerte aus der Historie. Daraus folgt die Jahresprognose der
Erzeugung.
```

```{req} Ertragsüberwachung
:id: REQ_PV_008
:status: accepted
:priority: should
:release: v1

Das System soll die Erzeugung je Anlage und Monat mit einer Ampel bewerten
und per E-Mail ({need}`REQ_NTF_004`) benachrichtigen, wenn die Tagesrate seit
der letzten Ablesung unter einem je Anlage konfigurierbaren Anteil der
saisonalen Erwartung liegt (Standard 50 %).
```

```{req} Einspeisevergütung
:id: REQ_PV_009
:status: accepted
:priority: must
:release: v1

Das System muss die Einspeisevergütung über das Tarifmodell
({need}`REQ_TAR_001` bis {need}`REQ_TAR_003`) abbilden: Vertrag am
Einspeiseregister, Vergütungssatz je kWh mit Gültigkeit, anteilige Sätze für
Leistungsanteile, Satz nach Ende der festen Vergütung. Zeiten negativer
Börsenpreise werden nicht berücksichtigt; diese Einschränkung ist dokumentiert.
```

```{req} Ersparnis durch Eigenverbrauch
:id: REQ_PV_010
:status: accepted
:priority: must
:release: v1

Das System muss je Zeitraum die Ersparnis durch Eigenverbrauch
(Eigenverbrauch × durchschnittlicher Arbeitspreis des Bezugsvertrags, ohne
Grundpreis) und den Vorteil des Eigenverbrauchs gegenüber Einspeisung
(Eigenverbrauch × (Arbeitspreis - Einspeisevergütung)) ausweisen.
```

```{req} Investition, Amortisation und Rendite
:id: REQ_PV_011
:status: accepted
:priority: should
:release: v1

Das System soll je Anlage Investition, laufende Kosten und
Ersatzinvestitionen im gemeinsamen Kostenbuch ({need}`ADR_020`) erfassen und
daraus die kumulierte Bilanz (Vergütung + Ersparnis - Kosten) je Jahr und
seit Inbetriebnahme, den prognostizierten Amortisationszeitpunkt und die
einfache Rendite pro Jahr berechnen.
```

```{req} PV-Darstellung und Dashboard
:id: REQ_PV_012
:status: accepted
:priority: should
:release: v1

Das System soll monatlich gestapelte Balken für Erzeugung (Eigenverbrauch,
Einspeisung) und Gesamtverbrauch (Eigenverbrauch, Bezug), Erzeugung gegen
Erwartung, die kumulierte Bilanz mit Amortisationspunkt sowie eine PV-Karte im
Dashboard (Erzeugung, Autarkiegrad, Eigenverbrauchsquote, Ampel) darstellen.
```

```{req} Ertragserwartung über PVGIS
:id: REQ_PV_013
:status: accepted
:priority: could
:release: später

Das System kann die monatliche Ertragserwartung optional über die
PVGIS-Schnittstelle der Europäischen Kommission ermitteln (Standort auf 0,1°
gerundet, Leistung, Ausrichtung, Neigung, Systemverluste).
```

```{req} Interner Zinsfuß
:id: REQ_PV_014
:status: accepted
:priority: could
:release: später

Das System kann für PV-Anlagen den internen Zinsfuß aus den Zahlungsströmen
des Kostenbuchs und den Erträgen berechnen.
```

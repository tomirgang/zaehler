# Oberfläche

```{req} Breadcrumb-Leiste
:id: REQ_UI_001
:status: accepted
:priority: must
:release: v1.1

Das System muss auf allen Seiten außer der Übersicht eine Breadcrumb-Leiste
zeigen. Wurzel ist „Übersicht“, der Pfad folgt der fachlichen Hierarchie
(z.B. `Übersicht › Messstellen › Hausstrom › Ablesung bearbeiten`).

**Akzeptanzkriterien**
- Alle Glieder außer dem letzten sind verlinkt; das letzte Glied trägt `aria-current="page"`.
- Die Leiste ist ein `<nav>` mit eigenem `aria-label` ({need}`REQ_NFR_007`).
- Auf kleinen Bildschirmen wird nur das übergeordnete Glied als Rücksprung angezeigt (z.B. „‹ Hausstrom“).
```

```{req} Burgermenü auf kleinen Bildschirmen
:id: REQ_UI_002
:status: accepted
:priority: must
:release: v1.1

Das System muss unterhalb einer Breite von 48rem die Navigation, den
Haushaltswechsel und das Abmelden in ein Menü hinter einer Menüschaltfläche
verlegen.

**Akzeptanzkriterien**
- Die Schaltfläche zeigt den Zustand über `aria-expanded`; Escape schließt das Menü und gibt den Fokus an die Schaltfläche zurück.
- Das Menü ist vollständig per Tastatur bedienbar ({need}`REQ_NFR_007`) und kommt ohne Inline-Skripte aus ({need}`REQ_NFR_003`).
- Ohne JavaScript bleibt die Navigation sichtbar und bedienbar.
```

```{req} Foto-Ansichtsseite
:id: REQ_UI_003
:status: accepted
:priority: should
:release: v1.1

Das System soll Fotos (Belegfotos von Ablesungen {need}`REQ_RDG_004`, Belege und
Rechnungen) auf einer eigenen Ansichtsseite statt als Rohbild in einem neuen Tab
öffnen. Die Seite zeigt das Foto mit Metadaten (Messstelle, Zeitpunkt, Wert bzw.
Belegdaten) und einer Schaltfläche „Schließen/Zurück“.

**Akzeptanzkriterien**
- „Schließen/Zurück“ führt zur Herkunftsseite, ersatzweise zur Detailseite der Messstelle.
- Escape schließt die Ansicht wie „Schließen/Zurück“.
- Ein Link „Original öffnen“ liefert das Foto in voller Auflösung.
```

```{req} Gruppierung der Übersicht
:id: REQ_UI_004
:status: accepted
:priority: should
:release: v1.1

Das System soll die Einträge der Übersicht ({need}`REQ_ANA_008`) nach Typ und
innerhalb eines Typs nach Name gruppieren. Typen sind die Messstellenarten und
PV-Anlagen ({need}`REQ_PV_012`) in der festen Reihenfolge Strom, PV, Wärme, Gas,
Wasser, Fahrzeug, Sonstiges ({need}`REQ_MTR_002`).

**Akzeptanzkriterien**
- Jede Gruppe hat eine Überschrift; leere Gruppen werden ausgeblendet.
- Die Sortierung nach Name beachtet keine Groß- und Kleinschreibung.
- Zwischenzähler (Wallbox, Wärmepumpe) erscheinen in der Gruppe Strom normal nach Name sortiert.
```

```{req} Setup-Assistent
:id: REQ_UI_005
:status: accepted
:priority: should
:release: v1.1

Das System soll Eigentümern einen Setup-Assistenten anbieten, der auch
komplexe Setups mit Hausstrom, Wärmepumpe, PV und E-Fahrzeug in einem Ablauf
einrichtet. Schritte:

1. Hausanschluss Strom: Einrichtungs- oder Zweirichtungszähler ({need}`REQ_MTR_004`).
2. Wärmepumpe: keine, eigener Zähler oder Zwischenzähler ({need}`REQ_MTR_008`).
3. PV-Anlage: Erzeugungszähler, Speicher, Einspeisevergütung ({need}`REQ_PV_001`, {need}`REQ_PV_002`, {need}`REQ_PV_009`).
4. E-Fahrzeug oder Plug-in-Hybrid mit optionalem Wallbox-Zwischenzähler ({need}`REQ_CAR_001`, {need}`REQ_CAR_008`).
5. Weitere Medien: Gas, Wasser, Wärme.
6. Verträge, optional ({need}`REQ_TAR_001`).
7. Zusammenfassung vor dem Anlegen.

Je Messstelle kann optional ein Foto hinterlegt werden. Mit AI-Konfiguration
schlägt es Zählernummer und Startstand vor ({need}`REQ_AI_007`), ohne
AI-Konfiguration wird es nur als Belegfoto des Startstands gespeichert.

**Akzeptanzkriterien**
- In einem leeren Haushalt wird der Assistent auf der Übersicht angeboten; sobald Messstellen existieren, ist er über die Einstellungen und die Messstellenliste erreichbar, um ein bestehendes Setup zu erweitern.
- Nur Eigentümer können den Assistenten nutzen.
- Nicht zutreffende Schritte werden übersprungen.
- Felder werden abhängig von den bisherigen Eingaben ein- und ausgeblendet, Pflichtfelder sind gekennzeichnet.
- Für jeden Zähler kann statt einer neuen eine vorhandene Messstelle gewählt werden, z.B. ein bestehender Wärmepumpen-Zwischenzähler.
- Angelegt wird erst nach Bestätigung der Zusammenfassung, alle Objekte in einer Transaktion.
- Ein begonnener Assistent bleibt als Entwurf erhalten und kann abgebrochen oder fortgesetzt werden.
```

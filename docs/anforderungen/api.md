# Programmierschnittstelle

```{req} REST-API mit OpenAPI
:id: REQ_API_001
:status: accepted
:priority: should
:release: v1

Das System soll eine versionierte JSON-REST-API (`/api/v1`) bereitstellen,
über die alle Daten eines Haushalts gelesen und geschrieben werden können,
die auch die Oberfläche bearbeitet: Zähler, Stände, Verträge, Kosten,
Fahrzeuge mit Kostenbuch, PV-Anlagen mit Kostenbuch, Haushalt und Dokumente.
Ein Token darf dabei höchstens, was der Nutzer, der es erstellt hat, mit
seiner aktuellen Rolle darf. Die OpenAPI-Beschreibung wird aus dem Code
erzeugt und unter `/api/v1/openapi.json` ausgeliefert.
```

```{req} API-Tokens
:id: REQ_API_002
:status: accepted
:priority: should
:release: v1

Das System soll persönliche API-Tokens mit Haushalts- und Rollenbindung
bereitstellen. Tokens werden nur gehasht gespeichert, sind widerrufbar und
haben optional ein Ablaufdatum.
```

```{req} Automatisch gelieferte Zählerstände
:id: REQ_API_003
:status: accepted
:priority: should
:release: v1

Das System soll Zählerstände aus automatischen Quellen (z.B. Home Assistant)
über die REST-API annehmen und je Register, Kalendertag und Quelle höchstens
eine Ablesung speichern. Auswertungen innerhalb eines Tages sind nicht Teil
des Umfangs.
```

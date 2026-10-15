# Import und Export

```{req} CSV-Import von Zählerständen
:id: REQ_IMP_001
:status: accepted
:priority: must
:release: MVP

Das System muss historische Zählerstände aus CSV-Dateien importieren.

**Akzeptanzkriterien**
- Spaltenzuordnung (Datum, Wert, optional Register/Zählernummer/Notiz) im UI.
- Datumsformate `TT.MM.JJJJ` und ISO 8601, Dezimaltrenner Komma oder Punkt.
- Vorschau mit Plausibilitätsprüfung vor dem Import; Duplikate (gleicher
  Zeitpunkt und Wert) werden übersprungen.
```

```{req} Export
:id: REQ_IMP_002
:status: accepted
:priority: should
:release: v1

Das System soll alle Daten eines Haushalts als CSV (je Entität) und als JSON
exportieren.
```

```{req} Vollständige Sicherung und Wiederherstellung
:id: REQ_IMP_003
:status: accepted
:priority: could
:release: später

Das System kann einen Haushalt inklusive Fotos als Archiv exportieren und in
eine andere Instanz importieren (z.B. Wechsel SQLite → PostgreSQL).
```

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
:status: superseded
:priority: could
:release: später
:links: REQ_IMP_004

Das System kann einen Haushalt inklusive Fotos als Archiv exportieren und in
eine andere Instanz importieren (z.B. Wechsel SQLite → PostgreSQL). Ersetzt
durch {need}`REQ_IMP_004`.
```

```{req} Vollständiger Export und Import ohne Datenverlust
:id: REQ_IMP_004
:status: accepted
:priority: must
:release: MVP

Das System muss alle Daten vollständig, einschließlich der Fotos, exportieren
und in eine neue Installation importieren können, ohne dass dabei Daten
verloren gehen.

**Akzeptanzkriterien**
- Betrieb: Export der ganzen Installation (Nutzer, Haushalte, Messstellen,
  Geräte, Register, Ablesungen, Entwürfe, Fotos) und Import in eine leere
  Installation; unabhängig vom Datenbank-Backend (z.B. SQLite nach PostgreSQL).
- Nutzer: Export eines Haushalts und Import als neuer Haushalt, auch in eine
  andere Installation.
- Export, Import und erneuter Export ergeben dieselben Daten.
- Das Archiv ist versioniert; neuere Versionen lesen Archive älterer
  Versionen. Beschädigte Archive werden abgelehnt, ohne Daten zu ändern.
```

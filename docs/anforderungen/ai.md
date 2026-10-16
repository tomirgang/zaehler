# AI-gestützte Erkennung

```{req} Zählerstand per Foto erkennen
:id: REQ_AI_001
:status: accepted
:priority: must
:release: MVP

Das System muss aus einem Foto (Kamera oder Upload) eines Zählers einen
Vorschlag für Zählerstand, Zählernummer, Register und Einheit erzeugen.
AI-Backend ist OpenRouter.

**Akzeptanzkriterien**
- Das Ergebnis enthält eine Konfidenz (0..1); unlesbare Felder bleiben leer.
- Antwortzeit bis zum Vorschlag typischerweise unter 10 s.
- Bilder werden vor dem Versand auf höchstens 2 MP verkleinert.
```

```{req} Automatische Zuordnung zum Zähler
:id: REQ_AI_002
:status: accepted
:priority: should
:release: MVP

Das System soll den erkannten Stand anhand der Zählernummer (sonst Art und
Wertebereich) einer Messstelle des aktuellen Haushalts zuordnen. Bei
Mehrdeutigkeit wählt der Nutzer aus.
```

```{req} Bestätigungspflicht
:id: REQ_AI_003
:status: accepted
:priority: must
:release: MVP

Das System darf AI-Ergebnisse nur als Vorschlag anzeigen. Gespeichert wird erst
nach Bestätigung bzw. Korrektur durch den Nutzer; die Plausibilitätsprüfung
({need}`REQ_RDG_002`) gilt auch hier. Abweichungen zwischen Vorschlag und
gespeichertem Wert werden zur Qualitätsmessung protokolliert.
```

```{req} Belege für Fahrzeugkosten erkennen
:id: REQ_AI_004
:status: accepted
:priority: should
:release: v1

Das System soll aus Fotos von Tankquittungen, Ladebelegen und
Werkstattrechnungen Datum, Betrag, Menge und Kategorie als Vorschlag für das
Kostenbuch extrahieren.
```

```{req} Rechnungsimport
:id: REQ_AI_005
:status: accepted
:priority: should
:release: v1

Das System soll aus Jahresabrechnungen (PDF oder Foto) Anbieter,
Abrechnungszeitraum, Zählerstände, Preise und Abschläge als Vorschlag
extrahieren, damit Verträge und historische Daten nicht manuell erfasst
werden müssen.
```

```{req} AI-Konfiguration und Datenschutz
:id: REQ_AI_006
:status: accepted
:priority: must
:release: MVP

Das System muss das AI-Modell konfigurierbar machen und die AI-Funktionen
vollständig abschaltbar halten. An OpenRouter werden nur das Bild/Dokument und
ein fester Prompt übertragen, keine Nutzer- oder Haushaltsdaten. Fehler oder
Zeitüberschreitungen führen zur manuellen Eingabe, nie zu Datenverlust.
```

```{req} Messstelle per Foto anlegen
:id: REQ_AI_007
:status: accepted
:priority: should
:release: MVP

Das System soll das Anlegen einer neuen Messstelle aus einem Foto des Zählers
ermöglichen. Erkannte Angaben (Art, Einheit, Zählernummer, Register,
Zählerstand) füllen das Formular als Vorschlag vor; alle Felder bleiben
editierbar ({need}`REQ_AI_003`). Das Foto wird Belegfoto des Startstands.
Ohne AI-Konfiguration bleibt der Ablauf mit leerem Formular und Foto nutzbar
({need}`REQ_AI_006`).
```

```{req} Mehrere Fotos auf einmal erfassen
:id: REQ_AI_008
:status: accepted
:priority: should
:release: MVP

Das System soll das Hochladen mehrerer Fotos in einem Schritt ermöglichen. Die
Fotos werden nacheinander analysiert und anschließend vom Nutzer nacheinander
geprüft und bestätigt oder verworfen ({need}`REQ_AI_003`).

**Akzeptanzkriterien**
- Ein Upload nimmt bis zu 20 Fotos an; nicht lesbare Fotos werden gemeldet,
  die übrigen weiterverarbeitet.
- Die Analyse läuft im Hintergrund, ein Foto nach dem anderen.
- Die Prüfseite zeigt die Position („Foto 2 von 5“); nach dem Speichern oder
  Verwerfen folgt das nächste offene Foto.
- Noch nicht analysierte Fotos zeigen einen Wartehinweis, der sich selbst
  aktualisiert.
```

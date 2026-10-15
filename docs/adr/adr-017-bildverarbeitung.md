# ADR-017: Serverseitige Bildverarbeitung ohne Metadaten

```{adr} Bilder serverseitig normalisieren, nur JPEG ohne Metadaten speichern
:id: ADR_017
:status: accepted
:decides: REQ_AI_001, REQ_RDG_004, REQ_NFR_004, SPEC_IMG_PROCESSING

**Datum:** 2026-10-15
```

## Kontext
Fotos von Smartphones enthalten EXIF-Daten (u.a. GPS-Position) und sind
mehrere Megapixel groß. Die AI benötigt höchstens etwa 2 MP. iOS speichert
Fotos als HEIC, wandelt sie beim Upload über `<input type="file">` mit
`accept="image/jpeg,…"` in der Regel in JPEG um.

## Entscheidung
- Normalisierung im Server mit `image` 0.25: Format prüfen (JPEG, PNG, WebP),
  EXIF-Ausrichtung anwenden, auf höchstens 2 MP verkleinern, als JPEG ohne
  Metadaten kodieren.
- Gespeichert und an OpenRouter gesendet wird nur das normalisierte Bild.
- HEIC wird nicht unterstützt.

## Alternativen
- *Verkleinern im Browser (Canvas)*: weniger Upload-Volumen, aber nicht
  erzwingbar; der Server müsste trotzdem prüfen und normalisieren.
- *HEIC über `libheif`*: C-Abhängigkeit im Image, Lizenz- und Build-Aufwand.
- *Original zusätzlich speichern*: verdoppelt den Speicherbedarf und behält
  die Metadaten.

## Konsequenzen
- CPU-Last je Upload (Dekodieren, Skalieren) in `spawn_blocking`.
- Uploads im HEIC-Format liefern eine Fehlermeldung mit Hinweis auf JPEG.
- Clientseitiges Verkleinern kann später zusätzlich ergänzt werden.

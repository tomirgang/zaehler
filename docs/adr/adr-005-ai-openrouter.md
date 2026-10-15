# ADR-005: AI-Erkennung über OpenRouter mit Bestätigungspflicht

```{adr} OpenRouter, Structured Outputs, Vorschlag + Bestätigung
:id: ADR_005
:status: accepted
:decides: REQ_AI_001, REQ_AI_003, REQ_AI_006, SPEC_AI_EXTRACTION, SPEC_AI_FLOW

**Datum:** 2026-10-15
```

## Kontext
Vision-LLMs lesen Zählerstände aus Fotos aus; Fehler bei Nachkommastellen und
verdeckten Ziffern treten auf. Verfügbare Modelle und Preise ändern sich häufig.

## Entscheidung
- Backend **OpenRouter** (OpenAI-kompatibel); Modell per Konfiguration.
- **Structured Outputs** (`json_schema`, `strict`) statt Auswertung von Freitext.
- AI liefert nur **Vorschläge**; Speichern erst nach Bestätigung. Abweichungen
  werden für eine Qualitätsmetrik protokolliert.
- Keine personenbezogenen Daten im Prompt.

## Alternativen
- *Lokales OCR* (Tesseract): erfordert eigene Vorverarbeitung für 7-Segment-Anzeigen,
  Rollenzählwerke und Spiegelungen.
- *Direkte Provider-APIs*: Bindung an einen Anbieter.
- *Auto-Übernahme ab Konfidenzschwelle*: LLM-Konfidenzwerte sind nicht kalibriert;
  eine Schwelle schließt Fehlerkennungen nicht aus.

## Konsequenzen
- Externe Abhängigkeit, die vollständig abschaltbar bleiben muss.
- Modellwechsel lässt sich über die protokollierten Korrekturen bewerten.

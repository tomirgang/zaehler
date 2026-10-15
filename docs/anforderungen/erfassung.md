# Erfassung von Zählerständen

```{req} Erfassung per Formular
:id: REQ_RDG_001
:status: accepted
:priority: must
:release: MVP

Das System muss Zählerstände über ein mobil optimiertes Formular erfassen:
Zähler wählen, Datum/Uhrzeit (Vorgabe: jetzt), Wert je Register, optionale Notiz.

**Akzeptanzkriterien**
- Der letzte Stand und das Datum der letzten Ablesung werden angezeigt.
- Eingabe mit Komma oder Punkt als Dezimaltrenner.
- Erfassung eines Stands in höchstens drei Interaktionen ab Dashboard.
```

```{req} Plausibilitätsprüfung
:id: REQ_RDG_002
:status: accepted
:priority: must
:release: MVP

Das System muss neue Stände vor dem Speichern prüfen und warnen, wenn
- der Stand kleiner als der vorherige ist (ohne möglichen Überlauf/Wechsel),
- der Tagesverbrauch einen konfigurierbaren Maximalwert übersteigt,
- der Zeitpunkt vor der letzten Ablesung liegt.

Der Nutzer kann nach expliziter Bestätigung trotzdem speichern.
```

```{req} Korrektur mit Historie
:id: REQ_RDG_003
:status: accepted
:priority: should
:release: v1

Das System soll Korrekturen und Löschungen von Ständen erlauben und dabei
protokollieren, wer wann welchen Wert geändert hat.
```

```{req} Belegfoto speichern
:id: REQ_RDG_004
:status: accepted
:priority: should
:release: MVP

Das System soll das Foto einer Ablesung als Beleg speichern (Objektspeicher)
und in der Detailansicht anzeigen. Fotos werden mit dem Stand gelöscht.
```

```{req} Ableseturnus und Fälligkeit
:id: REQ_RDG_005
:status: accepted
:priority: should
:release: v1

Das System soll je Zähler einen Ableseturnus (z.B. monatlich am 1.) verwalten
und fällige bzw. überfällige Ablesungen im Dashboard hervorheben.
```

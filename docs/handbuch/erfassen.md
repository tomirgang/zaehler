# Stände erfassen

## Per Formular

Auf der Übersicht führt **Stand erfassen** direkt zum Formular einer
Messstelle. Es enthält ein Feld je Register, darunter steht der letzte Stand
zur Orientierung. Zeitpunkt ist „jetzt“; für nachgetragene Stände lässt er
sich ändern. Eine Notiz ist optional.

```{figure} bilder/ablesung-erfassen.png
:alt: Formular Stand erfassen mit zwei Registern
:width: 100%

Stand erfassen für einen Zweirichtungszähler mit Bezug (1.8.0) und
Einspeisung (2.8.0).
```

Vor dem Speichern prüft die App jeden Wert:

- **Fallender Stand**: Der neue Wert ist kleiner als der vorherige – meist
  ein Tippfehler; bei Zählern mit Überlauf (Vorkommastellen gesetzt) wird der
  Überlauf erkannt.
- **Ungewöhnlich hoher Verbrauch**: Der Tagesverbrauch liegt über dem
  hinterlegten Maximum.
- **Zeitpunkt vor dem Einbau** oder in der Zukunft wird abgelehnt.

Bei einer Warnung zeigt die App sie an; mit **Trotzdem speichern** wird der
Stand übernommen.

## Per Foto

**Foto erfassen** nimmt ein oder bis zu 20 Fotos von Zählern auf einmal
entgegen. Die Fotos werden verkleinert, Ortsdaten und andere Metadaten
entfernt; der Aufnahmezeitpunkt aus dem Foto wird als Ablesezeitpunkt
vorgeschlagen.

```{figure} bilder/foto-erfassung.png
:alt: Seite Stand per Foto erfassen
:width: 100%

Ohne eingerichtete Erkennung wird das Foto gespeichert und der Stand von
Hand eingegeben.
```

Ist in der Installation eine KI-Erkennung eingerichtet, schlägt die App
Stand, Zählernummer und die passende Messstelle vor und zeigt, wie sicher
die Erkennung ist. Gespeichert wird immer erst nach Bestätigung auf der
Prüfseite; dort lassen sich alle Werte korrigieren. Bei mehreren Fotos
führt die App nacheinander durch die Prüfseiten. Das Foto bleibt als
Belegfoto an der Ablesung und lässt sich in der Ablesungsliste öffnen.

## Sammelerfassung mit PV

Für einen Hausanschluss mit PV-Anlage erfasst **PV → Hausanschluss →
Sammelerfassung** die Stände aller zugehörigen Zähler (Hausanschluss,
Erzeugung, Speicher) mit einem gemeinsamen Zeitpunkt. Leere Felder werden
nicht gespeichert; Fotos lassen sich zuordnen.

```{figure} bilder/pv-sammelerfassung.png
:alt: Sammelerfassung für Hausanschluss und Erzeugungszähler
:width: 100%

Sammelerfassung: Bezug, Einspeisung und Erzeugung auf einmal.
```

## CSV-Import

**CSV-Import** übernimmt Zählerstände aus einer Datei, z.B. aus einer
Tabellenkalkulation oder einem anderen Programm. Je Zeile stehen Datum und
Zählerstand, optional Uhrzeit, Register, Zählernummer und Notiz; Trennzeichen
Semikolon, Komma oder Tabulator. Nach dem Hochladen ordnet man die Spalten
zu und wählt Messstelle, Datumsformat und Dezimaltrenner. **Prüfen** zeigt
eine Vorschau je Zeile (übernehmen, Hinweis, Duplikat, Fehler); importiert
wird erst danach. Bereits vorhandene Stände werden als Duplikat erkannt.

```{figure} bilder/import.png
:alt: Seite Zählerstände importieren
:width: 100%

CSV-Import: Datei wählen, danach Zuordnung und Vorschau.
```

## Automatisch gelieferte Stände

Stände aus Smart-Home-Systemen (z.B. Home Assistant) oder Exporten von
Speichersystemen gelangen über die REST-API in die App. Dafür legt man unter
**Einstellungen → API-Tokens** ein Token an (siehe {doc}`einstellungen`).
Je Register und Tag speichert die App höchstens einen automatisch gelieferten
Stand; manuelle Ablesungen bleiben unberührt. Für SENEC-Wochenexporte gibt es
das Skript `scripts/senec_import.py` (siehe {doc}`../betrieb/api`).

## Korrigieren und löschen

Jede Ablesung in der Liste der Messstelle hat **Bearbeiten** und
**Löschen**. Korrekturen und Löschungen werden mit altem und neuem Wert,
Person und Zeitpunkt protokolliert; der **Änderungsverlauf** der Messstelle
zeigt alle Änderungen. Ablesungen eines Zählerwechsels lassen sich nicht
löschen.

```{figure} bilder/messstelle-ablesungen.png
:alt: Liste der Ablesungen einer Messstelle
:width: 100%

Ablesungen einer Messstelle mit Quelle (Formular, Foto, Import, API) und
Aktionen.
```

```{figure} bilder/verlauf.png
:alt: Änderungsverlauf mit einer Korrektur
:width: 100%

Der Änderungsverlauf zeigt, wer wann welchen Wert geändert hat.
```

# Benutzerhandbuch

Dieses Handbuch beschreibt die Bedienung von Zähler für Nutzerinnen und
Nutzer. Die Bilder zeigen einen Beispielhaushalt „Musterhaus“ mit Hausstrom,
PV-Anlage, Wärmepumpe, Wallbox, Wasser und zwei Fahrzeugen.

## Aufbau der Oberfläche

Oben stehen die Bereiche der App: **Übersicht**, **Messstellen**, **Kosten**,
**Fahrzeuge**, **PV**, **Foto erfassen**, **CSV-Import** und
**Einstellungen**. Rechts daneben zeigt das Feld **Haushalt** den aktiven
Haushalt; wer mehreren Haushalten angehört, wechselt dort. Unter der
Navigation zeigt eine Brotkrumenleiste, wo man sich befindet, und führt mit
einem Klick zurück.

```{figure} bilder/uebersicht.png
:alt: Übersicht mit Karten je Messstelle, gruppiert nach Art
:width: 100%

Die Übersicht gruppiert die Messstellen nach Art (Strom, Wasser, Fahrzeug …)
und zeigt je Karte den letzten Stand, den Verbrauch des letzten vollen
Monats mit Ampel, die Jahresprognose und die nächste fällige Ablesung.
```

Auf dem Smartphone klappt die Navigation in ein Menü zusammen. Die App lässt
sich über das Browsermenü („Zum Startbildschirm hinzufügen“) wie eine
eigene App installieren.

```{figure} bilder/mobil-uebersicht.png
:alt: Übersicht auf dem Smartphone
:width: 300px

Übersicht auf dem Smartphone
```

```{figure} bilder/mobil-ablesung.png
:alt: Stand erfassen auf dem Smartphone
:width: 300px

Stand erfassen auf dem Smartphone
```

## Begriffe

Messstelle
: Ein Zähler aus Sicht des Haushalts, z.B. „Hausstrom“ oder „Wasser“. Eine
  Messstelle bleibt bei einem Zählerwechsel erhalten; das physische Gerät mit
  Zählernummer wird ausgetauscht.

Register
: Ein Zählwerk eines Zählers. Ein Zweirichtungszähler hat die Register
  Bezug (z.B. `1.8.0`) und Einspeisung (`2.8.0`), ein Doppeltarifzähler HT
  und NT.

Zwischenzähler
: Ein Zähler hinter einem anderen, z.B. Wärmepumpe oder Wallbox hinter dem
  Hausstromzähler. Sein Verbrauch ist im übergeordneten Zähler enthalten.

Gemessen, geschätzt, prognostiziert
: Werte zwischen zwei Ablesungen sind gemessen. Liegen Zeiträume vor der
  ersten oder nach der letzten Ablesung, schätzt die App sie mit derselben
  Rate bzw. dem Saisonprofil und kennzeichnet sie.

## Rollen

Jede Person gehört einem Haushalt mit einer Rolle an:

| Rolle | Darf |
|---|---|
| Lesen | alles ansehen |
| Mitglied | zusätzlich Stände erfassen, korrigieren und importieren, Kostenbuch und Ladevorgänge pflegen |
| Eigentümer | zusätzlich Messstellen, Verträge, Fahrzeuge, PV-Anlagen, Mitglieder und den Haushalt verwalten |

Schaltflächen, die die eigene Rolle nicht erlaubt, werden nicht angezeigt.

## Inhalt

- {doc}`erste-schritte`: Anmelden, Setup-Assistent, Messstellen anlegen
- {doc}`erfassen`: Stände per Formular, Foto, Sammelerfassung und Import
- {doc}`auswerten`: Verbrauch, Vergleiche, Ampel und Prognose
- {doc}`kosten`: Verträge, Abschläge und Kostenübersicht
- {doc}`photovoltaik`: PV-Anlagen, Kennzahlen und Wirtschaftlichkeit
- {doc}`fahrzeuge`: Kostenbuch, Laden, Wertverlust und Kosten pro km
- {doc}`einstellungen`: Haushalt, Mitglieder, Benachrichtigungen, Datensicherung und API

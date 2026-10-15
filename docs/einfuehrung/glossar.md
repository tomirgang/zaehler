# Glossar

Haushalt
: Mandant. Alle fachlichen Daten gehören genau einem Haushalt. Nutzer erhalten
  über eine Mitgliedschaft (Rolle `owner`, `member`, `viewer`) Zugriff.

Messstelle / Zähler
: Logischer Zähler, z.B. „Hausstrom“ oder „VW ID.3“. Überdauert einen
  Gerätewechsel. Im Code: `Meter`.

Zählergerät
: Physisches Gerät an einer Messstelle mit Zählernummer und Vorkommastellen.
  Ein **Zählerwechsel** beendet ein Gerät und beginnt ein neues. Im Code: `MeterDevice`.

Zwischenzähler
: Messstelle hinter einer anderen Messstelle (z.B. Wallbox-Zähler hinter dem
  Hausstromzähler). Ihr Verbrauch ist im Verbrauch der übergeordneten
  Messstelle enthalten und wird einem Fahrzeug zugeordnet.

Hausanschluss
: Zweirichtungszähler des Netzbetreibers mit den Registern Bezug und
  Einspeisung. Bezugsgröße der PV-Kennzahlen; mehrere PV-Anlagen können einen
  Hausanschluss teilen.

PV-Anlage
: Photovoltaikanlage mit Typ `Überschusseinspeisung`, `Volleinspeisung` oder
  `Steckersolargerät`, Leistung in kWp und zugeordneten Messstellen
  (Hausanschluss, Erzeugungszähler, optional Speicher).

Eigenverbrauch
: Selbst genutzter Anteil der Erzeugung: Erzeugung minus Einspeisung minus
  Nettoladung des Speichers.

Eigenverbrauchsquote
: Eigenverbrauch geteilt durch Erzeugung.

Autarkiegrad
: Eigenverbrauch geteilt durch Gesamtverbrauch (Bezug plus Eigenverbrauch).

Spezifischer Ertrag
: Erzeugung einer Anlage je kWp Leistung, meist je Jahr (kWh/kWp).

Einspeisevergütung
: Vergütung je eingespeister kWh; im Tarifmodell als Vertrag mit negativem
  Arbeitspreis abgebildet.

Degradation
: Jährlicher Leistungsverlust von PV-Modulen in Prozent.

Amortisationszeitpunkt
: Zeitpunkt, ab dem die kumulierten Erträge (Vergütung und Ersparnis) die
  kumulierten Kosten einer Anlage erreichen.

Antriebsart
: `Verbrenner`, `Elektro` oder `Plug-in-Hybrid`; bestimmt, ob Tank- oder
  Ladevorgänge erfasst werden.

Register
: Einzelnes Zählwerk eines Geräts, z.B. `HT`/`NT` (Zweitarif) oder `export`
  (Einspeisung). Ein Zähler mit einem Zählwerk hat das Register `main`.

Zählerstand / Ablesung
: Wert eines Registers zu einem Zeitpunkt, Quelle: Formular, Foto (AI), CSV-Import
  oder Rechnung.

Überlauf
: Ein Zählwerk mit *n* Vorkommastellen springt nach 10ⁿ-1 auf 0.

Verbrauch
: Differenz zweier Zählerstände; auf Zeiträume (Monat, Jahr) linear oder per
  Saisonprofil verteilt.

Saisonprofil
: Relative Gewichtung der Kalendermonate am Jahresverbrauch (z.B. Heizung im
  Winter höher). Wird aus eigener Historie gelernt.

Abrechnungszeitraum
: Zeitraum eines Vertrags, über den abgerechnet wird, z.B. 01.04.-31.03.

Vertrag
: Liefervertrag für eine Messstelle mit Anbieter, Abrechnungszeitraum,
  Kostenpositionen und Abschlägen. Ein Anbieterwechsel erzeugt einen neuen Vertrag.

Kostenposition
: Teil eines Vertrags mit eigenem Preis, z.B. Frischwasser, Schmutzwasser,
  Zählermiete. Hat einen Umrechnungsfaktor von Mess- auf Abrechnungseinheit
  (Gas: m³ × Brennwert × Zustandszahl = kWh).

Grundpreis / Arbeitspreis
: Fixer Preis pro Zeit (Tag/Monat/Jahr) bzw. Preis pro Einheit (brutto).

Staffelpreis
: Arbeitspreis abhängig von der kumulierten Menge im Abrechnungszeitraum.

Abschlag
: Regelmäßige Vorauszahlung. Abschläge minus tatsächliche bzw. prognostizierte
  Kosten ergeben Erstattung oder Nachzahlung.

Erwartungsbereich
: Band um den erwarteten Verbrauch, aus Historie (Mittelwert ± k·σ) oder aus
  einem manuellen Ziel. Grundlage der Ampel grün/gelb/rot.

Kosten pro km
: *Variable Kosten* (Kraftstoff/Strom, Werkstatt, Reifen …) bzw. *Vollkosten*
  (zusätzlich Fixkosten wie Versicherung und Steuer sowie Wertverlust) geteilt
  durch die gefahrenen Kilometer.

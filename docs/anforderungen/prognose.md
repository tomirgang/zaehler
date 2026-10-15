# Prognosen

```{req} Jahresverbrauchsprognose
:id: REQ_FC_001
:status: accepted
:priority: must
:release: MVP

Das System muss den Verbrauch für das laufende Kalenderjahr und den laufenden
Abrechnungszeitraum hochrechnen. Grundlage ist der gemessene Verbrauch und das
Saisonprofil. Der Abdeckungsgrad (gemessener Anteil) wird angezeigt.
```

```{req} Kostenprognose
:id: REQ_FC_002
:status: accepted
:priority: must
:release: v1

Das System muss aus der Verbrauchsprognose und den gültigen Tarifen die Kosten
für den Abrechnungszeitraum und das Kalenderjahr prognostizieren.
```

```{req} Prognoseunsicherheit
:id: REQ_FC_003
:status: accepted
:priority: could
:release: später

Das System kann ein Unsicherheitsband für die Prognose ausweisen (z.B. aus der
Streuung der Vorjahre) und mit sinkender Restlaufzeit schmaler werden lassen.
```

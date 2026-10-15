# Benachrichtigungen

```{req} Ablese-Erinnerungen
:id: REQ_NTF_001
:status: accepted
:priority: must
:release: v1

Das System muss gemäß Ableseturnus ({need}`REQ_RDG_005`) an fällige Ablesungen
erinnern und bei Überfälligkeit erneut erinnern (höchstens einmal täglich).
```

```{req} Kanal E-Mail
:id: REQ_NTF_002
:status: accepted
:priority: must
:release: v1

Das System muss Benachrichtigungen per E-Mail (SMTP mit STARTTLS/TLS) versenden.
```

```{req} Abweichungsalarme
:id: REQ_NTF_004
:status: accepted
:priority: should
:release: v1

Das System soll benachrichtigen, wenn ein neuer Stand einen Verbrauch außerhalb
des Erwartungsbereichs ergibt (Ampel rot), z.B. als Hinweis auf ein Wasserleck.
```

```{req} Benachrichtigungseinstellungen
:id: REQ_NTF_005
:status: accepted
:priority: should
:release: v1

Das System soll je Nutzer die E-Mail-Adresse für Benachrichtigungen und die
gewünschten Ereignistypen konfigurierbar machen.
```

# E-mail texts (SPEC_NTF_EMAIL). Arguments are already formatted.

reminder-subject = Reading due: { $household }
reminder-intro = The following readings are due in household "{ $household }":
reminder-due = { $meter }: due on { $date }
reminder-overdue = { $meter }: overdue since { $date }
reminder-capture = Record: { $link }

alarm-subject = Unusual consumption: { $meter }
alarm-intro = In household "{ $household }", the consumption of meter "{ $meter }" differs clearly from the expectation.
alarm-period = Period: { $from } to { $until }
alarm-consumption = Consumption: { $consumption } { $unit }
alarm-expected = Expected: { $lower } to { $upper } { $unit }
alarm-details = Details: { $link }

invite-subject = Invitation to household "{ $household }"
invite-intro = { $inviter } invites you to household "{ $household }" (role: { $role }).
invite-accept = Accept invitation: { $link }
invite-expires = The link is valid until { $date } and can be used only once.

role-owner = Owner
role-member = Member
role-viewer = Viewer

footer-settings = Notification settings: { $link }
footer-sender = This message was sent automatically by Zähler.

pv-alarm-subject = Low PV yield: { $system }
pv-alarm-intro = In household "{ $household }", the generation of PV system "{ $system }" (meter "{ $meter }") is below the expectation.
pv-alarm-generation = Generation: { $generation } kWh, expected: { $expected } kWh (threshold { $threshold } %)

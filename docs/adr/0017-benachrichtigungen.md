# ADR 0017: Benachrichtigungen per E-Mail und Webhook, Wochenbericht

- Status: angenommen
- Datum: 2026-10-22

## Kontext
Bisher meldete nur die Tray-App Konflikte und Fehler. Auf dem Server (ohne Tray-App) standen sie
nur im Protokoll, in der Web-Oberfläche und in `protonsync`. Der Auftraggeber möchte bei Konflikten
benachrichtigt werden, z. B. per E-Mail, „gerne auch anderes, falls sinnvoll“, außerdem einen
Wochenbericht; alles soll einzeln an- und abschaltbar sein. Mit dem Löschschutz (ADR 0016) werden
Konflikte häufiger, deshalb ist eine Meldung nach außen wichtiger geworden.

## Entscheidung
- **Port `Notifier`** in der Anwendungsschicht (`send(Message)`, Fehler vorübergehend/endgültig),
  `FakeNotifier` im Testkit. Neues Crate **`adapter-notify`** mit zwei Kanälen:
  - **E-Mail über SMTP** mit `lettre` (MIT/Apache, ohne Standard-Features, rustls mit aws-lc-rs
    wie der Rest, Wurzelzertifikate aus `webpki-roots`, das schon im Baum war). STARTTLS, TLS
    oder unverschlüsselt (nur für einen Mailserver im eigenen Netz).
  - **Webhook** mit dem schon vorhandenen `reqwest`: JSON oder Text mit `Title`-Kopfzeile. Damit
    gehen ntfy, Gotify, Home Assistant und eigene Skripte; Push aufs Handy ohne eigenen Dienst.
  - Nicht gebaut: Matrix, Telegram, Signal o. Ä. Das geht über einen Webhook-Empfänger
    (z. B. ntfy oder Home Assistant), ohne weitere Abhängigkeiten im Daemon.
- **Daemon-Modul `notify`** hört auf die eigenen Ereignisse (wie die Tray-App), entscheidet in
  `Notifications` ohne Ein-/Ausgabe, was gemeldet wird, und verteilt über `Dispatcher`:
  - Themen: Konflikte, Sync-Fehler, Job-Fehler, abgelaufene Anmeldung, Wochenbericht; jedes je
    Kanal schaltbar (`[notify.mail]`, `[notify.webhook]`). Standard: alles an außer Wochenbericht.
  - Gegen Nachrichtenfluten: Konflikte einer Minute werden gebündelt; ein scheiternder Abgleich
    wird einmal gemeldet und erst nach einer Erholung wieder; ohne Anmeldung kommt nur die
    Anmelde-Meldung.
  - Vorübergehende Fehler werden nach 10 s und 60 s wiederholt; der Versand läuft in eigenen
    Aufgaben und hält weder Ereignisse noch Abgleiche auf.
- **Wochenbericht** nach Cron-Ausdruck (`weekly_report_schedule`, Standard `0 8 * * 1`, Ortszeit
  über denselben Parser und dieselbe Zeitzone wie die Upload-Jobs). Gespeichertes (Konflikte,
  Papierkorb, Jobs, letzter Abgleich) holt er über die Daemon-API (`Backend`), die Zähler
  (Übertragungen, Fehler) sammelt er im Speicher aus den Ereignissen. Nach einem Neustart beginnt
  die Zählung neu; der Bericht nennt seinen Zeitraum. Ein verpasster Termin wird nicht nachgeholt.
- **Geheimnisse** (SMTP-Passwort, Webhook-Token) stehen nie in der Konfiguration: systemd-Credential,
  Datei mit Rechten 0600 oder Umgebungsvariable, wie beim Schlüssel der Geheimnisdatei (ADR 0006).
  `Debug` der Einstellungen blendet sie aus.
- **Sprache** der Nachrichten über `[notify] language` (Standard Deutsch).
- **Tray-App:** Desktop-Meldungen bekommen ein Thema und lassen sich in `tray.toml`
  (`[notification_topics]`) einzeln abschalten. Ein Schalter im Einstellungsfenster folgt später.

## Folgen
- Vier neue kleine Crates im Baum (`lettre`, `email-encoding`, `email_address`,
  `quoted_printable`), alle MIT/Apache.
- Ohne `[notify]` ändert sich nichts.
- Getestet gegen Fake-SMTP- und Fake-HTTP-Server auf localhost und mit pausierter Tokio-Zeit;
  ein echter Mailversand ist noch nicht erprobt.

---

## English

**ADR 0017: Notifications by email and webhook, weekly report**

- Status: accepted
- Date: 2026-10-22

### Context
So far only the tray app reported conflicts and failures. On the server (without a tray app) they
were only in the log, the web interface and `protonsync`. The client wants to be notified about
conflicts, e.g. by email, "and other ways too if useful", plus a weekly report; everything should
be switchable on its own. With the protection (ADR 0016) conflicts become more frequent, so
reporting them outside matters more.

### Decision
- **`Notifier` port** in the application layer (`send(Message)`, temporary/permanent errors),
  `FakeNotifier` in the testkit. New crate **`adapter-notify`** with two channels:
  - **Email over SMTP** with `lettre` (MIT/Apache, no default features, rustls with aws-lc-rs like
    the rest, root certificates from `webpki-roots`, which was already in the tree). STARTTLS, TLS
    or plain (only for a mail server in the own network).
  - **Webhook** with the existing `reqwest`: JSON, or text with a `Title` header. This covers ntfy,
    Gotify, Home Assistant and own scripts; push to the phone without a service of our own.
  - Not built: Matrix, Telegram, Signal and the like. These work through a webhook receiver
    (e.g. ntfy or Home Assistant) without more dependencies in the daemon.
- **Daemon module `notify`** listens to the daemon's own events (like the tray app), decides in
  `Notifications` without I/O what is reported, and distributes via `Dispatcher`:
  - Topics: conflicts, sync failures, job failures, expired login, weekly report; each switchable
    per channel (`[notify.mail]`, `[notify.webhook]`). Default: everything on except the report.
  - Against floods: conflicts of one minute are batched; a failing sync is reported once and again
    only after it recovered; without a login only the login message comes.
  - Temporary errors are retried after 10 s and 60 s; sending runs in tasks of its own and holds up
    neither events nor syncs.
- **Weekly report** by cron expression (`weekly_report_schedule`, default `0 8 * * 1`, local time
  through the same parser and time zone as the upload jobs). Stored data (conflicts, trash, jobs,
  last sync) comes through the daemon API (`Backend`), the counters (transfers, errors) are
  collected in memory from the events. After a restart counting starts again; the report names its
  period. A missed time is not made up.
- **Secrets** (SMTP password, webhook token) never stand in the configuration: systemd credential,
  file with mode 0600 or environment variable, as with the key of the secrets file (ADR 0006).
  `Debug` of the settings hides them.
- **Language** of the messages via `[notify] language` (default German).
- **Tray app:** desktop notifications get a topic and can be switched off one by one in
  `tray.toml` (`[notification_topics]`). A switch in the settings window follows later.

### Consequences
- Four new small crates in the tree (`lettre`, `email-encoding`, `email_address`,
  `quoted_printable`), all MIT/Apache.
- Without `[notify]` nothing changes.
- Tested against fake SMTP and HTTP servers on localhost and with paused Tokio time; real email
  delivery has not been tried yet.

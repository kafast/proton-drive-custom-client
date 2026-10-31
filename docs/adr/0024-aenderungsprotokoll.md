# ADR 0024: Änderungsprotokoll mit täglicher Zusammenfassung

- Status: angenommen
- Datum: 2026-10-30

## Kontext
Für die Anfangs- und Testphase möchte der Auftraggeber nachvollziehen können, was protonsync
wirklich verändert hat, „vor allem online“, also in Proton. Bisher gab es dafür nur Zähler
(letzter Abgleich, Wochenbericht) und das Journal des Daemons, in dem einzelne Pfade fehlten.
Optional soll einmal täglich eine E-Mail mit den Änderungen kommen.

## Entscheidung
- **Was protokolliert wird:** jeder **ausgeführte** Schritt eines Abgleichs, mit Zeitpunkt,
  Sync-Paar, Pfad, Ordner oder Datei, **geänderter Seite** (Proton oder lokal) und Art: neu,
  neue Version, in den Papierkorb, ins Archiv, archiviert und ersetzt, Kopie behalten (Backup-Modus),
  Konflikt. Die Domänenregel `change_log::classify` leitet das aus der Aktion ab. Probeläufe,
  gescheiterte Schritte und wegen der Richtung ignorierte Änderungen stehen nicht darin (sie
  ändern nichts; Fehler meldet schon der Abgleich). „Seite Proton“ heißt: protonsync hat in
  Proton etwas geändert; „Seite lokal“ heißt: eine Änderung aus Proton wurde lokal übernommen.
- **Umbenennungen und Verschiebungen** erkennt der Abgleich (ADR 0007) als Löschen am alten und
  Anlegen am neuen Ort; so stehen sie auch im Protokoll.
- **Speicherort:** Port `ChangeLog` mit Tabelle `changes` im vorhandenen SQLite-Zustand
  (Migration 4), Contract-Tests für Fake und SQLite. Die Sync-Engine kennt den Port nicht; sie
  liefert die Änderungen im `SyncReport`, der Daemon speichert sie. Jede Änderung steht
  zusätzlich als Zeile `change` im Journal.
- **Aufbewahrung:** `[general] change_log_days`, Standard 30 Tage wie der Papierkorb; `0`
  schaltet das Protokoll ab. Ältere Einträge löscht der Daemon beim Schreiben neuer.
- **Anzeigen:** API-Anfrage `Changes` (Zeitraum, Paar, Höchstzahl, höchstens 10 000 je Anfrage),
  `protonsync changes [--pair] [--days] [--limit] [--json]`, Web-Seite „Änderungen“. Die
  zweisprachigen Beschreibungen liegen im Crate `client`, damit CLI, Web und E-Mail gleich
  formulieren.
- **Tägliche Zusammenfassung:** ein weiteres Thema `daily_changes` der vorhandenen
  Benachrichtigungen (ADR 0017), je Kanal (`[notify.mail]`, `[notify.webhook]`) schaltbar und
  **standardmäßig aus**; Zeitpunkt `[notify] daily_changes_schedule` (Cron in Ortszeit,
  Standard täglich 7:00). Die Nachricht deckt die Zeit seit der letzten Zusammenfassung ab
  (die erste nach dem Start: 24 Stunden) und listet zuerst die Änderungen in Proton, dann die
  übernommenen, jeweils älteste zuerst, höchstens 300 je Abschnitt. Auch ohne Änderungen
  kommt eine kurze Nachricht: in der Testphase zeigt sie, dass der Daemon läuft.
- **Versandweg:** der vorhandene SMTP-Versand, kein lokales `sendmail`. Begründung: SMTP ist
  schon gebaut und getestet, funktioniert im Container ohne Mailprogramm und auch mit
  Proton Mail (SMTP-Token); ein lokales `sendmail` müsste erst eingerichtet werden und fehlt in
  schlanken Images. Wer lieber Push-Nachrichten will, schaltet `daily_changes` am Webhook ein.

## Folgen
- Keine neuen Abhängigkeiten.
- Nicht protokolliert werden bisher: Änderungen über das FUSE-Laufwerk, Upload-Jobs, der
  Foto-Abgleich, Konfliktlösungen und Wiederherstellungen aus dem Papierkorb. Sie lassen sich
  über denselben Port nachrüsten (eigene Zeilen mit eigener Kennung statt des Sync-Paars).
- Die Tray-App zeigt das Protokoll noch nicht; `protonsync changes` und die Web-Seite reichen
  für die Testphase.
- Wer `change_log_days` später auf 0 setzt, behält die alten Einträge, bis er es wieder einschaltet.

---

## English

**ADR 0024: Change log with a daily digest**

- Status: accepted
- Date: 2026-10-30

### Context
For the early and test phase the client wants to see what protonsync really changed, "above all
online", i.e. in Proton. So far there were only counters (last sync, weekly report) and the
daemon journal, which lacked the individual paths. Optionally an email with the changes should
arrive once a day.

### Decision
- **What is logged:** every **executed** step of a sync, with time, sync pair, path, folder or
  file, **changed side** (Proton or local) and kind: new, new version, to the trash, to the
  archive, archived and replaced, copy kept (backup mode), conflict. The domain rule
  `change_log::classify` derives this from the action. Dry runs, failed steps and changes ignored
  because of the direction are not in it (they change nothing; the sync already reports
  failures). "Side Proton" means protonsync changed something in Proton; "side local" means a
  change from Proton was taken over locally.
- **Renames and moves** are seen by the sync (ADR 0007) as a deletion at the old and a creation
  at the new place, and are logged that way.
- **Storage:** port `ChangeLog` with table `changes` in the existing SQLite state (migration 4),
  contract tests for the fake and SQLite. The sync engine does not know the port; it returns the
  changes in its `SyncReport`, and the daemon stores them. Every change is also a `change` line in
  the journal.
- **Retention:** `[general] change_log_days`, default 30 days like the trash; `0` turns the log
  off. The daemon removes older entries when it writes new ones.
- **Viewing:** API request `Changes` (period, pair, limit, at most 10,000 per request),
  `protonsync changes [--pair] [--days] [--limit] [--json]`, web page "Changes". The bilingual
  descriptions live in the `client` crate so CLI, web and email word things the same way.
- **Daily digest:** one more topic `daily_changes` of the existing notifications (ADR 0017),
  switchable per channel (`[notify.mail]`, `[notify.webhook]`) and **off by default**; time
  `[notify] daily_changes_schedule` (cron in local time, default every day 7:00). The message
  covers the time since the last digest (the first after the start: 24 hours) and lists the
  changes in Proton first, then the ones taken over, each oldest first, at most 300 per section.
  A short message goes out even without changes: in the test phase it shows the daemon is running.
- **Delivery:** the existing SMTP sending, no local `sendmail`. Reason: SMTP is already built and
  tested, works in the container without a mail program and also with Proton Mail (SMTP token);
  a local `sendmail` would have to be set up first and is missing in slim images. Whoever prefers
  push messages switches `daily_changes` on for the webhook.

### Consequences
- No new dependencies.
- Not logged so far: changes through the FUSE drive, upload jobs, the photo sync, conflict
  resolutions and restores from the trash. They can be added through the same port (own rows
  with their own id instead of the sync pair).
- The tray app does not show the log yet; `protonsync changes` and the web page are enough for
  the test phase.
- Setting `change_log_days` to 0 later keeps the old entries until it is switched on again.

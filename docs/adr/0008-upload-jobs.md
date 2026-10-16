# ADR 0008: Upload-Jobs mit eigenem Cron-Parser, Ortszeit über `jiff` und vorsichtiger Aufbewahrung

- Status: angenommen
- Datum: 2026-10-16

## Kontext
Phase 2 verlangt allgemeine Upload-Jobs (Abschnitt 3.5): Quellordner, Muster, Ziel, Zeitplan
(Cron oder „bei neuer Datei“), Aufbewahrung, Prüfung, Wiederholung und Benachrichtigung.
Anwendungsfall sind Home-Assistant-Backups auf einem Server, oft auf einer SMB/NFS-Freigabe.
Zu entscheiden war, wie Zeitpläne berechnet werden, wie „neue Datei“ erkannt wird, ohne Proton
ständig zu fragen, und wie die Aufbewahrung keine Daten verliert.

## Entscheidung
- **Domäne:** `UploadJob`, `CronSchedule`, `GlobPattern` und `Retention` sind reine Typen ohne I/O.
  Der Cron-Parser ist selbst geschrieben (fünf Felder, Listen, Bereiche, Schritte, Namen,
  `@daily` usw., Vixie-Regel für Tag/Wochentag) und rechnet in **Ortszeit ohne Zeitzone**
  (`LocalDateTime`). Eine Bibliothek hätte Zeitzonen in die Domäne gezogen; die Regeln sind klein
  und mit Property-Tests abgesichert.
- **Zeitzone nur im Daemon:** `LocalZone` (Crate `jiff`, MIT/Unlicense) rechnet zwischen Zeitpunkt
  und Ortszeit, Zeitzone aus `TZ` bzw. `/etc/localtime`, sonst UTC. Lücke im März: direkt danach
  laufen; doppelte Stunde im Oktober: nur einmal. Verpasste Termine werden nicht nachgeholt.
  Lange Wartezeiten werden in Schritten von einer Minute an der Uhr geprüft (Ruhezustand,
  Uhrstellen).
- **„Bei neuer Datei“:** Watcher auf dem Quellordner plus Prüfung alle fünf Minuten (Netzlaufwerke
  melden oft nichts). Gelaufen wird nur, wenn sich die Signatur der passenden Dateien (Name, Größe,
  Änderungszeit) geändert hat; Proton wird sonst nicht gefragt. Dateien in der Ruhezeit führen zu
  einem erneuten Lauf nach `settle_seconds`.
- **Ablauf (`UploadJobRunner`):** gleiche Größe und, bei `verify = "hash"`, gleiche SHA-1 gelten
  als schon hochgeladen; geänderte Dateien werden als neue Revision geladen. Jeder Upload wird
  geprüft.
- **Aufbewahrung:** nur nach einem Lauf ohne Fehler, nur für Dateien, die zum Muster passen, die
  neueste Datei bleibt immer. Abgelaufenes geht in den Proton-Papierkorb und in den Papierkorb von
  protonsync unter dem Besitzer `job:<id>` und ist wiederherstellbar.
- **Fehler:** Wiederholung nach 1, 2, 4, … Minuten (höchstens 60), bis `max_attempts`; danach das
  Ereignis `JobGaveUp` und ein Fehler im Protokoll. Jeder Lauf kommt in den Verlauf
  (SQLite-Tabelle `job_runs`, höchstens 50 je Job).
- Jobs laufen nacheinander (eigene Sperre), unabhängig von den Abgleichen der Sync-Paare.

## Folgen
- Keine zusätzliche Abhängigkeit in Domäne und Anwendungsschicht; `jiff` nur im Daemon und in der
  CLI (Anzeige der Zeiten in Ortszeit).
- Im Container muss `TZ` gesetzt werden, sonst laufen Zeitpläne in UTC.
- Eine leere, nicht eingehängte Freigabe ist für den Job ein leerer Ordner: kein Fehler, aber
  dank „neueste Datei bleibt“ auch kein Verlust im Ziel.
- Benachrichtigungen erreichen den Nutzer vorerst über Protokoll und Ereignisstrom; Web-Oberfläche
  und Tray-App zeigen sie, sobald es sie gibt.

---

## Nachtrag (2026-10-16, Phase 6)
Verpasste Termine werden jetzt einmal direkt nach dem Start nachgeholt, wenn der Job schon einmal
lief und seitdem ein Termin verstrichen ist (z. B. Rechner aus). Ohne bisherigen Lauf wartet der
Job auf den nächsten Termin.

## English

# ADR 0008: Upload jobs with an own cron parser, local time via `jiff` and cautious retention

- Status: accepted
- Date: 2026-10-16

### Context
Phase 2 calls for general upload jobs (section 3.5): source folder, pattern, target, schedule
(cron or "on new file"), retention, verification, retries and notification. The use case is
Home Assistant backups on a server, often on an SMB/NFS share. We had to decide how schedules are
computed, how "new file" is detected without asking Proton all the time, and how retention avoids
losing data.

### Decision
- **Domain:** `UploadJob`, `CronSchedule`, `GlobPattern` and `Retention` are pure types without I/O.
  The cron parser is our own (five fields, lists, ranges, steps, names, `@daily` and so on, the
  Vixie rule for day/weekday) and works in **local time without a time zone** (`LocalDateTime`).
  A library would have pulled time zones into the domain; the rules are small and covered by
  property tests.
- **Time zone only in the daemon:** `LocalZone` (crate `jiff`, MIT/Unlicense) converts between
  timestamps and local time, with the zone from `TZ` or `/etc/localtime`, otherwise UTC. Spring gap:
  run right after it; repeated autumn hour: run once. Missed times are not made up. Long waits are
  checked against the wall clock in one-minute steps (suspend, clock changes).
- **"On new file":** a watcher on the source folder plus a check every five minutes (network shares
  often report nothing). A run happens only if the signature of the matching files (name, size,
  modification time) changed; otherwise Proton is not contacted. Files within their settle time
  lead to another run after `settle_seconds`.
- **Run (`UploadJobRunner`):** the same size and, with `verify = "hash"`, the same SHA-1 count as
  already uploaded; changed files are uploaded as a new revision. Every upload is verified.
- **Retention:** only after a run without errors, only for files matching the pattern, and the
  newest file always stays. Expired files go to the Proton trash and to protonsync's trash under
  the owner `job:<id>` and can be restored.
- **Errors:** retries after 1, 2, 4, ... minutes (at most 60) up to `max_attempts`; then the
  `JobGaveUp` event and an error in the log. Every run is recorded in the history (SQLite table
  `job_runs`, at most 50 per job).
- Jobs run one after another (own lock), independently of the sync pairs.

### Consequences
- No extra dependency in the domain and application layers; `jiff` only in the daemon and the CLI
  (to show times in local time).
- In the container `TZ` must be set, otherwise schedules run in UTC.
- An empty, unmounted share is an empty folder to the job: no error, but thanks to "the newest file
  stays" no loss in the target either.
- For now notifications reach the user through the log and the event stream; the web interface
  and the tray app will show them once they exist.

## Addendum (2026-10-16, phase 6)
Missed times are now made up once right after the start if the job has run before and a time has
passed since then (e.g. computer off). Without a previous run the job waits for the next time.

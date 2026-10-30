# ADR 0023: Warnung bei fast oder ganz vollem Speicher

- Status: angenommen
- Datum: 2026-10-30

## Kontext
Läuft die lokale Platte voll, geht nichts kaputt: Downloads landen erst in einer Temp-Datei und
werden nur vollständig an ihren Platz verschoben; SQLite schreibt transaktional. Der Abgleich
scheitert dann aber bei jedem Lauf, und im Backup-Modus wächst das `_archiv` ohne Grenze. Der
Auftraggeber möchte eine regelmäßige Prüfung mit Warnung, bevor und wenn die Platte voll ist.

## Entscheidung
- **Stufen** (Domäne `disk`): „fast voll“ unter `disk_low_free_percent` (Standard 10 %) freiem Platz,
  „voll“ unter `disk_full_free_percent` (Standard 2 %). Prozent statt fester Größen, weil die
  Platten von Desktop und Server sehr verschieden groß sind.
- **Was geprüft wird:** die lokalen Ordner aller Sync-Paare, der Fotoordner und das
  Datenverzeichnis (Zustand, Laufwerks-Caches). Ordner auf derselben Platte (gleiche
  Gerätenummer) werden zusammen gemeldet.
- **Wann:** beim Start und dann alle `disk_check_minutes` (Standard 15, `0` = aus).
- **Gemeldet wird nur ein Wechsel der Stufe** (`DiskMonitor`, Anwendungsschicht), auch der zurück zu
  „wieder genug Platz“. So kommt bei voller Platte nicht alle 15 Minuten eine Nachricht.
- **Port** `DiskSpaceProbe`; `adapter-fs::FsDiskSpace` misst unter Unix mit `statvfs` (über
  `rustix`, ohne `unsafe`). Auf anderen Plattformen meldet er „nicht unterstützt“; die Prüfung
  überspringt den Ordner dann (ADR 0018: portabel bleiben).
- **Ausgabe:** Ereignis `disk_space_changed` der Daemon-API (Protokollversion bleibt 1), Log-Eintrag
  (`warn` bzw. `info`), Desktop-Benachrichtigung der Tray-App und Mail/Webhook mit dem neuen Thema
  `disk_space` (Standard an; in der Tray-App `[notification_topics] disk_space`). Der Text kommt für
  alle aus `client::i18n::disk_space_message` (de/en).
- protonsync hält bei voller Platte nichts an; es warnt nur. Eine harte Grenze („mindestens X frei
  lassen“) wäre eine eigene Änderung.

## Folgen
- Der Nutzer erfährt rechtzeitig, dass er Platz schaffen muss, z. B. alte Versionen im `_archiv`.
- Status, CLI und Web zeigen die Stufe noch nicht dauerhaft an, nur die Benachrichtigungen.

---

## English

**ADR 0023: Warning when a disk is almost or fully full**

- Status: accepted
- Date: 2026-10-30

### Context
If the local disk runs full, nothing breaks: downloads go to a temporary file first and are only
moved into place when complete; SQLite writes transactionally. But the sync then fails at every run,
and in backup mode `_archiv` grows without limit. The client wants a regular check with a warning
before and when the disk is full.

### Decision
- **Levels** (domain `disk`): "almost full" below `disk_low_free_percent` (default 10 %) free space,
  "full" below `disk_full_free_percent` (default 2 %). Percent rather than fixed sizes, because
  desktop and server disks differ a lot in size.
- **What is checked:** the local folders of all sync pairs, the photo folder and the data directory
  (state, drive caches). Folders on the same disk (same device number) are reported together.
- **When:** at startup and then every `disk_check_minutes` (default 15, `0` = off).
- **Only a change of level is reported** (`DiskMonitor`, application layer), including the one back
  to "enough space again". So a full disk does not cause a message every 15 minutes.
- **Port** `DiskSpaceProbe`; `adapter-fs::FsDiskSpace` measures with `statvfs` on Unix (via
  `rustix`, without `unsafe`). On other platforms it reports "unsupported" and the check skips the
  folder (ADR 0018: stay portable).
- **Output:** event `disk_space_changed` of the daemon API (protocol version stays 1), a log entry
  (`warn` or `info`), a desktop notification of the tray app and mail/webhook with the new topic
  `disk_space` (on by default; in the tray app `[notification_topics] disk_space`). The text comes
  from `client::i18n::disk_space_message` for all of them (de/en).
- protonsync does not stop anything when the disk is full; it only warns. A hard limit ("always
  keep X free") would be a separate change.

### Consequences
- The user learns in time that space must be freed, e.g. old versions in `_archiv`.
- Status, CLI and web do not show the level permanently yet, only the notifications.

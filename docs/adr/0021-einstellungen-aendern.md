# ADR 0021: Wichtige Einstellungen über die Oberflächen ändern

- Status: angenommen
- Datum: 2026-10-30

## Kontext
Bisher zeigten Tray-App und Web-Oberfläche die Einstellungen des Daemons nur an. Geändert wurde
alles in der `config.toml`, danach musste der Daemon neu starten. Der Auftraggeber möchte die
wichtigsten Einstellungen, etwa den Modus (Probelauf/aktiv), direkt auf den Einstellungsseiten
ändern können. Nach CLAUDE.md 3.6/4.3 haben die Oberflächen keine eigene Geschäftslogik; alles
läuft über die Daemon-API.

## Entscheidung
- **Welche Einstellungen:** allgemein `dry_run` und `trash_retention_days`, je `[[pair]]`
  `direction`, `protect_remote` und `archive_versions`. Das sind die Schalter, die man im Betrieb
  umlegt. Alles, was Ordner, Konten, Netzwerk oder Zeitpläne betrifft (Pfade, neue Paare,
  Upload-Jobs, Laufwerke, `[photos]`, `[web]`, `[notify]`), bleibt in der Datei mit Neustart.
- **API:** `Request::Settings` liefert `SettingsInfo` (aktuelle Werte, Pfad der Datei, ob sie
  schreibbar ist). `Request::ChangeSettings { change }` nimmt eine `SettingsChange`, in der jedes
  Feld optional ist; nur gesetzte Felder ändern sich. Antwort ist wieder `SettingsInfo`. Die
  Protokollversion bleibt 1 (nur neue Anfragen).
- **Speichern zuerst, dann anwenden.** Der Daemon prüft die Änderung (Paar bekannt, mindestens ein
  Tag), schreibt sie in die Datei und übernimmt sie erst danach. Schlägt das Schreiben fehl, ändert
  sich nichts. So läuft der Daemon nie mit Werten, die ein Neustart verlöre.
- **Datei schreiben** (`daemon::config_file`, `toml_edit`): nur die geänderten Schlüssel werden
  angefasst; Kommentare, Reihenfolge und Abstände bleiben, auch der Kommentar hinter einem Wert.
  Fehlt `[general]`, entsteht es oben. Das Ergebnis wird vor dem Schreiben mit dem normalen Parser
  geprüft. Geschrieben wird über eine temporäre Datei daneben und Umbenennen (die Rechte der alten
  Datei bleiben, neue Dateien 0600); ist nur die Datei, nicht der Ordner schreibbar, wird sie direkt
  überschrieben. Ein Symlink bleibt erhalten, geändert wird sein Ziel. Fehlt die Datei, wird sie
  angelegt.
- **Sofort wirksam, ohne Neustart.** `Core` hält die allgemeinen Werte und die Optionen jedes Paars
  hinter einem `RwLock`. Eine Änderung hält dieselbe Sperre wie ein Abgleich: Ein Lauf sieht immer
  entweder die alten oder die neuen Werte. Automatische Läufe lesen `dry_run` erst nach dem Sperren.
  Ändert sich ein Paar, wird seine Verbindung neu aufgebaut (mit frisch gelesenem entfernten Baum),
  weil sie die Optionen trägt.
- **Nicht schreibbar** (z. B. Container mit schreibgeschützt eingebundener Datei): `editable =
  false`; Tray-App und Web zeigen die Werte gesperrt mit Hinweis, `ChangeSettings` antwortet mit
  `unavailable`.
- **Oberflächen:**
  - Tray-App, Seite „Einstellungen“: Auswahl für den Modus, Zahlenfeld für die Tage, je Sync-Ordner
    Auswahl der Richtung und zwei Häkchen. Jede Änderung geht sofort an den Daemon (kein
    „Speichern“-Knopf); die Logik dafür steht in `tray-model::daemon_settings`.
  - Web, Seite „Einstellungen“: ein Formular mit „Speichern“ (CSRF-geschützt, ohne JavaScript).
    Gesendet werden nur Werte, die sich vom aktuellen Stand unterscheiden, damit die Datei nur dort
    geändert wird, wo der Nutzer etwas geändert hat.
  - CLI: `protonsync config show` und `protonsync config set` (z. B. `--dry-run false`,
    `--pair main --direction backup-only`).

## Folgen
- Wer die Datei von Hand bearbeitet, während der Daemon läuft, und danach über eine Oberfläche
  etwas ändert: Der Daemon liest die Datei jedes Mal neu und ändert nur die betroffenen Schlüssel;
  die übrigen Handänderungen bleiben in der Datei, gelten aber wie bisher erst nach einem Neustart.
- Explizit gesetzte Werte stehen danach in der Datei, auch wenn sie dem Standard entsprechen.
- Ein Richtungswechsel behält den Zustand des letzten Abgleichs (Basis). Beim Wechsel von
  `two_way` auf `download_only` werden lokale Änderungen danach nicht mehr hochgeladen; beim
  Wechsel zurück gleicht der nächste Lauf sie wie gewohnt ab.
- Erweiterbar: Weitere Schlüssel lassen sich nach demselben Muster in `SettingsInfo`,
  `SettingsChange` und `config_file::apply_change` aufnehmen.

---

## English

**ADR 0021: Change important settings through the front ends**

- Status: accepted
- Date: 2026-10-30

### Context
So far the tray app and the web interface only showed the daemon's settings. Everything was changed
in `config.toml`, and the daemon had to restart afterwards. The client wants to change the most
important settings, such as the mode (dry run/active), directly on the settings pages. Following
CLAUDE.md 3.6/4.3, the front ends have no business logic of their own; everything goes through the
daemon API.

### Decision
- **Which settings:** in general `dry_run` and `trash_retention_days`, per `[[pair]]` `direction`,
  `protect_remote` and `archive_versions`. These are the switches one flips during operation.
  Everything about folders, accounts, network or schedules (paths, new pairs, upload jobs, drives,
  `[photos]`, `[web]`, `[notify]`) stays in the file with a restart.
- **API:** `Request::Settings` returns `SettingsInfo` (current values, path of the file, whether it
  is writable). `Request::ChangeSettings { change }` takes a `SettingsChange` in which every field is
  optional; only fields that are set change. The answer is `SettingsInfo` again. The protocol
  version stays 1 (only new requests).
- **Save first, then apply.** The daemon checks the change (pair known, at least one day), writes it
  into the file and only then takes it over. If writing fails, nothing changes. So the daemon never
  runs with values a restart would lose.
- **Writing the file** (`daemon::config_file`, `toml_edit`): only the changed keys are touched;
  comments, order and spacing stay, including the comment after a value. A missing `[general]` is
  created at the top. The result is checked with the normal parser before writing. Writing goes
  through a temporary file next to it and a rename (the old file's permissions stay, new files get
  0600); if only the file but not the folder is writable, it is overwritten in place. A symlink is
  kept and its target changed. A missing file is created.
- **Applies right away, without a restart.** `Core` keeps the general values and each pair's options
  behind an `RwLock`. A change holds the same lock as a sync: a run always sees either the old or the
  new values. Automatic runs read `dry_run` only after locking. If a pair changes, its connection is
  rebuilt (with a freshly read remote tree) because it carries the options.
- **Not writable** (e.g. a container with the file mounted read-only): `editable = false`; tray app
  and web show the values locked with a hint, `ChangeSettings` answers `unavailable`.
- **Front ends:**
  - Tray app, page "Settings": a choice for the mode, a number field for the days, per sync folder a
    choice of direction and two check boxes. Every change goes to the daemon at once (no "Save"
    button); the logic for it lives in `tray-model::daemon_settings`.
  - Web, page "Settings": a form with "Save" (CSRF-protected, without JavaScript). Only values that
    differ from the current state are sent, so the file only changes where the user changed
    something.
  - CLI: `protonsync config show` and `protonsync config set` (e.g. `--dry-run false`,
    `--pair main --direction backup-only`).

### Consequences
- If someone edits the file by hand while the daemon runs and then changes something through a front
  end: the daemon reads the file anew each time and only changes the affected keys; the other hand
  edits stay in the file but, as before, only apply after a restart.
- Explicitly set values are in the file afterwards, even if they equal the default.
- A change of direction keeps the state of the last sync (base). After switching from `two_way` to
  `download_only`, local changes are no longer uploaded; after switching back, the next run syncs
  them as usual.
- Extensible: further keys can be added to `SettingsInfo`, `SettingsChange` and
  `config_file::apply_change` following the same pattern.

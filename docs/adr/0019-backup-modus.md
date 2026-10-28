# ADR 0019: Backup-Modus mit Versionsarchiv (`backup_only`)

- Status: angenommen
- Datum: 2026-10-28

## Kontext
Der Auftraggeber möchte einen Modus „Backup only“: Alles, was es in Proton gibt, wird lokal
gespeichert, aber nie etwas gelöscht oder hochgeladen. Ändert sich eine vorhandene Datei, soll die
alte Version in einen Archivordner am selben Ort wandern, mit einem Zeitstempel als Präfix im
Namen, sodass beliebig viele Versionen einer Datei entstehen können.
`download_only` reicht dafür nicht: Dort werden in Proton gelöschte Dateien lokal in den
Papierkorb gelegt (und nach der Aufbewahrungszeit endgültig gelöscht), und eine neue Version
überschreibt die alte lokale Datei.

## Entscheidung
- Neue Richtung `direction = "backup_only"` je `[[pair]]` (`SyncDirection::BackupOnly`).
  Die Regeln stehen in der Domäne als `conflict::decide_backup` (Property-Tests):

  | lokal \ Proton | unverändert | geändert / neu | gelöscht |
  |---|---|---|---|
  | unverändert | nichts | archivieren, dann laden | lokale Kopie bleibt |
  | geändert / neu | nichts | archivieren, dann laden (gleicher Inhalt: nur Basis) | lokale Kopie bleibt |
  | gelöscht | aus Proton wiederherstellen | laden | nur Basis entfernen |

- **Nie hochladen, nie löschen, keine Konflikte.** Die Sync-Engine erzeugt in diesem Modus weder
  `Upload` noch `TrashRemote`, `TrashLocal` oder `Conflict`. Es geht trotzdem keine Version
  verloren, weil jede lokale Datei vor dem Ersetzen archiviert wird.
- **Archiv:** Vor dem Herunterladen einer neueren Version wird die lokale Datei nach
  `_archiv/` im selben Ordner verschoben (neue Aktion `ArchiveAndDownload`), z. B.
  `Dokumente/bericht.pdf` → `Dokumente/_archiv/2026-10-28_23-06-40_bericht.pdf`.
  - Präfix: Zeitpunkt des Archivierens in **UTC**, `JJJJ-MM-TT_HH-MM-SS_`. Ohne Doppelpunkte
    (SMB/Windows-tauglich), alphabetisch = chronologisch sortiert. In der Nachricht stand
    „mit dem und ts Präfix“; gelesen als „mit Datum und Zeitstempel als Präfix“.
  - Zwei Versionen in derselben Sekunde: `… (2).pdf`, `… (3).pdf`.
  - Der Ordnername `_archiv` bleibt deutsch (sichtbarer Name wie „Konflikt“, ADR 0014).
- **In Proton gelöscht:** Die lokale Datei (bzw. der ganze Ordner) bleibt liegen; nur die Basis
  wird entfernt (Aktion `KeepLocalCopy`). Danach ist der Eintrag eine rein lokale Datei und taucht
  in keinem Plan mehr auf. Erscheint später in Proton wieder eine Datei mit demselben Namen,
  wird die lokale Kopie archiviert und die neue geladen (bei gleichem Inhalt nur verknüpft).
- **Lokale Änderungen** gehen nie nach Proton. Rein lokale Dateien und lokale Bearbeitungen
  bleiben unangetastet, bis sich die Datei in Proton ändert; dann wird die lokale Fassung
  archiviert. Eine lokal gelöschte, in Proton unveränderte Datei wird wiederhergestellt, damit die
  Sicherung vollständig bleibt.
- Nur für `[[pair]]`. In `[photos]` wird `backup_only` abgelehnt: Fotos liegen in eigenen
  Monatsordnern und werden ohnehin nie als neue Version ersetzt (`download_only` passt dort).
  FUSE-Laufwerke haben keine Richtung.
- `protect_remote` ist mit `backup_only` erlaubt, aber wirkungslos (es wird nichts übertragen).
- Im Bericht zählt „archivieren, dann laden“ als heruntergeladen; der Probelauf zeigt die
  Archivierung als eigenen Schritt.

## Folgen
- Das Archiv wächst ohne Grenze; es gibt (noch) keine Aufbewahrungsregel dafür. Wer Platz braucht,
  löscht alte Dateien in `_archiv` von Hand; protonsync stellt sie nicht wieder her (rein lokal).
- Die `_archiv`-Ordner liegen im Sync-Ordner. Stellt man das Paar später auf `two_way` oder
  `upload_only` um, würden sie hochgeladen. Vorher verschieben oder löschen.
- Neue Werte in API, CLI, Web und Tray-App (`backup_only`, `archive_and_download`,
  `keep_local_copy`). Ältere Oberflächen kennen sie nicht; Daemon und Oberflächen werden
  gemeinsam aktualisiert (wie bei ADR 0016).

---

## English

**ADR 0019: Backup mode with version archive (`backup_only`)**

- Status: accepted
- Date: 2026-10-28

### Context
The client wants a "backup only" mode: everything that exists in Proton is stored locally, but
nothing is ever deleted or uploaded. When an existing file changes, the old version should move to
an archive folder in the same place, with a timestamp prefix in its name, so that any number of
versions of a file can pile up. `download_only` does not fit: there, files deleted in Proton are
moved to the local trash (and deleted for good after the retention period), and a new version
overwrites the old local file.

### Decision
- New direction `direction = "backup_only"` per `[[pair]]` (`SyncDirection::BackupOnly`). The rules
  live in the domain as `conflict::decide_backup` (property tests):

  | local \ Proton | unchanged | changed / new | deleted |
  |---|---|---|---|
  | unchanged | nothing | archive, then download | local copy stays |
  | changed / new | nothing | archive, then download (same content: base only) | local copy stays |
  | deleted | restore from Proton | download | remove base only |

- **Never upload, never delete, no conflicts.** In this mode the sync engine produces neither
  `Upload` nor `TrashRemote`, `TrashLocal` or `Conflict`. Still no version is lost, because every
  local file is archived before it is replaced.
- **Archive:** before a newer version is downloaded, the local file is moved to `_archiv/` in the
  same folder (new action `ArchiveAndDownload`), e.g. `Dokumente/report.pdf` →
  `Dokumente/_archiv/2026-10-28_23-06-40_report.pdf`.
  - Prefix: the time of archiving in **UTC**, `YYYY-MM-DD_HH-MM-SS_`. No colons (works on
    SMB/Windows), alphabetical = chronological order. The request said "mit dem und ts Präfix";
    read as "with a date and timestamp prefix".
  - Two versions within the same second: `… (2).pdf`, `… (3).pdf`.
  - The folder name `_archiv` stays German (a visible name like "Konflikt", ADR 0014).
- **Deleted in Proton:** the local file (or the whole folder) stays; only the base is removed
  (action `KeepLocalCopy`). After that the entry is a purely local file and no longer appears in
  any plan. If a file with the same name appears in Proton again later, the local copy is archived
  and the new one downloaded (with identical content it is only linked).
- **Local changes** never reach Proton. Purely local files and local edits stay untouched until
  the file changes in Proton; then the local version is archived. A file deleted locally but
  unchanged in Proton is restored, so the backup stays complete.
- Only for `[[pair]]`. `[photos]` refuses `backup_only`: photos live in their own month folders and
  are never replaced by a new version anyway (`download_only` fits there). FUSE drives have no
  direction.
- `protect_remote` is allowed together with `backup_only` but has no effect (nothing is transferred).
- In the report "archive, then download" counts as downloaded; the dry run shows the archiving as
  a step of its own.

### Consequences
- The archive grows without limit; there is no retention rule for it (yet). To free space, delete
  old files in `_archiv` by hand; protonsync does not restore them (purely local).
- The `_archiv` folders live inside the sync folder. If the pair is later switched to `two_way` or
  `upload_only`, they would be uploaded. Move or delete them first.
- New values in API, CLI, web and tray app (`backup_only`, `archive_and_download`,
  `keep_local_copy`). Older front ends do not know them; daemon and front ends are updated together
  (as with ADR 0016).

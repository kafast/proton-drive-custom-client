# ADR 0020: Versionsarchiv auf beiden Seiten (`archive_versions`)

- Status: angenommen
- Datum: 2026-10-29

## Kontext
Der Auftraggeber möchte für den normalen Abgleich eine Option, bei der nichts gelöscht, sondern
immer nur archiviert wird, wie beim Backup-Modus (ADR 0019), aber **online und lokal**. Bisher gilt
im Zwei-Wege-Abgleich: Löschungen landen im Papierkorb der jeweils anderen Seite und werden nach
`trash_retention_days` endgültig gelöscht; eine neue Version ersetzt die alte (lokal wird
überschrieben, in Proton entsteht eine neue Revision, deren Vorgänger Proton nach eigenen Regeln
aufräumt).

## Entscheidung
- Neuer Schlüssel `archive_versions = true` je `[[pair]]` (Standard `false`), Feld
  `SyncPair::archive_versions`. Er gilt für jede Richtung; mit `backup_only` ändert er nichts
  (dort wird ohnehin nur lokal archiviert). In `[photos]` gibt es ihn nicht (unbekannter Schlüssel).
- Die Regel steht in der Domäne als `conflict::archive_instead` (Property-Tests) und wird nach
  `decide` und `protect_remote` angewendet:

  | ohne Archiv | mit `archive_versions` |
  |---|---|
  | `TrashLocal` (in Proton gelöscht) | `ArchiveLocal`: lokale Datei bzw. Ordner nach `_archiv/` |
  | `TrashRemote` (lokal gelöscht) | `ArchiveRemote`: Knoten in Proton nach `_archiv/` |
  | `Download` über eine vorhandene lokale Datei | `ArchiveAndDownload` (wie ADR 0019) |
  | `Upload` über eine vorhandene Datei in Proton | `ArchiveRemoteAndUpload`: alte Datei in Proton nach `_archiv/`, dann neu hochladen |

  Alles andere bleibt, insbesondere Konflikte (sie behalten ohnehin beide Versionen) und neue
  Dateien. Wer gewinnt und was die Richtung erlaubt, ändert sich nie (Property-Test).
- **Archiv:** Ordner `_archiv` im selben Ordner wie das Original, Name mit UTC-Zeitstempel davor
  (`JJJJ-MM-TT_HH-MM-SS_`, Zähler ` (2)` bei gleicher Sekunde), genau wie in ADR 0019 und mit
  derselben Funktion `archive::archived_name`. Gelöschte Ordner wandern als Ganzes.
- **In Proton** wird der vorhandene Knoten verschoben, nicht kopiert: erst `move_node` in den
  Archivordner, dann `rename` auf den Zeitstempel-Namen. So bleibt die Revisionsgeschichte des
  Knotens erhalten, und ein Abbruch zwischen den beiden Schritten lässt den Knoten im (nicht
  synchronisierten) Archiv statt neben dem Original. Ist der Originalname im Archiv schon belegt,
  wird zuerst umbenannt und dann verschoben. Fehlt der Ordner `_archiv`, wird er angelegt; ein
  schon vorhandener wird wiederverwendet. Steht dort eine **Datei** namens `_archiv`, schlägt der
  Schritt fehl und nichts wird gelöscht; der nächste Lauf versucht es erneut.
- **Die Archive werden nicht synchronisiert.** Bei eingeschalteter Option entfernt die Sync-Engine
  alle Pfade mit einem Bestandteil `_archiv` aus lokaler Sicht, entfernter Sicht und Basis
  (`archive::is_archived`). Jede Seite behält ihr eigenes Archiv: Was in Proton gelöscht wurde,
  liegt im lokalen Archiv, was lokal gelöscht wurde, im Archiv in Proton. Sonst würde jede
  Archivierung auf die andere Seite kopiert, und das Aufräumen eines Archivs wäre wieder eine
  Löschung, die archiviert würde.
- **Gleicher Inhalt** (z. B. nur die Änderungszeit ist neu, gleiche SHA-1) wird nicht archiviert;
  nur die Basis wird aktualisiert. Sonst füllte sich das Archiv mit identischen Kopien.
- `protect_remote` ist strenger und hat Vorrang: lokale Löschungen gehen dann gar nicht nach Proton,
  Ersetzungen werden Konflikte.
- **Konflikte lösen:** `keep-local`/`keep-remote` legen die verworfene lokale Fassung wie bisher in
  den lokalen Papierkorb (bewusste Entscheidung des Nutzers, 30 Tage wiederherstellbar). Die daraus
  folgende Ersetzung bzw. Löschung in Proton läuft beim nächsten Abgleich über das Archiv.
- Neue API-Werte `archive_local`, `archive_remote`, `archive_remote_and_upload` und die Zahl
  `archived` im Ergebnis eines Abgleichs (`#[serde(default)]`, ältere Daemons liefern sie nicht).
  CLI, Web und Tray-App zeigen sie an.

## Folgen
- Die Archive wachsen ohne Grenze, lokal und im Proton-Speicherplatz; es gibt (noch) keine
  Aufbewahrungsregel. Aufräumen von Hand; protonsync gibt das nicht weiter.
- Ein eigener Ordner namens `_archiv` wird bei eingeschalteter Option nicht mehr synchronisiert.
- Schaltet man die Option später ab, werden die `_archiv`-Ordner wie normale Ordner abgeglichen
  (in beide Richtungen kopiert). Vorher verschieben oder löschen.
- Proton kann neue Ordner direkt in „Meine Dateien“ ablehnen (HTTP 422, Handbuch 9.2). Liegt ein
  Paar direkt auf „Meine Dateien“, kann das Anlegen von `_archiv` dort scheitern; dann bleibt die
  Datei in Proton erhalten und der Schritt erscheint als Fehler.
- Wiederherstellen heißt: Datei aus `_archiv` zurückverschieben und den Zeitstempel entfernen.

---

## English

**ADR 0020: Version archive on both sides (`archive_versions`)**

- Status: accepted
- Date: 2026-10-29

### Context
The client wants an option for the normal sync where nothing is deleted but always only archived,
as in the backup mode (ADR 0019), but **online and locally**. So far, two-way sync moves deletions
to the trash of the other side, where they are deleted for good after `trash_retention_days`; a new
version replaces the old one (overwritten locally, a new revision in Proton whose predecessors
Proton cleans up by its own rules).

### Decision
- New key `archive_versions = true` per `[[pair]]` (default `false`), field
  `SyncPair::archive_versions`. It applies to every direction; with `backup_only` it changes nothing
  (that mode only archives locally anyway). `[photos]` does not have it (unknown key).
- The rule lives in the domain as `conflict::archive_instead` (property tests) and is applied after
  `decide` and `protect_remote`:

  | without archive | with `archive_versions` |
  |---|---|
  | `TrashLocal` (deleted in Proton) | `ArchiveLocal`: local file or folder into `_archiv/` |
  | `TrashRemote` (deleted locally) | `ArchiveRemote`: node in Proton into `_archiv/` |
  | `Download` over an existing local file | `ArchiveAndDownload` (as in ADR 0019) |
  | `Upload` over an existing file in Proton | `ArchiveRemoteAndUpload`: old file in Proton into `_archiv/`, then upload anew |

  Everything else stays, in particular conflicts (they keep both versions anyway) and new files.
  Which side wins and what the direction allows never changes (property test).
- **Archive:** folder `_archiv` in the same folder as the original, name prefixed with a UTC
  timestamp (`YYYY-MM-DD_HH-MM-SS_`, counter ` (2)` within the same second), exactly as in ADR 0019
  and with the same function `archive::archived_name`. Deleted folders move as a whole.
- **In Proton** the existing node is moved, not copied: first `move_node` into the archive folder,
  then `rename` to the timestamped name. This keeps the node's revision history, and an interruption
  between the two steps leaves the node inside the (not synced) archive instead of next to the
  original. If the original name is already taken in the archive, it is renamed first and moved
  afterwards. A missing `_archiv` folder is created, an existing one reused. If a **file** named
  `_archiv` is in the way, the step fails and nothing is deleted; the next run tries again.
- **The archives are not synced.** With the option on, the sync engine drops every path with a
  component `_archiv` from the local view, the remote view and the base (`archive::is_archived`).
  Each side keeps its own archive: what was deleted in Proton is in the local archive, what was
  deleted locally is in the archive in Proton. Otherwise every archiving would be copied to the
  other side, and cleaning up an archive would again be a deletion that gets archived.
- **Identical content** (e.g. only the modification time is new, same SHA-1) is not archived; only
  the base is updated. Otherwise the archive would fill up with identical copies.
- `protect_remote` is stricter and takes precedence: local deletions then do not reach Proton at all,
  replacements become conflicts.
- **Resolving conflicts:** `keep-local`/`keep-remote` still put the discarded local version into the
  local trash (a deliberate user decision, restorable for 30 days). The resulting replacement or
  deletion in Proton goes through the archive on the next sync.
- New API values `archive_local`, `archive_remote`, `archive_remote_and_upload` and the count
  `archived` in a sync result (`#[serde(default)]`, older daemons do not send it). CLI, web and tray
  app show them.

### Consequences
- The archives grow without limit, locally and in the Proton storage quota; there is no retention
  rule (yet). Clean up by hand; protonsync does not pass that on.
- An own folder named `_archiv` is no longer synced while the option is on.
- If the option is switched off later, the `_archiv` folders are synced like normal folders (copied
  in both directions). Move or delete them first.
- Proton may refuse new folders directly in "My files" (HTTP 422, manual 9.2). If a pair sits
  directly on "My files", creating `_archiv` there may fail; the file then stays in Proton and the
  step shows up as a failure.
- Restoring means: move the file back out of `_archiv` and remove the timestamp.

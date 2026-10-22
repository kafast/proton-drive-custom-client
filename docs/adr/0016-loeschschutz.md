# ADR 0016: Löschschutz für Proton (`protect_remote`)

- Status: angenommen
- Datum: 2026-10-22

## Kontext
Der Auftraggeber möchte eine Option, mit der ein Sync-Paar in Proton nichts löscht und nichts
ersetzt: Lokal wird normal synchronisiert, aber eine lokale Löschung darf nicht nach Proton
gelangen. Entsteht lokal später wieder eine Datei mit demselben Namen, die es in Proton schon gibt,
soll das ein Konflikt werden. Die Einbahnstraße `upload_only`/`download_only` passt nicht, weil
neue Dateien weiterhin hochgeladen und Änderungen aus Proton weiterhin geladen werden sollen.

## Entscheidung
- Neuer Schlüssel `protect_remote` (Standard `false`) je `[[pair]]` und in `[photos]`.
- Die Regel steht in der Domäne als `conflict::protect_remote` und wird nach der
  Drei-Wege-Entscheidung angewendet:
  - `TrashRemote` (lokal gelöscht) wird zu `IgnoreLocalChange`: die Datei bleibt in Proton,
    lokal bleibt sie gelöscht. Die Basis bleibt bestehen, damit der Fall wiedererkannt wird.
  - `Upload` auf einen Pfad, den es in Proton gibt (lokal bearbeitet oder gelöscht und neu
    angelegt), wird zu einem Konflikt der neuen Art `LocalModifiedRemoteProtected`. Er wird
    wie „beide geändert“ behandelt: Proton-Version unter dem Originalnamen, lokale Version als
    Konfliktkopie, die als neue Datei hochgeladen wird. Es geht nichts verloren und nichts in
    Proton wird überschrieben.
  - Ist der Inhalt gleich (z. B. eine gelöschte Datei wurde unverändert zurückkopiert), entsteht
    kein Konflikt; nur die Basis wird aktualisiert.
  - Neue Dateien werden hochgeladen, Änderungen und Löschungen aus Proton wirken lokal wie bisher.
- Lokale Bearbeitungen bestehender Dateien zählen ebenfalls als „Ersetzen“ und werden zum
  Konflikt (vom Auftraggeber am 2026-10-22 so entschieden).
- Beim Lösen eines solchen Konflikts ist `keep-local` gesperrt (Fehler `RemoteProtected`), weil
  der nächste Abgleich die geschützte Datei ohnehin nicht ersetzen würde. `keep-remote` und
  `keep-both` gehen. Wer die lokale Version wirklich in Proton haben will, schaltet den Schutz
  für das Paar kurz ab.
- Fotos: Bearbeitete Fotos werden ohnehin nie als neue Version hochgeladen; der Schutz verhindert
  dort nur das Verschieben lokal gelöschter Fotos in den Proton-Papierkorb.
- Nicht betroffen: FUSE-Laufwerke (Löschen im Laufwerk ist ausdrücklich ein Löschen in Proton)
  und die Aufbewahrungsregel der Upload-Jobs (eigene Einstellung je Job).

## Folgen
- Ein lokal gelöschter, geschützter Eintrag bleibt dauerhaft im Plan (`ignored`), wie bei
  `download_only`. Der Probelauf zeigt ihn als „lokale Änderung bleibt lokal“.
- Mit eingeschaltetem Schutz erzeugt jede lokale Bearbeitung einer vorhandenen Datei einen
  Konflikt. Das ist gewollt, aber laut; Benachrichtigungen bei Konflikten folgen (ADR 0017).
- Neue Konfliktart in API, CLI, Web und Tray-App (`local_modified_remote_protected`).

---

## English

**ADR 0016: Protection for Proton (`protect_remote`)**

- Status: accepted
- Date: 2026-10-22

### Context
The client wants an option under which a sync pair never deletes or replaces anything in Proton:
local syncing works as usual, but a local deletion must not reach Proton. If a file with the same
name as one already in Proton appears locally later, that should become a conflict. The one-way
directions `upload_only`/`download_only` do not fit, because new files should still be uploaded and
changes from Proton should still be downloaded.

### Decision
- New key `protect_remote` (default `false`) per `[[pair]]` and in `[photos]`.
- The rule lives in the domain as `conflict::protect_remote` and is applied after the three-way
  decision:
  - `TrashRemote` (deleted locally) becomes `IgnoreLocalChange`: the file stays in Proton and stays
    deleted locally. The base is kept so the case is recognised again.
  - `Upload` to a path that exists in Proton (edited locally, or deleted and created again) becomes
    a conflict of the new kind `LocalModifiedRemoteProtected`. It is handled like "both modified":
    the Proton version under the original name, the local version as a conflict copy that is
    uploaded as a new file. Nothing is lost and nothing in Proton is overwritten.
  - If the content is the same (e.g. a deleted file was copied back unchanged), there is no
    conflict; only the base is updated.
  - New files are uploaded, changes and deletions from Proton apply locally as before.
- Local edits of existing files also count as "replacing" and become conflicts (decided this way by
  the client on 2026-10-22).
- When resolving such a conflict, `keep-local` is refused (error `RemoteProtected`), because the
  next sync would not replace the protected file anyway. `keep-remote` and `keep-both` work.
  To really get the local version into Proton, turn the protection off for the pair briefly.
- Photos: edited photos are never uploaded as a new version anyway; there the protection only
  stops moving locally deleted photos to the Proton trash.
- Not affected: FUSE drives (deleting in the drive is explicitly deleting in Proton) and the
  retention rule of upload jobs (a setting per job).

### Consequences
- A protected entry deleted locally stays in the plan for good (`ignored`), as with
  `download_only`. The dry run shows it as "local change stays local".
- With the protection on, every local edit of an existing file creates a conflict. This is
  intended but noisy; notifications on conflicts follow (ADR 0017).
- New conflict kind in API, CLI, web and tray app (`local_modified_remote_protected`).

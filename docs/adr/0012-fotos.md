# ADR 0012: Fotos als eigener Abgleich mit Drei-Wege-Regel, Duplikate nur als Vorschlag

- Status: angenommen
- Datum: 2026-10-16

## Kontext
Phase 5 verlangt (Abschnitt 3.4) die Anbindung an den **separaten Proton-Photos-Bereich**:
Fotos (meist vom Handy) werden je nach Einstellung in einen lokalen Fotoordner geladen
und/oder von dort hochgeladen. Dazu kommt eine **getrennte Duplikaterkennung**, die nur
Vorschläge macht und nie selbst löscht.

Was `proton-drive-rs` 0.9.1 dafür bietet (Quellcode geprüft, `ProtonPhotosClient`):
- **Lesen:** Zeitleiste (`enumerate_timeline`: Kennung und Aufnahmezeit, seitenweise),
  Metadaten in Stapeln (`enumerate_nodes`) mit Name, Medientyp, Größe, aktiver Revision,
  SHA-1 des Klartexts, Aufnahmezeit, Markierungen (Favorit, Video, Live-Foto …), Alben und
  zusammengehörigen Fotos (Live-Foto = Bild + Video, Serienbilder). Herunterladen ganz oder
  als Datenstrom.
- **Schreiben:** `upload_photo_from` (Datenstrom, mit Aufnahmezeit und Markierungen),
  Papierkorb, Wiederherstellen und endgültiges Löschen über den Drive-Client (die Knoten-
  kennung enthält das Volume), Ereignisse über den Bereich des Foto-Volumes.
- **Duplikate:** `find_duplicates_many` (Name + SHA-1, serverseitig über HMAC).
- **Fehlt noch:** Anlegen des Foto-Volumes (ein Konto ohne Fotos hat keins; dann gibt es nichts
  abzugleichen, und ein Upload ist erst möglich, wenn eine offizielle App das Volume angelegt hat),
  Alben schreiben in geteilten Alben, neue Revisionen eines Fotos.

## Entscheidung
- **Eigener Port `PhotoLibrary`** statt `RemoteStorage`: Fotos bilden keinen Ordnerbaum, sondern
  eine flache Zeitleiste; Umbenennen und Verschieben gibt es dort nicht. Der Port bietet Auflisten,
  einzelnes Foto, Ereignisse, Herunterladen, Hochladen, Papierkorb, Wiederherstellen und
  endgültiges Löschen. `FakePhotoLibrary` und gemeinsame Contract-Tests wie bei den anderen Ports.
- **Lokale Ordnerstruktur `JJJJ/MM/Name`** nach Aufnahmezeit in der Zeitzone des Systems (wie
  bei den Upload-Jobs, `TZ`). Gibt es den Namen im Monat schon, bekommt das Foto die ersten
  8 Zeichen seiner Kennung angehängt (`IMG_0001 (a1b2c3d4).jpg`). Der gewählte Pfad wird
  gespeichert und ändert sich danach nicht mehr.
- **Dieselbe Drei-Wege-Regel wie der Datei-Abgleich** (`domain::conflict::decide`), je Foto mit
  Basis (Revision in Proton, Größe und Änderungszeit lokal). Die Basis liegt in der vorhandenen
  Tabelle unter dem Besitzer `photos:library`; es gibt keine neue Tabelle.
  - Richtung `download` (Standard, Backup der Handy-Fotos), `upload` oder `two_way`.
  - In Proton gelöscht → lokale Kopie in den lokalen Papierkorb; lokal gelöscht (nur bei
    `two_way`) → Foto in den Proton-Papierkorb. Beides erscheint im gemeinsamen Papierkorb
    (`protonsync trash`) und läuft nach der eingestellten Frist ab.
  - **Neue lokale Dateien** werden vor dem Hochladen mit SHA-1 und Größe gegen die Bibliothek
    geprüft; ist das Foto schon da, wird es nur verknüpft und nicht doppelt hochgeladen.
    Aufnahmezeit beim Hochladen ist vorerst die Änderungszeit der Datei (EXIF folgt später).
  - **Lokal bearbeitete Fotos** werden nicht hochgeladen, weil Photos keine neuen Revisionen
    kennt; sie werden gemeldet. Wurde dasselbe Foto auch in Proton geändert, bleiben beide:
    die lokale Datei bleibt liegen, die Proton-Version kommt unter einem zweiten Namen dazu.
    Damit wird nie etwas still überschrieben, eine eigene Konfliktwarteschlange braucht es nicht.
  - Nur Mediendateien zählen (Bilder, RAW, Videos nach Endung); alles andere im Fotoordner
    bleibt unberührt.
- **Ereignisse statt Durchlaufen:** Die Bibliothek wird beim ersten Lauf nach dem Start einmal
  vollständig gelesen und im Speicher gehalten; danach holt der Daemon nur die Ereignisse des
  Foto-Volumes (mindestens 30 s Abstand wie beim Datei-Abgleich) und liest geänderte Fotos
  einzeln nach. Bei einer Lücke im Ereignisstrom und einmal täglich als Sicherheitsnetz wird
  vollständig neu gelesen.
- **Duplikaterkennung als eigene Funktion** (`protonsync photos duplicates`), rein in der Domäne:
  - *Gleicher Inhalt:* gleiche SHA-1 und Größe (sicher).
  - *Wahrscheinlich gleich:* gleicher Name (ohne Groß-/Kleinschreibung) und gleiche
    Aufnahmesekunde, aber anderer Inhalt (z. B. neu komprimiert).
  - Zugehörige Fotos (der Video-Teil eines Live-Fotos) zählen nicht als Duplikat.
  - Vorschlag, welches Foto bleibt: Favorit vor Album-Mitglied vor dem zuerst hochgeladenen.
  - **Löschen nur auf ausdrücklichen Befehl** mit den Kennungen aus der Liste
    (`protonsync photos duplicates trash ID… --apply`), ohne `--apply` nur Anzeige; die Fotos
    gehen in den Proton-Papierkorb und in den eigenen Papierkorb, nie direkt ins endgültige
    Löschen. Abgelehnt werden Kennungen, die in keiner aktuellen Gruppe stehen, und Befehle, die
    alle Fotos einer Gruppe löschen würden: Mindestens eines bleibt immer.

## Folgen
- Kein neues Datenbankschema; Papierkorb, Probelauf und Ereignisse verhalten sich wie beim
  Datei-Abgleich.
- Ein Neustart des Daemons liest die Bibliothek einmal ganz (bei 10 000 Fotos einige Dutzend
  Anfragen), danach nur Ereignisse.
- Offene Punkte: Aufnahmezeit aus EXIF beim Hochladen, Alben lokal abbilden, Vorschaubilder,
  Anlegen des Foto-Volumes, sobald die Community-Crates es können.

## English

# ADR 0012: Photos as a separate sync with the three-way rule, duplicates only as suggestions

- Status: accepted
- Date: 2026-10-16

## Context
Phase 5 (section 3.4) asks for the **separate Proton Photos area**: photos (mostly from the
phone) are downloaded into a local photo folder and/or uploaded from it, depending on the
setting. On top comes a **separate duplicate finder** that only suggests and never deletes.

What `proton-drive-rs` 0.9.1 offers (source checked, `ProtonPhotosClient`):
- **Reading:** the timeline (`enumerate_timeline`: id and capture time, paged), metadata in
  batches (`enumerate_nodes`) with name, media type, size, active revision, SHA-1 of the
  plaintext, capture time, tags (favourite, video, live photo …), albums and related photos
  (live photo = picture + video, bursts). Download whole or as a stream.
- **Writing:** `upload_photo_from` (streaming, with capture time and tags), trash, restore and
  permanent deletion through the Drive client (the node id carries the volume), events on the
  photo volume's scope.
- **Duplicates:** `find_duplicates_many` (name + SHA-1, server side via HMAC).
- **Still missing:** creating the photo volume (an account without photos has none; then there
  is nothing to sync, and uploading only works once an official app has created the volume),
  writing shared albums, new revisions of a photo.

## Decision
- **Own port `PhotoLibrary`** instead of `RemoteStorage`: photos are not a folder tree but a flat
  timeline without renames or moves. The port offers listing, a single photo, events, download,
  upload, trash, restore and permanent deletion. `FakePhotoLibrary` and shared contract tests as
  for the other ports.
- **Local layout `YYYY/MM/name`** by capture time in the system time zone (as for upload jobs,
  `TZ`). If the name already exists in that month, the first 8 characters of the photo id are
  appended (`IMG_0001 (a1b2c3d4).jpg`). The chosen path is stored and never changes afterwards.
- **The same three-way rule as the file sync** (`domain::conflict::decide`), per photo with a
  base (revision in Proton, size and modification time locally). The base lives in the existing
  table under the owner `photos:library`; there is no new table.
  - Direction `download` (default, backup of the phone photos), `upload` or `two_way`.
  - Deleted in Proton → local copy into the local trash; deleted locally (only with `two_way`) →
    photo into the Proton trash. Both appear in the shared trash (`protonsync trash`) and expire
    after the configured period.
  - **New local files** are checked against the library by SHA-1 and size before uploading; if
    the photo is already there it is only linked, not uploaded twice. The capture time for
    uploads is the file's modification time for now (EXIF comes later).
  - **Photos edited locally** are not uploaded, because Photos has no new revisions; they are
    reported. If the same photo also changed in Proton, both stay: the local file remains and the
    Proton version is added under a second name. Nothing is ever overwritten silently, so no
    separate conflict queue is needed.
  - Only media files count (pictures, RAW, videos by extension); everything else in the photo
    folder is left alone.
- **Events instead of traversal:** the library is read completely once on the first run after
  start-up and kept in memory; afterwards the daemon only fetches the photo volume's events (at
  least 30 s apart, as for the file sync) and re-reads changed photos one by one. After a gap in
  the event stream and once a day as a safety net it reads everything again.
- **Duplicate finder as its own function** (`protonsync photos duplicates`), purely in the domain:
  - *Same content:* same SHA-1 and size (certain).
  - *Probably the same:* same name (ignoring case) and same capture second, but different content
    (e.g. recompressed).
  - Related photos (the video part of a live photo) do not count as duplicates.
  - Suggestion of which photo to keep: favourite before album member before the first uploaded.
  - **Deleting only on an explicit command** with the ids from the list
    (`protonsync photos duplicates trash ID… --apply`), without `--apply` it only shows; the photos
    go into the Proton trash and into our own trash, never straight to permanent deletion. Ids
    that are in no current group are refused, and so is a command that would delete every photo
    of a group: at least one always stays.

## Consequences
- No new database schema; trash, dry run and events behave as for the file sync.
- A daemon restart reads the library once completely (a few dozen requests for 10,000 photos),
  afterwards only events.
- Open points: capture time from EXIF when uploading, mirroring albums locally, thumbnails,
  creating the photo volume once the community crates can.

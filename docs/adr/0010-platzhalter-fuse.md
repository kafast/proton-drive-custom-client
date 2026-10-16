# ADR 0010: Platzhalter über einen FUSE-Mount mit Block-Zwischenspeicher

- Status: angenommen
- Datum: 2026-10-16

## Kontext
Phase 3 verlangt Dateien „nur bei Bedarf“ (Abschnitt 3.3): Ein virtuelles Laufwerk zeigt sofort
alle Dateien, der Inhalt wird beim Öffnen geladen und lokal zwischengespeichert. Dazu „Immer lokal
halten“ pro Ordner/Datei und eine automatische Freigabe nach X Tagen, die nur greifen darf, wenn
die Version in Proton aktuell und vollständig ist. Proton verlangt Ereignisse statt rekursivem
Durchlaufen. Die Proton-Crates liefern Byte-Bereiche (`download_range`).

## Entscheidung
- **Platzhalter gibt es nur im FUSE-Mount.** Linux hat keine Platzhalter-Schnittstelle für normale
  Ordner (anders als die Cloud Files API unter Windows). Sync-Ordner bleiben vollständig lokal;
  „Immer lokal halten“ und die Freigabe betreffen den Zwischenspeicher eines Mounts.
- **Konfiguration:** beliebig viele `[[mount]]` (Kennung, Mountpunkt, Ordner in Proton,
  `keep_local` als Voreinstellung für „Immer lokal halten“, Standard aus) und ein globaler
  Abschnitt `[cache]` (Freigabe nach Tagen, Standard 30; Größenlimit, Standard 10 GB).
- **Block-Zwischenspeicher:** Jede Datei wird in Blöcken zu 4 MiB in einer dünn belegten (sparse)
  Datei im Datenverzeichnis abgelegt. Welche Blöcke vorhanden sind, steht als Bitmap in SQLite.
  Lesen holt nur fehlende Blöcke (plus einen Block Vorauslesen). Große Dateien (Videos) öffnen so
  sofort. Ändert sich die Revision in Proton, wird der Eintrag verworfen, sofern er nicht lokal
  geändert ist.
- **Metadaten bei Bedarf:** Ein Ordner wird erst beim ersten Auflisten bei Proton gelesen und dann
  im Speicher gehalten; Proton-Ereignisse halten ihn aktuell. Inode-Nummern gelten nur während des
  Mounts.
- **Schreiben:** Beim Öffnen zum Schreiben werden fehlende Blöcke zuerst geladen; geschrieben wird
  in den Zwischenspeicher, der Eintrag ist dann „geändert“ (in SQLite vermerkt, übersteht Neustart
  und Absturz). Nach dem letzten Schließen wird hochgeladen (neue Revision bzw. neue Datei). Hat
  sich die Revision in Proton inzwischen geändert, wird die lokale Fassung als Konfliktkopie
  hochgeladen; überschrieben wird nie.
- **Löschen und Umbenennen:** Löschen legt den Knoten in den Proton-Papierkorb und trägt ihn in
  den Papierkorb von protonsync ein (Besitzer `mount:<id>`), wiederherstellbar mit
  `protonsync trash restore`. Umbenennen und Verschieben gehen direkt an Proton.
- **Anheften:** Regeln pro Pfad in SQLite; die nächstgelegene Regel (Datei, dann Elternordner bis
  zur Wurzel) gilt, sonst `keep_local`. Angeheftete Dateien lädt der Daemon im Hintergrund
  vollständig.
- **Freigabe:** Kandidaten sind nur Einträge, die nicht angeheftet, nicht geändert und nicht
  geöffnet sind: zuerst die seit X Tagen unbenutzten, dann, solange das Größenlimit überschritten
  ist, die am längsten unbenutzten. Lokal unveränderte Inhalte stammen aus Proton oder wurden
  vollständig hochgeladen; vor dem Entfernen prüft der Daemon nur noch, dass die Datei in Proton
  weiterhin existiert (eine neuere Revision dort ist ebenfalls sicher und wird bei Bedarf neu
  geladen). Mit `dry_run = true` wird nur geplant.
- **Technik:** Die Anwendungsschicht bekommt ein FUSE-unabhängiges `VirtualDrive` (Nachschlagen,
  Auflisten, Lesen, Schreiben, Anlegen, Löschen, Umbenennen), getestet gegen Fakes. Das Crate
  `fuse` übersetzt nur zwischen `fuser` (0.18, MIT, ohne libfuse; zur Laufzeit wird
  `fusermount3` gebraucht) und `VirtualDrive`. Jede FUSE-Anfrage läuft als eigene Tokio-Aufgabe,
  damit ein langsamer Download nicht das ganze Laufwerk blockiert. Das Crate gibt es nur unter
  Linux und nur mit dem Feature `fuse` (Desktop an, Server-Image aus).
- **Nicht unterstützt:** harte und symbolische Links, erweiterte Attribute, Dateirechte
  (`chmod` wird ignoriert, alles gehört dem Benutzer des Daemons).

## Folgen
- Der Server-Build bleibt ohne FUSE und ohne zusätzliche Abhängigkeit.
- Nicht angeheftete Inhalte sind ohne Netz nicht lesbar; das Laufwerk meldet dann einen
  Ein-/Ausgabefehler.
- Echte FUSE-Tests brauchen `/dev/fuse` und `fusermount3` und laufen deshalb als `#[ignore]`;
  die Logik dahinter ist vollständig gegen Fakes getestet.

---

## English

**ADR 0010: Placeholders through a FUSE mount with a block cache**

- Status: accepted
- Date: 2026-10-16

### Context
Phase 3 asks for files "on demand" (section 3.3): a virtual drive shows all files at once, the
content is loaded on open and cached locally. In addition "always keep local" per folder/file and
automatic eviction after X days, which may only happen when the version in Proton is current and
complete. Proton asks for events instead of recursive traversal. The Proton crates provide byte
ranges (`download_range`).

### Decision
- **Placeholders exist only in the FUSE mount.** Linux has no placeholder interface for normal
  folders (unlike the Cloud Files API on Windows). Sync folders stay fully local; "always keep
  local" and eviction concern the cache of a mount.
- **Configuration:** any number of `[[mount]]` sections (id, mount point, folder in Proton,
  `keep_local` as the default for "always keep local", off by default) and a global `[cache]`
  section (evict after days, default 30; size limit, default 10 GB).
- **Block cache:** every file is stored in 4 MiB blocks in a sparse file in the data directory.
  Which blocks are present is stored as a bitmap in SQLite. Reading fetches only missing blocks
  (plus one block of read-ahead), so large files (videos) open at once. If the revision in Proton
  changes, the entry is discarded unless it was changed locally.
- **Metadata on demand:** a folder is read from Proton on its first listing and then kept in
  memory; Proton events keep it current. Inode numbers are valid only while mounted.
- **Writing:** opening for writing first loads the missing blocks; writes go to the cache and
  mark the entry as "changed" (recorded in SQLite, survives restarts and crashes). After the last
  close it is uploaded (new revision or new file). If the revision in Proton changed meanwhile,
  the local version is uploaded as a conflict copy; nothing is ever overwritten.
- **Deleting and renaming:** deleting moves the node into the Proton trash and records it in the
  protonsync trash (owner `mount:<id>`), restorable with `protonsync trash restore`. Renames and
  moves go straight to Proton.
- **Pinning:** rules per path in SQLite; the closest rule wins (file, then parent folders up to
  the root), otherwise `keep_local`. The daemon downloads pinned files completely in the
  background.
- **Eviction:** candidates are only entries that are not pinned, not changed and not open: first
  those unused for X days, then, while the size limit is exceeded, the least recently used.
  Unchanged local content came from Proton or was uploaded completely; before removing, the
  daemon only checks that the file still exists in Proton (a newer revision there is safe too and
  is loaded on demand). With `dry_run = true` it only plans.
- **Technology:** the application layer gets a FUSE-independent `VirtualDrive` (lookup, listing,
  read, write, create, delete, rename), tested against fakes. The `fuse` crate only translates
  between `fuser` (0.18, MIT, without libfuse; `fusermount3` is needed at runtime) and
  `VirtualDrive`. Every FUSE request runs as its own Tokio task so a slow download does not block
  the whole drive. The crate exists only on Linux and only with the `fuse` feature (on for the
  desktop, off for the server image).
- **Not supported:** hard and symbolic links, extended attributes, file permissions (`chmod` is
  ignored, everything belongs to the daemon's user).

### Consequences
- The server build stays without FUSE and without an extra dependency.
- Content that is not pinned cannot be read without a network; the drive then reports an I/O
  error.
- Real FUSE tests need `/dev/fuse` and `fusermount3` and therefore run as `#[ignore]`; the logic
  behind them is fully tested against fakes.

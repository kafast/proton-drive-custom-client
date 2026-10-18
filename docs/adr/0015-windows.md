# ADR 0015: Windows vorerst zurückgestellt, Architektur bleibt offen

- Status: angenommen
- Datum: 2026-10-18

## Kontext
`CLAUDE.md` Abschnitt 9 verlangt, nach Phase 6 die Lauffähigkeit unter Windows neu zu bewerten.
Linux bleibt Zielplattform; Windows ist untergeordnet. Proton bietet für Windows bereits einen
offiziellen Client mit Synchronisation und Dateien „bei Bedarf“. Für diese Bewertung wurde der
Code Crate für Crate auf Unix-Eigenheiten durchsucht (`std::os::unix`, `cfg(unix)`, Unix-Sockets,
Dateirechte, `libc`, `fuser`, inotify, systemd, XDG-Pfade, Keyring-Backends, Qt, Pfade als Text,
Windows-Namensregeln). Ein Testbau für `x86_64-pc-windows-gnu` war nicht möglich, weil das Ziel
in der Bauumgebung nicht installiert ist; die Aussagen beruhen auf dem Quelltext und `Cargo.lock`.

### Befund je Crate
**Ohne Änderung portabel**
- `domain`, `application`, `testkit`: keine Plattformabhängigkeiten. Die Sync-Engine und
  `VirtualDrive` (Block-Zwischenspeicher, `path_status`) sind reines Rust hinter Ports.
- `api`: portabel bis auf den voreingestellten Socket-Pfad (`crates/api/src/wire.rs:13`,
  `/run/protonsync/daemon.sock`, und `XDG_RUNTIME_DIR` in Zeile 55).
- `adapter-state`: SQLite (`bundled`), Dateirechte schon mit `cfg(unix)`/`cfg(not(unix))`
  getrennt (`crates/adapter-state/src/lib.rs:71-80`).
- `adapter-proton`: keine Plattformabhängigkeiten im eigenen Code. Die Proton-Crates bauen auf
  `tokio`/`reqwest`; `aws-lc-sys` und `rusqlite` brauchen unter Windows einen C-Compiler (MSVC).
- `web`: TCP mit `axum`/`tokio-rustls`, portabel.
- `fuse`: das ganze Crate ist `#![cfg(target_os = "linux")]` (`crates/fuse/src/lib.rs:20`) und im
  Daemon nur für Linux eingebunden; es baut unter Windows leer und stört nicht.

**Mit kleinen bis mittleren Anpassungen**
- `adapter-fs`: `notify` nutzt unter Windows `ReadDirectoryChangesW` (in `Cargo.lock` schon
  enthalten). Dateirechte 0600/0700 sind mit `cfg(unix)` geschützt (`src/cache.rs:56-76`), unter
  Windows also wirkungslos. Offen sind die Windows-Namensregeln: `NodeName` verbietet nur `/` und
  das Nullbyte (`crates/domain/src/node.rs:52-55`), `RelativePath::to_path_buf` zerlegt am `/`
  (`crates/domain/src/path.rs:109`), und `FsLocal::absolute` hängt das Ergebnis ungeprüft an
  (`crates/adapter-fs/src/lib.rs:71-72`). Unter Windows ist `\` aber ebenfalls ein Trenner; ein
  Name wie `..\x` aus Proton würde den Sync-Ordner verlassen. Dazu kommen verbotene Zeichen
  (`<>:"|?*`), reservierte Namen (`CON`, `NUL`, …), Punkt oder Leerzeichen am Ende, Groß-/
  Kleinschreibung (zwei Proton-Namen, die sich nur darin unterscheiden, kollidieren) und Dateien,
  die ein anderes Programm geöffnet hält (Umbenennen und `persist` schlagen dann fehl). Das
  gehört in den lokalen Adapter (Abbilden bzw. Melden als Konflikt), nicht in die Domäne.
- `adapter-secrets`: `keyring` 4.2 bringt mit der Standardfunktion `v1` die Windows-
  Anmeldeinformationsverwaltung schon mit; `KeyringStore` braucht nur andere Texte
  (`src/keyring_store.rs`). Die verschlüsselte Datei (`src/file_store.rs`, `src/master_key.rs`)
  setzt Rechte nur unter Unix; unter Windows schützt nur die ACL des Benutzerprofils.
  `CREDENTIALS_DIRECTORY` (systemd) fehlt dort einfach. Für einen Windows-Server wäre DPAPI die
  passende Ablage des Schlüssels.
- `daemon`: Die Pfade folgen XDG und `HOME` (`src/config.rs:211-243`), unter Windows gibt es
  meist kein `HOME`, der Rückfall ist dann `/` (`src/config.rs:240`); richtig wären `%APPDATA%`
  bzw. `%LOCALAPPDATA%`. `".local/share"` ist als Text mit `/` zusammengesetzt
  (`src/config.rs:231`). Signale sind schon getrennt (`src/app.rs:292-305`, SIGTERM nur unter
  Unix, sonst Strg+C). Ohne FUSE wird `[[mount]]` nur verbunden (`src/app.rs:190-194`), der Port
  `DriveMounter` (`src/core/mounts.rs:36`) ist die Stelle für einen Windows-Adapter.
- `tray-model`: Pfade wie im Daemon (`src/settings.rs:154-180`, Rückfall `/` in Zeile 174);
  Autostart schreibt eine `.desktop`-Datei nach `~/.config/autostart` (`src/settings.rs:200-214`).
  Unter Windows wäre das ein Eintrag unter `HKCU\…\Run` oder eine Verknüpfung im
  Autostart-Ordner.
  Die Hinweise `systemctl --user start protonsyncd` (`src/texts.rs:42-43`,
  `crates/cli/src/commands.rs:796-797`) passen dort nicht.
- `gui-qt`: `cxx-qt` und Qt 6 laufen unter Windows (MSVC, `windeployqt`, Qt dynamisch wie unter
  Linux). Eigener Plattformcode ist nicht vorhanden; die Tray-App erbt die Probleme aus `client`
  und `tray-model`.

**Baut unter Windows heute nicht**
- `daemon/src/server.rs:17` nutzt `tokio::net::UnixListener`/`UnixStream` ohne `cfg`, und
  `client/src/transport.rs:4` nutzt `std::os::unix::net::UnixStream`. Beides gibt es unter
  Windows weder in `std` noch in `tokio`, obwohl Windows 10 AF_UNIX kennt (nur über
  Zusatz-Crates wie `uds_windows`, ohne `tokio`-Anbindung). Damit bauen `daemon`, `client`,
  `cli`, `tray-model` und `gui-qt` unter Windows nicht. Auch die Tests
  `daemon/tests/server_and_scheduler.rs` und die Tests in `client` sind Unix-only.
  Gut ist: `client` hat schon den Trait `Transport` (`src/transport.rs:31`), und der Daemon-Server
  ist auf `bind`, `serve` und `connection` beschränkt.

**Eigener Adapter nötig**
- **Platzhalter:** FUSE (`fuser`, `fusermount3`, `libc`) gibt es unter Windows nicht. Der
  Gegenpart ist die **Cloud Files API** (cfapi): Platzhalter liegen als NTFS-Dateien in einem
  registrierten Sync-Root, das System ruft zum Laden zurück. Das ist eher ein Sync-Ordner mit
  Platzhaltern als ein Mount und braucht FFI über das `windows`-Crate oder `cloud-filter`, also
  `unsafe` mit einer Ausnahme wie bei `gui-qt` (ADR 0011). Vorteil: Explorer zeigt Status und
  bietet „Immer auf diesem Gerät behalten“ / „Speicherplatz freigeben“ selbst an; die
  Dateimanager-Integration aus ADR 0013 wäre dort nicht nötig.
- **Daemon-API:** Named Pipe (`tokio::net::windows::named_pipe`, z. B.
  `\\.\pipe\protonsync-<Benutzer>`, Zugriff nur für den Benutzer) statt Unix-Socket. Der
  Unix-Socket ab Windows 10 ist mit `tokio` nicht direkt nutzbar und bringt keinen Vorteil.
- **Verpackung und Betrieb:** MSI (z. B. `cargo-wix`) und winget statt PKGBUILD, Autostart der
  Tray-App per Registry und Daemon als Prozess der Anmeldung (oder Windows-Dienst für einen
  Server) statt systemd; das Server-Image bleibt Linux.

### Grobe Aufwandsschätzung
| Bereich | Aufwand |
|---|---|
| Pfade nach `%APPDATA%`/`%LOCALAPPDATA%`, kein Rückfall auf `/`, Texte ohne `systemctl` | klein |
| Pfadschutz gegen `\` und Nicht-Normal-Komponenten im lokalen Adapter | klein |
| Windows-Prüfbau der portablen Crates in der CI | klein |
| Named Pipe in Daemon und Client, Tests darauf umstellen | mittel |
| Windows-Namensregeln, Groß-/Kleinschreibung, gesperrte Dateien in `adapter-fs` | mittel |
| Geheimnisse (Anmeldeinformationsverwaltung testen, DPAPI für die Datei) | klein bis mittel |
| Tray-App unter Windows bauen, Autostart über Registry | mittel |
| MSI/winget, Daemon-Start, CI-Läufer mit Windows | mittel |
| Platzhalter über die Cloud Files API | groß |

## Entscheidung
- **Windows bleibt zurückgestellt.** Linux ist die Zielplattform, Proton hat für Windows einen
  offiziellen Client mit Sync und Dateien bei Bedarf, und der größte Teil des Nutzens (Platzhalter
  über cfapi) wäre zugleich der größte Aufwand. Ein Windows-Port beginnt erst, wenn es dafür
  einen konkreten Bedarf gibt, den der offizielle Client nicht deckt (z. B. Upload-Jobs oder der
  Fotospiegel); dann zuerst ohne Platzhalter (Sync-Ordner, CLI, Tray-App).
- **Die Architektur bleibt offen.** Plattformabhängiges bleibt in Adaptern hinter Ports
  (`DriveMounter`, `Transport`, `SecretStore`, `LocalStorage`); Domäne, Use Cases und
  Daemon-Ablauf bleiben frei davon.
- **Nur billige Portabilitätsfixes jetzt**, wenn ohnehin an der Stelle gearbeitet wird oder sie
  ein Risiko senken:
  1. `FsLocal::absolute` (`crates/adapter-fs/src/lib.rs:71`) prüft, dass jede Komponente eine
     normale ist (kein `..`, kein Wurzel- oder Laufwerksteil). Unter Linux ändert das nichts,
     unter Windows verhindert es das Verlassen des Sync-Ordners.
  2. `".local/share"` als zwei `join` (`crates/daemon/src/config.rs:231`).
  3. Optional: CI-Schritt `cargo check --target x86_64-pc-windows-gnu` für die reinen
     Rust-Crates (`domain`, `application`, `api`, `testkit`, `adapter-fs`, `adapter-secrets`),
     damit sie portabel bleiben. Crates mit C-Anteil (`rusqlite`, `aws-lc-sys`) bräuchten dafür
     einen MinGW-Compiler und bleiben außen vor.
- Named Pipe, Windows-Pfade, Namensregeln, Autostart, Verpackung und cfapi werden erst mit einem
  Windows-Port umgesetzt.

## Folgen
- Der Linux-Build ändert sich nicht; die vorgeschlagenen Fixes sind klein und unter Linux
  verhaltensneutral.
- `daemon`, `client`, `cli`, `tray-model` und `gui-qt` bauen unter Windows weiterhin nicht, bis die
  Daemon-API eine Named Pipe bekommt. Das ist bekannt und gewollt.
- Neuer Code soll keine neuen Unix-Annahmen außerhalb der Adapter einführen; Pfade weiter mit
  `Path`/`PathBuf`, Unix-Spezielles mit `cfg(unix)` und einem Gegenstück.
- Erscheint der angekündigte offizielle Linux-Client oder ändert sich der Windows-Client von
  Proton, wird diese Entscheidung neu bewertet.

---

## English

# ADR 0015: Windows deferred for now, architecture stays open

- Status: accepted
- Date: 2026-10-18

## Context
`CLAUDE.md` section 9 asks to re-evaluate Windows support after phase 6. Linux stays the target
platform; Windows is secondary. Proton already ships an official Windows client with sync and
on-demand files. For this assessment the code was searched crate by crate for Unix specifics
(`std::os::unix`, `cfg(unix)`, Unix sockets, file permissions, `libc`, `fuser`, inotify,
systemd, XDG paths, keyring backends, Qt, paths built as text, Windows name rules). A test build
for `x86_64-pc-windows-gnu` was not possible because the target is not installed in the build
environment; the findings rest on the source code and `Cargo.lock`.

### Findings per crate
**Portable without changes**
- `domain`, `application`, `testkit`: no platform dependencies. The sync engine and
  `VirtualDrive` (block cache, `path_status`) are pure Rust behind ports.
- `api`: portable except for the default socket path (`crates/api/src/wire.rs:13`,
  `/run/protonsync/daemon.sock`, and `XDG_RUNTIME_DIR` on line 55).
- `adapter-state`: SQLite (`bundled`), file permissions already split with
  `cfg(unix)`/`cfg(not(unix))` (`crates/adapter-state/src/lib.rs:71-80`).
- `adapter-proton`: no platform dependencies in its own code. The Proton crates build on
  `tokio`/`reqwest`; `aws-lc-sys` and `rusqlite` need a C compiler (MSVC) on Windows.
- `web`: TCP with `axum`/`tokio-rustls`, portable.
- `fuse`: the whole crate is `#![cfg(target_os = "linux")]` (`crates/fuse/src/lib.rs:20`) and is
  only pulled into the daemon on Linux; on Windows it builds empty and does no harm.

**Small to medium changes**
- `adapter-fs`: `notify` uses `ReadDirectoryChangesW` on Windows (already in `Cargo.lock`).
  Permissions 0600/0700 are guarded by `cfg(unix)` (`src/cache.rs:56-76`), so they do nothing
  on Windows. The Windows name rules are open: `NodeName` only forbids `/` and the null byte
  (`crates/domain/src/node.rs:52-55`), `RelativePath::to_path_buf` splits at `/`
  (`crates/domain/src/path.rs:109`), and `FsLocal::absolute` appends the result unchecked
  (`crates/adapter-fs/src/lib.rs:71-72`). On Windows `\` is a separator as well; a name like
  `..\x` coming from Proton would leave the sync folder. On top of that come forbidden
  characters (`<>:"|?*`), reserved names (`CON`, `NUL`, …), a trailing dot or space, case
  insensitivity (two Proton names that differ only in case collide) and files held open by
  another program (rename and `persist` then fail). This belongs in the local adapter (map the
  name or report a conflict), not in the domain.
- `adapter-secrets`: `keyring` 4.2 already includes the Windows Credential Manager through its
  default feature `v1`; `KeyringStore` only needs different texts (`src/keyring_store.rs`). The
  encrypted file (`src/file_store.rs`, `src/master_key.rs`) sets permissions only on Unix; on
  Windows only the ACL of the user profile protects it. `CREDENTIALS_DIRECTORY` (systemd) is
  simply absent there. For a Windows server, DPAPI would be the right place for the key.
- `daemon`: paths follow XDG and `HOME` (`src/config.rs:211-243`); Windows usually has no
  `HOME`, so the fallback is `/` (`src/config.rs:240`); `%APPDATA%` or `%LOCALAPPDATA%` would be
  correct. `".local/share"` is built as text with `/` (`src/config.rs:231`). Signals are already
  split (`src/app.rs:292-305`, SIGTERM only on Unix, otherwise Ctrl+C). Without FUSE,
  `[[mount]]` is only connected (`src/app.rs:190-194`); the port `DriveMounter`
  (`src/core/mounts.rs:36`) is where a Windows adapter would plug in.
- `tray-model`: paths as in the daemon (`src/settings.rs:154-180`, fallback `/` on line 174);
  autostart writes a `.desktop` file to `~/.config/autostart` (`src/settings.rs:200-214`). On
  Windows this would be an entry under `HKCU\…\Run` or a shortcut in the Startup folder. The
  hints `systemctl --user start protonsyncd` (`src/texts.rs:42-43`,
  `crates/cli/src/commands.rs:796-797`) do not fit there.
- `gui-qt`: `cxx-qt` and Qt 6 run on Windows (MSVC, `windeployqt`, Qt linked dynamically as on
  Linux). There is no platform code of its own; the tray app inherits the issues of `client` and
  `tray-model`.

**Does not build on Windows today**
- `daemon/src/server.rs:17` uses `tokio::net::UnixListener`/`UnixStream` without `cfg`, and
  `client/src/transport.rs:4` uses `std::os::unix::net::UnixStream`. Neither `std` nor `tokio`
  offers these on Windows, even though Windows 10 knows AF_UNIX (only through extra crates such
  as `uds_windows`, without `tokio` support). So `daemon`, `client`, `cli`, `tray-model` and
  `gui-qt` do not build on Windows. The tests in `daemon/tests/server_and_scheduler.rs` and in
  `client` are Unix-only too. The good part: `client` already has the `Transport` trait
  (`src/transport.rs:31`), and the daemon server is limited to `bind`, `serve` and `connection`.

**Needs its own adapter**
- **Placeholders:** FUSE (`fuser`, `fusermount3`, `libc`) does not exist on Windows. The
  counterpart is the **Cloud Files API** (cfapi): placeholders are NTFS files in a registered
  sync root, and the system calls back to load them. That is more a sync folder with
  placeholders than a mount, and it needs FFI through the `windows` crate or `cloud-filter`,
  i.e. `unsafe` with an exception like the one for `gui-qt` (ADR 0011). Benefit: Explorer shows
  the status and offers "Always keep on this device" / "Free up space" by itself; the file
  manager integration from ADR 0013 would not be needed there.
- **Daemon API:** a named pipe (`tokio::net::windows::named_pipe`, e.g.
  `\\.\pipe\protonsync-<user>`, access for the user only) instead of a Unix socket. The Unix
  socket from Windows 10 onwards cannot be used with `tokio` directly and brings no benefit.
- **Packaging and operation:** MSI (e.g. `cargo-wix`) and winget instead of PKGBUILD, autostart
  of the tray app via the registry and the daemon as a login process (or a Windows service for a
  server) instead of systemd; the server image stays Linux.

### Rough effort estimate
| Area | Effort |
|---|---|
| Paths to `%APPDATA%`/`%LOCALAPPDATA%`, no fallback to `/`, texts without `systemctl` | small |
| Path guard against `\` and non-normal components in the local adapter | small |
| Windows check build of the portable crates in CI | small |
| Named pipe in daemon and client, switch the tests over | medium |
| Windows name rules, case insensitivity, locked files in `adapter-fs` | medium |
| Secrets (test the Credential Manager, DPAPI for the file) | small to medium |
| Build the tray app on Windows, autostart via the registry | medium |
| MSI/winget, daemon start, CI runner with Windows | medium |
| Placeholders through the Cloud Files API | large |

## Decision
- **Windows stays deferred.** Linux is the target platform, Proton has an official Windows
  client with sync and on-demand files, and the largest part of the benefit (placeholders via
  cfapi) would also be the largest effort. A Windows port only starts when there is a concrete
  need the official client does not cover (for example upload jobs or the photo mirror); it
  would then start without placeholders (sync folders, CLI, tray app).
- **The architecture stays open.** Platform-specific code stays in adapters behind ports
  (`DriveMounter`, `Transport`, `SecretStore`, `LocalStorage`); domain, use cases and the daemon
  flow stay free of it.
- **Only cheap portability fixes now**, when the code is touched anyway or they lower a risk:
  1. `FsLocal::absolute` (`crates/adapter-fs/src/lib.rs:71`) checks that every component is a
     normal one (no `..`, no root or drive part). On Linux this changes nothing; on Windows it
     prevents leaving the sync folder.
  2. `".local/share"` as two `join` calls (`crates/daemon/src/config.rs:231`).
  3. Optional: a CI step `cargo check --target x86_64-pc-windows-gnu` for the pure Rust crates
     (`domain`, `application`, `api`, `testkit`, `adapter-fs`, `adapter-secrets`) so they stay
     portable. Crates with C parts (`rusqlite`, `aws-lc-sys`) would need a MinGW compiler and
     stay out.
- Named pipe, Windows paths, name rules, autostart, packaging and cfapi are only done as part of
  a Windows port.

## Consequences
- The Linux build does not change; the proposed fixes are small and behave the same on Linux.
- `daemon`, `client`, `cli`, `tray-model` and `gui-qt` still do not build on Windows until the
  daemon API gets a named pipe. This is known and intended.
- New code should not add new Unix assumptions outside the adapters; keep building paths with
  `Path`/`PathBuf`, and put Unix-specific code under `cfg(unix)` with a counterpart.
- If the announced official Linux client appears or Proton's Windows client changes, this
  decision is re-evaluated.

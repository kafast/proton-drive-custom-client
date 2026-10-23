# ADR 0018: Windows neu bewertet: Port ohne Platzhalter in Stufen

- Status: vorgeschlagen (wartet auf die Entscheidung des Auftraggebers)
- Datum: 2026-10-23
- Ergänzt: [ADR 0015](0015-windows.md)

## Kontext
ADR 0015 (2026-10-18) hat Windows zurückgestellt, bis es einen konkreten Bedarf gibt, den der
offizielle Proton-Client nicht deckt. Am 2026-10-23 hat der Auftraggeber um eine Neubewertung
gebeten. Seitdem hat sich geändert:

- **Neue Funktionen, die der offizielle Windows-Client nicht hat:** Löschschutz
  `protect_remote` (ADR 0016), Benachrichtigungen per E-Mail/Webhook und Wochenbericht
  (ADR 0017). Dazu kamen schon vorher Upload-Jobs mit Aufbewahrung (ADR 0008), der Fotospiegel in
  einen lokalen Ordner (ADR 0012), Konfliktwarteschlange statt stillem Überschreiben, Probelauf
  und Konfiguration als Textdatei. Damit gibt es den Bedarf, den ADR 0015 als Bedingung nennt.
- **Ein Windows-Rechner ist vorhanden** (Windows 11 mit MSVC, Build-Probe vom 2026-10-19). Der
  Port kann dort echt erprobt werden.
- **Der Befund aus ADR 0015 gilt weiter.** Neu ist nur `adapter-notify` (`lettre`, `reqwest`), das
  keinen Plattformcode hat. Was heute unter Windows nicht baut, hängt an einer einzigen Stelle:
  dem Unix-Socket in `daemon/src/server.rs` und `client/src/transport.rs`.

### Optionen
| Option | Nutzen | Aufwand |
|---|---|---|
| A: nur portabel halten | Windows bleibt möglich, kein lauffähiger Client | klein (erledigt, siehe unten) |
| B: Port ohne Platzhalter | Daemon, CLI, Sync-Ordner, Upload-Jobs, Fotospiegel, Löschschutz, Benachrichtigungen, Web, später Tray-App | mittel, in Stufen |
| C: wie B, dazu Platzhalter | zusätzlich Dateien bei Bedarf im Explorer | groß |

Für Platzhalter unter Windows gibt es zwei Wege:
- **Cloud Files API (cfapi):** Teil von Windows, kein Treiber nötig, Explorer zeigt Status und
  bietet „Immer auf diesem Gerät behalten“ / „Speicherplatz freigeben“ selbst an. Braucht FFI
  (`windows`-Crate oder `cloud-filter`), also `unsafe` mit Ausnahme wie bei `gui-qt`. Das Modell
  ist ein Sync-Ordner mit Platzhaltern, nicht ein Laufwerk; `VirtualDrive` passt nur zum Teil
  (Laden per Byte-Bereich und Zwischenspeicher ja, Verzeichnisbaum und Schreiben laufen über
  NTFS).
- **WinFsp:** wäre am nächsten an FUSE, braucht aber einen eigenen Treiber, und die Lizenz (GPLv3
  mit Ausnahme nur für FLOSS-Lizenzen) passt nicht zu PolyForm Noncommercial. Scheidet aus.

Platzhalter sind genau das, was der offizielle Windows-Client schon gut kann. Der Mehrwert von
protonsync unter Windows liegt bei den Funktionen aus Option B.

## Entscheidung (Empfehlung)
**Option B: Port ohne Platzhalter, in Stufen.** Jede Stufe ist ein eigener PR, Linux bleibt
unverändert und hat Vorrang. Option C wird erst geprüft, wenn B läuft und der Bedarf dann noch
besteht.

0. **Portabilität sichern (mit diesem ADR erledigt):** Pfadschutz in `FsLocal::absolute` (jeder
   Pfadteil muss genau eine normale Komponente sein, sonst Fehler statt Verlassen des
   Sync-Ordners), Standardpfade ohne fest eingebautes `/`, CI-Job `windows` auf `windows-latest`
   (MSVC) mit clippy für alle schon portablen Crates und Tests für Domäne, Anwendung und
   `adapter-fs`.
1. **Daemon-API über Named Pipe:** `tokio::net::windows::named_pipe` im Daemon, ein
   `Transport` für Windows im `client`. Name `\\.\pipe\protonsync-<Benutzer>`, Zugriff per
   Sicherheitsbeschreibung nur für den angemeldeten Benutzer, entfernte Clients abgewiesen.
   Unter Linux bleibt der Unix-Socket. Danach bauen `daemon`, `client`, `cli` und `tray-model`;
   die CI prüft und testet sie unter Windows mit.
2. **Pfade und Texte:** Konfiguration unter `%APPDATA%\protonsync`, Daten unter
   `%LOCALAPPDATA%\protonsync`, kein Rückfall auf `/`; Hinweise ohne `systemctl` unter Windows;
   `[[mount]]` meldet unter Windows klar „nicht unterstützt“.
3. **Windows-Namensregeln in `adapter-fs`:** verbotene Zeichen (`<>:"/\|?*`), reservierte Namen
   (`CON`, `NUL`, `COM1`, …), Punkt oder Leerzeichen am Ende, Namen, die sich nur in Groß-/
   Kleinschreibung unterscheiden, und von anderen Programmen gesperrte Dateien. Solche Einträge
   werden nicht heimlich umbenannt, sondern übersprungen und gemeldet (wie ein Konflikt, mit
   Erklärung), damit in Proton nichts verloren geht.
4. **Geheimnisse:** Windows-Anmeldeinformationsverwaltung über `keyring` testen; für die
   verschlüsselte Datei den Schlüssel optional mit DPAPI schützen.
5. **Start und Tray-App:** Daemon startet bei der Anmeldung (Aufgabenplanung oder durch die
   Tray-App), Autostart über `HKCU\Software\Microsoft\Windows\CurrentVersion\Run`; Tray-App mit
   Qt 6 (MSVC, `windeployqt`, Qt dynamisch wie unter Linux).
6. **Verpackung:** zuerst ein ZIP im Release-Workflow (Job auf `windows-latest`), später MSI
   (`cargo-wix`) und winget. Das Server-Image bleibt Linux.

Grobe Schätzung: Stufe 1 und 2 je eine Sitzung, Stufe 3 ein bis zwei, Stufe 4 eine, Stufe 5 und 6
zusammen zwei bis drei. Die Proton-Anbindung, die Sync-Engine und alle Use Cases bleiben
unverändert.

## Folgen
- Linux-Verhalten ändert sich durch keine Stufe; Plattformcode bleibt in Adaptern hinter Ports
  (`Transport`, Server-`bind`, `LocalStorage`, `SecretStore`, `DriveMounter`).
- Die CI läuft zusätzlich auf Windows. Ein roter Windows-Job blockiert wie jeder andere.
- Bis Stufe 1 fertig ist, gibt es keinen lauffähigen Windows-Client.
- Ohne Platzhalter liegen unter Windows alle Dateien vollständig lokal. Wer Dateien bei Bedarf
  braucht, nutzt dafür den offiziellen Client.
- Fällt die Entscheidung auf Option A, bleibt es bei Stufe 0 und ADR 0015 gilt unverändert.

---

## English

# ADR 0018: Windows re-evaluated: port without placeholders in stages

- Status: proposed (awaiting the client's decision)
- Date: 2026-10-23
- Supplements: [ADR 0015](0015-windows.md)

## Context
ADR 0015 (2026-10-18) deferred Windows until there is a concrete need the official Proton client
does not cover. On 2026-10-23 the client asked for a re-evaluation. Since then:

- **New features the official Windows client lacks:** delete protection `protect_remote`
  (ADR 0016), notifications by email/webhook and a weekly report (ADR 0017). Earlier ones are
  upload jobs with retention (ADR 0008), the photo mirror into a local folder (ADR 0012), a
  conflict queue instead of silent overwriting, dry run, and configuration as a text file. So the
  need that ADR 0015 names as its condition now exists.
- **A Windows machine is available** (Windows 11 with MSVC, build probe of 2026-10-19). The port
  can be tried there for real.
- **The findings of ADR 0015 still hold.** The only new crate is `adapter-notify` (`lettre`,
  `reqwest`), which has no platform code. What does not build on Windows today hinges on a single
  spot: the Unix socket in `daemon/src/server.rs` and `client/src/transport.rs`.

### Options
| Option | Benefit | Effort |
|---|---|---|
| A: only stay portable | Windows stays possible, no working client | small (done, see below) |
| B: port without placeholders | daemon, CLI, sync folders, upload jobs, photo mirror, delete protection, notifications, web, later the tray app | medium, in stages |
| C: like B, plus placeholders | on-demand files in Explorer on top | large |

There are two ways to get placeholders on Windows:
- **Cloud Files API (cfapi):** part of Windows, no driver needed, Explorer shows the status and
  offers "Always keep on this device" / "Free up space" by itself. Needs FFI (`windows` crate or
  `cloud-filter`), i.e. `unsafe` with an exception as for `gui-qt`. The model is a sync folder
  with placeholders, not a drive; `VirtualDrive` fits only in part (range loads and the cache do,
  the directory tree and writes go through NTFS).
- **WinFsp:** closest to FUSE, but it needs its own driver, and its licence (GPLv3 with an
  exception only for FLOSS licences) does not fit PolyForm Noncommercial. Ruled out.

Placeholders are exactly what the official Windows client already does well. The added value of
protonsync on Windows lies in the features of option B.

## Decision (recommendation)
**Option B: port without placeholders, in stages.** Each stage is its own PR; Linux stays
unchanged and takes priority. Option C is only examined once B works and the need still exists.

0. **Secure portability (done with this ADR):** path guard in `FsLocal::absolute` (every path
   segment must be exactly one normal component, otherwise an error instead of leaving the sync
   folder), default paths without a hard-coded `/`, CI job `windows` on `windows-latest` (MSVC)
   with clippy for all already portable crates and tests for domain, application and
   `adapter-fs`.
1. **Daemon API over a named pipe:** `tokio::net::windows::named_pipe` in the daemon, a Windows
   `Transport` in `client`. Name `\\.\pipe\protonsync-<user>`, access restricted by a security
   descriptor to the logged-in user, remote clients rejected. Linux keeps the Unix socket.
   Afterwards `daemon`, `client`, `cli` and `tray-model` build; CI checks and tests them on
   Windows too.
2. **Paths and texts:** configuration under `%APPDATA%\protonsync`, data under
   `%LOCALAPPDATA%\protonsync`, no fallback to `/`; hints without `systemctl` on Windows;
   `[[mount]]` clearly reports "not supported" on Windows.
3. **Windows name rules in `adapter-fs`:** forbidden characters (`<>:"/\|?*`), reserved names
   (`CON`, `NUL`, `COM1`, …), a trailing dot or space, names that differ only in case, and files
   locked by other programs. Such entries are not silently renamed but skipped and reported
   (like a conflict, with an explanation), so nothing is lost in Proton.
4. **Secrets:** test the Windows Credential Manager through `keyring`; optionally protect the key
   of the encrypted file with DPAPI.
5. **Start and tray app:** the daemon starts at login (Task Scheduler or via the tray app),
   autostart via `HKCU\Software\Microsoft\Windows\CurrentVersion\Run`; tray app with Qt 6 (MSVC,
   `windeployqt`, Qt linked dynamically as on Linux).
6. **Packaging:** first a ZIP in the release workflow (job on `windows-latest`), later MSI
   (`cargo-wix`) and winget. The server image stays Linux.

Rough estimate: stages 1 and 2 one session each, stage 3 one to two, stage 4 one, stages 5 and 6
together two to three. The Proton integration, the sync engine and all use cases stay unchanged.

## Consequences
- No stage changes Linux behaviour; platform code stays in adapters behind ports (`Transport`,
  server `bind`, `LocalStorage`, `SecretStore`, `DriveMounter`).
- CI additionally runs on Windows. A red Windows job blocks like any other.
- Until stage 1 is done there is no working Windows client.
- Without placeholders all files are fully local on Windows. Anyone who needs on-demand files
  uses the official client for that.
- If the decision is option A, it stays at stage 0 and ADR 0015 applies unchanged.

# protonsync

Inoffizieller Proton-Drive-Client für Linux: hält einen lokalen Ordner mit Proton Drive synchron
oder zeigt Proton Drive als Laufwerk, dessen Dateien erst bei Bedarf geladen werden, als
Desktop-Dienst oder headless auf einem Server (Container/VM).

> **Hinweis:** protonsync ist **nicht** von Proton entwickelt oder unterstützt. Es nutzt
> standardmäßig Protons offizielles Drive-SDK (TypeScript, Vorschau) über einen Hilfsprozess,
> wahlweise die inoffiziellen Community-Crates `proton-sdk`/`proton-drive-rs` (ADR 0025), und
> meldet sich gegenüber Proton ehrlich als Drittanbieter-App (`external-drive-protonsync@0.1.0-alpha`). Das Projekt ist im
> Alpha-Stadium. Bitte nur mit Daten verwenden, von denen es eine weitere Sicherung gibt.

> **KI-generiert:** Code, Tests und Dokumentation dieses Projekts wurden weitgehend mit KI
> erstellt, mit [Claude](https://claude.ai) (Claude Code) von Anthropic. Der Auftraggeber hat
> Anforderungen vorgegeben und Ergebnisse geprüft, aber nicht jede Zeile von Hand geschrieben.
> Bitte den Code entsprechend kritisch prüfen, bevor er mit wichtigen Daten eingesetzt wird.

Stand: alle Phasen umgesetzt (Phase 6 fertig) – Login mit 2FA, Zwei-Wege-Sync, Konfliktwarteschlange, Papierkorb,
wahlweise Versionsarchiv `_archiv` statt Löschen auf beiden Seiten (Handbuch 4.3),
reines Lese-Backup über ein zweites Konto mit freigegebenen Ordnern (Handbuch 4.4, auf Proxmox:
[Anleitung](docs/proxmox-lesebackup.md)),
Warnung bei fast vollem Speicher (Handbuch 6.10),
Daemon und CLI, Upload-Jobs (z. B. Home-Assistant-Backups nach Zeitplan), optionale
Web-Oberfläche für den Server (Handbuch 7.3), FUSE-Laufwerk mit „Immer lokal halten“ und
automatischer Freigabe (Handbuch 4.7, nur Desktop), Tray-App mit Konfliktdialog und
Benachrichtigungen (Handbuch 6.6, Qt 6). Phase 5: Abgleich von Proton Photos in einen
Fotoordner und Duplikaterkennung, die nur vorschlägt (Handbuch 4.8 und 6.8). Phase 6:
Kontextmenüs für Dolphin und Nautilus, Embleme in Nautilus (Handbuch 6.9). Änderungsprotokoll
aller Abgleiche mit optionaler täglicher E-Mail (Handbuch 6.11). Ausführliche Anleitung: [**Benutzerhandbuch**](docs/handbuch.md) (English: [manual](docs/manual.md)).
Stand und offene Punkte: [`docs/STATUS.md`](docs/STATUS.md), Architektur: [`docs/adr/`](docs/adr/).

## Installieren

Fertige Pakete liegen bei jedem Release unter [**Releases**](https://github.com/kafast/proton-drive-custom-client/releases) (kein Bauen nötig,
kein GitHub-Konto nötig). `VERSION` durch die gewünschte Version ersetzen, z. B. `0.1.0`.

**Arch/CachyOS** (Daemon, CLI, Tray-App, systemd-Dienste, Dateimanager-Menüs):

```sh
VERSION=0.1.0
sudo pacman -U "https://github.com/kafast/proton-drive-custom-client/releases/download/v$VERSION/protonsync-bin-$VERSION-1-x86_64.pkg.tar.zst"
```

`pacman` installiert dabei auch `fuse3` und Qt 6. Weiter mit [Erste Schritte](#erste-schritte-desktop)
(den `cargo install`-Befehl dort überspringen, die Programme liegen schon in `/usr/bin`).

**Andere Linux-Systeme** (nur Daemon und CLI, statisch gelinkt, ohne Tray-App und ohne Secret
Service, also wie im Server-Betrieb):

```sh
curl -LO "https://github.com/kafast/proton-drive-custom-client/releases/download/v$VERSION/protonsync-$VERSION-x86_64-linux-musl.tar.gz"
tar -xzf protonsync-$VERSION-x86_64-linux-musl.tar.gz
sudo install -m755 protonsync-$VERSION-x86_64-linux-musl/protonsync* /usr/local/bin/
```

**Server-Container:** `docker pull ghcr.io/kafast/protonsync:latest`; `compose.yaml` nutzt dieses
Image (siehe [Erste Schritte (Server)](#erste-schritte-server-container)).

Die Datei `SHA256SUMS` im Release enthält die Prüfsummen aller Dateien.

## Bauen

Voraussetzung: Rust ≥ 1.96 (`rustup` installiert die passende Version über `rust-toolchain.toml`).
Für die Tray-App zusätzlich Qt 6 mit QML (Arch/CachyOS: `qt6-base qt6-declarative qt6-svg`).

```sh
cargo build --release      # protonsyncd, protonsync und protonsync-tray in target/release/
cargo test                 # alle Tests (ohne Proton-Konto)
cargo build --release -p protonsync-daemon -p protonsync-cli   # nur Daemon und CLI, ohne Qt
scripts/check.sh           # alle Prüfungen der CI (Format, Clippy, Tests, Doku) lokal
```

Arch/CachyOS: `cd packaging/arch && makepkg -si` baut und installiert das Paket `protonsync-git`
(Handbuch 3.5).

## Erste Schritte (Desktop)

1. Installieren und Konfiguration anlegen:
   ```sh
   cargo install --path crates/daemon && cargo install --path crates/cli   # entfällt mit Paket
   mkdir -p ~/.config/protonsync
   protonsync config example > ~/.config/protonsync/config.toml   # danach anpassen
   ```
2. Daemon als User-Dienst starten:
   ```sh
   cp packaging/systemd/protonsyncd.user.service ~/.config/systemd/user/protonsyncd.service
   systemctl --user daemon-reload && systemctl --user enable --now protonsyncd
   ```
3. Anmelden (fragt Passwort, 2FA-Code und ggf. Mailbox-Passwort ab, ohne Echo):
   ```sh
   protonsync login
   ```
   Gespeichert werden nur Sitzungstokens und Schlüssel-Passphrasen, im Secret Service
   (GNOME Keyring/KWallet), niemals das Passwort.
4. Prüfen, dann scharf schalten:
   ```sh
   protonsync sync            # zeigt nur, was passieren würde (Dry-Run)
   protonsync sync --apply    # führt es einmal aus
   ```
   Solange in der Konfiguration `dry_run = true` steht (Voreinstellung), plant auch der
   automatische Abgleich nur. Für den Dauerbetrieb `dry_run = false` setzen und den Dienst neu starten.
5. Optional die Tray-App: `cargo install --path crates/gui-qt`, dann `protonsync-tray &` und in
   ihren Einstellungen „Beim Anmelden starten“ anhaken.

## Erste Schritte (Server, Container)

Im Verzeichnis `packaging/container/` (dort liegen `compose.yaml` und eine Beispiel-`config.toml`):

```sh
cd packaging/container
$EDITOR config.toml                                    # Sync-Paare und Volume-Pfade anpassen
openssl rand -hex 32 > protonsync-key && chmod 0400 protonsync-key
sudo chown 65532 protonsync-key                        # Benutzer „nonroot“ im Container
docker compose up -d
docker compose exec protonsync protonsync login
```

Das Image ist statisch gelinkt (musl), läuft als nicht-root (UID 65532) und enthält nur die beiden
Binaries. Eingebundene Sync-Ordner müssen für diese UID beschreibbar sein.
Ohne Container: `packaging/systemd/protonsyncd.service` (Schlüssel über systemd-Credentials).

Schlüsselquellen für die verschlüsselte Sitzungsdatei, in dieser Reihenfolge:
`$CREDENTIALS_DIRECTORY/protonsync-key`, `PROTONSYNC_KEY_FILE` (Rechte höchstens 0600),
`PROTONSYNC_KEY` (64 Hex-Zeichen).

## CLI

| Befehl | Zweck |
|---|---|
| `protonsync login` / `logout` | Anmelden (2FA, Mailbox-Passwort) bzw. Sitzung löschen |
| `protonsync status` | Konto, Sync-Paare, letzter Lauf |
| `protonsync sync [--pair ID] [--apply]` | Abgleich planen oder ausführen |
| `protonsync conflicts [resolve PAAR ID keep-local\|keep-remote\|keep-both]` | Konflikte ansehen/entscheiden |
| `protonsync trash [restore PAAR ID \| purge --apply]` | Papierkorb ansehen, wiederherstellen, abgelaufene Einträge leeren |
| `protonsync jobs [run JOB [--apply] \| history JOB]` | Upload-Jobs ansehen, ausführen, Verlauf |
| `protonsync mounts` | FUSE-Laufwerke, Zwischenspeicher, angeheftete Pfade |
| `protonsync pin PFAD…` / `unpin PFAD…` | „Immer lokal halten“ ein- bzw. ausschalten |
| `protonsync free [PFAD] [--apply]` | Lokalen Speicher freigeben (ohne `--apply` nur planen) |
| `protonsync file-status PFAD…` | Zustand von Dateien (Platzhalter, synchron, Konflikt …) |
| `protonsync photos [sync [--apply] \| duplicates [trash ID… [--apply]]]` | Fotos: Zustand, Abgleich, Duplikate (nur Vorschläge) |
| `protonsync config example` | Beispielkonfiguration ausgeben |
| `protonsync config show` / `config set …` | Wichtige Einstellungen anzeigen bzw. ohne Neustart ändern (auch in Tray-App und Web) |

`--json` liefert maschinenlesbare Ausgabe. Die Sprache folgt `LC_ALL`, `LC_MESSAGES` bzw. `LANG` (Deutsch/Englisch).

Exit-Codes: `0` OK · `1` Fehler · `2` falscher Aufruf · `3` Daemon nicht erreichbar ·
`4` nicht angemeldet · `5` Anmeldung abgelehnt · `6` API-Version passt nicht ·
`7` teilweise fehlgeschlagen.

## Lizenz

[PolyForm Noncommercial 1.0.0](LICENSE.md): private Nutzung erlaubt, kommerzielle nicht.
Abhängigkeiten stehen unter eigenen (MIT/Apache u. a.) Lizenzen, geprüft mit `cargo deny`.

---

## English

Unofficial Proton Drive client for Linux: keeps a local folder in sync with Proton Drive or
shows Proton Drive as a drive whose files are downloaded on demand, either as a desktop service
or headless on a server (container/VM).

> **Note:** protonsync is **not** developed or supported by Proton. By default it uses Proton's
> official Drive SDK (TypeScript, preview) through a helper process, alternatively the unofficial
> community crates `proton-sdk`/`proton-drive-rs` (ADR 0025), and honestly identifies itself to
> Proton as a third-party app (`external-drive-protonsync@0.1.0-alpha`). The project is in alpha stage.
> Please only use it with data for which another backup exists.

> **AI-generated:** the code, tests and documentation of this project were largely created with
> AI, using [Claude](https://claude.ai) (Claude Code) by Anthropic. The maintainer set the
> requirements and reviewed the results, but did not write every line by hand. Please review the
> code critically before using it with important data.

Status: all phases implemented (phase 6 done): login with 2FA, two-way sync, conflict queue, trash,
optionally a version archive `_archiv` instead of deleting on both sides (manual 4.3),
a read-only backup through a second account with shared folders (manual 4.4, on Proxmox:
[guide](docs/proxmox-read-backup.md)), a warning when a
disk runs full (manual 6.10), daemon and CLI,
upload jobs (e.g. Home Assistant backups on a schedule), optional web interface for the
server (manual 7.3), FUSE drive with "always keep local" and automatic freeing of space
(manual 4.7, desktop only), tray app with conflict dialog and notifications (manual 6.6, Qt 6),
syncing Proton Photos into a photo folder and a duplicate finder that only suggests (manual 4.8
and 6.8), context menus for Dolphin and Nautilus and emblems in Nautilus (manual 6.9), and a
change log of all syncs with an optional daily email (manual 6.11).
Detailed guide: [**User manual**](docs/manual.md) (Deutsch: [Benutzerhandbuch](docs/handbuch.md)).
Status and open items: [`docs/STATUS.md`](docs/STATUS.md), architecture: [`docs/adr/`](docs/adr/).

### Install

Ready-made packages are attached to every release under [**Releases**](https://github.com/kafast/proton-drive-custom-client/releases) (no building and
no GitHub account needed). Replace `VERSION` with the wanted version, e.g. `0.1.0`.

**Arch/CachyOS** (daemon, CLI, tray app, systemd services, file manager menus):

```sh
VERSION=0.1.0
sudo pacman -U "https://github.com/kafast/proton-drive-custom-client/releases/download/v$VERSION/protonsync-bin-$VERSION-1-x86_64.pkg.tar.zst"
```

`pacman` also installs `fuse3` and Qt 6. Continue with [Getting started](#getting-started-desktop)
(skip the `cargo install` command there, the programs are already in `/usr/bin`).

**Other Linux systems** (daemon and CLI only, statically linked, without tray app and without
Secret Service, i.e. as in server mode):

```sh
curl -LO "https://github.com/kafast/proton-drive-custom-client/releases/download/v$VERSION/protonsync-$VERSION-x86_64-linux-musl.tar.gz"
tar -xzf protonsync-$VERSION-x86_64-linux-musl.tar.gz
sudo install -m755 protonsync-$VERSION-x86_64-linux-musl/protonsync* /usr/local/bin/
```

**Server container:** `docker pull ghcr.io/kafast/protonsync:latest`; `compose.yaml` uses this
image (see [Getting started (server)](#getting-started-server-container)).

The file `SHA256SUMS` in the release contains the checksums of all files.

### Building

Requirement: Rust ≥ 1.96 (`rustup` installs the matching version via `rust-toolchain.toml`).
The tray app also needs Qt 6 with QML (Arch/CachyOS: `qt6-base qt6-declarative qt6-svg`).

```sh
cargo build --release      # protonsyncd, protonsync and protonsync-tray in target/release/
cargo test                 # all tests (no Proton account needed)
cargo build --release -p protonsync-daemon -p protonsync-cli   # daemon and CLI only, without Qt
scripts/check.sh           # all CI checks (format, clippy, tests, docs) locally
```

Arch/CachyOS: `cd packaging/arch && makepkg -si` builds and installs the `protonsync-git` package
(manual 3.5).

### Getting started (desktop)

1. Install and create the configuration:
   ```sh
   cargo install --path crates/daemon && cargo install --path crates/cli   # not needed with a package
   mkdir -p ~/.config/protonsync
   protonsync config example > ~/.config/protonsync/config.toml   # adjust afterwards
   ```
2. Start the daemon as a user service:
   ```sh
   cp packaging/systemd/protonsyncd.user.service ~/.config/systemd/user/protonsyncd.service
   systemctl --user daemon-reload && systemctl --user enable --now protonsyncd
   ```
3. Log in (asks for the password, the 2FA code and, if needed, the mailbox password, without echo):
   ```sh
   protonsync login
   ```
   Only session tokens and key passphrases are stored, in the Secret Service
   (GNOME Keyring/KWallet), never the password.
4. Check first, then go live:
   ```sh
   protonsync sync            # only shows what would happen (dry run)
   protonsync sync --apply    # runs it once
   ```
   As long as the configuration says `dry_run = true` (the default), the automatic sync also
   only plans. For continuous operation, set `dry_run = false` and restart the service.
5. Optionally the tray app: `cargo install --path crates/gui-qt`, then `protonsync-tray &`, and
   tick "Start at login" in its settings.

### Getting started (server, container)

In the directory `packaging/container/` (it contains `compose.yaml` and an example `config.toml`):

```sh
cd packaging/container
$EDITOR config.toml                                    # adjust sync pairs and volume paths
openssl rand -hex 32 > protonsync-key && chmod 0400 protonsync-key
sudo chown 65532 protonsync-key                        # user "nonroot" in the container
docker compose up -d
docker compose exec protonsync protonsync login
```

The image is statically linked (musl), runs as non-root (UID 65532) and contains only the two
binaries. Mounted sync folders must be writable for this UID.
Without a container: `packaging/systemd/protonsyncd.service` (key via systemd credentials).

Key sources for the encrypted session file, in this order:
`$CREDENTIALS_DIRECTORY/protonsync-key`, `PROTONSYNC_KEY_FILE` (permissions at most 0600),
`PROTONSYNC_KEY` (64 hex characters).

### CLI

| Command | Purpose |
|---|---|
| `protonsync login` / `logout` | Log in (2FA, mailbox password) or delete the session |
| `protonsync status` | Account, sync pairs, last run |
| `protonsync sync [--pair ID] [--apply]` | Plan or run a sync |
| `protonsync conflicts [resolve PAAR ID keep-local\|keep-remote\|keep-both]` | View/resolve conflicts (`PAAR` = sync pair) |
| `protonsync trash [restore PAAR ID \| purge --apply]` | View the trash, restore, empty expired entries |
| `protonsync jobs [run JOB [--apply] \| history JOB]` | View upload jobs, run them, show history |
| `protonsync mounts` | FUSE drives, cache, pinned paths |
| `protonsync pin PATH…` / `unpin PATH…` | Turn "always keep local" on or off |
| `protonsync free [PATH] [--apply]` | Free local space (only plans without `--apply`) |
| `protonsync file-status PATH…` | State of files (placeholder, synced, conflict …) |
| `protonsync photos [sync [--apply] \| duplicates [trash ID… [--apply]]]` | Photos: state, sync, duplicates (suggestions only) |
| `protonsync config example` | Print an example configuration |
| `protonsync config show` / `config set …` | Show or change important settings without a restart (also in the tray app and web) |

`--json` produces machine-readable output. The language follows `LC_ALL`, `LC_MESSAGES` or `LANG` (German/English).

Exit codes: `0` OK · `1` error · `2` invalid usage · `3` daemon not reachable ·
`4` not logged in · `5` login rejected · `6` API version mismatch ·
`7` partially failed.

### License

[PolyForm Noncommercial 1.0.0](LICENSE.md): private use allowed, commercial use not.
Dependencies are under their own licenses (MIT/Apache and others), checked with `cargo deny`.

Required Notice: Copyright (c) 2026 Krischan (https://github.com/kafast)

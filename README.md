# protonsync

Inoffizieller Proton-Drive-Client für Linux: hält einen lokalen Ordner mit Proton Drive synchron,
als Desktop-Dienst oder headless auf einem Server (Container/VM).

> **Hinweis:** protonsync ist **nicht** von Proton entwickelt oder unterstützt. Es nutzt die
> inoffiziellen Community-Crates `proton-sdk`/`proton-drive-rs` und meldet sich gegenüber Proton
> ehrlich als Drittanbieter-App (`external-drive-protonsync@0.1.0-alpha`). Das Projekt ist im
> Alpha-Stadium. Bitte nur mit Daten verwenden, von denen es eine weitere Sicherung gibt.

Stand: Phase 1 (Kern) – Login mit 2FA, Zwei-Wege-Sync, Konfliktwarteschlange, Papierkorb,
Daemon und CLI. Ausführliche Anleitung: [**Benutzerhandbuch**](docs/handbuch.md) (English: [manual](docs/manual.md)).
Stand und offene Punkte: [`docs/STATUS.md`](docs/STATUS.md), Architektur: [`docs/adr/`](docs/adr/).

## Bauen

Voraussetzung: Rust ≥ 1.96 (`rustup` installiert die passende Version über `rust-toolchain.toml`).

```sh
cargo build --release      # Binaries: target/release/protonsyncd und target/release/protonsync
cargo test                 # alle Tests (ohne Proton-Konto)
```

## Erste Schritte (Desktop)

1. Installieren und Konfiguration anlegen:
   ```sh
   cargo install --path crates/daemon && cargo install --path crates/cli
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
| `protonsync config example` | Beispielkonfiguration ausgeben |

`--json` liefert maschinenlesbare Ausgabe. Die Sprache folgt `LC_ALL`, `LC_MESSAGES` bzw. `LANG` (Deutsch/Englisch).

Exit-Codes: `0` OK · `1` Fehler · `2` falscher Aufruf · `3` Daemon nicht erreichbar ·
`4` nicht angemeldet · `5` Anmeldung abgelehnt · `6` API-Version passt nicht ·
`7` teilweise fehlgeschlagen.

## Lizenz

[PolyForm Noncommercial 1.0.0](LICENSE.md): private Nutzung erlaubt, kommerzielle nicht.
Abhängigkeiten stehen unter eigenen (MIT/Apache u. a.) Lizenzen, geprüft mit `cargo deny`.

---

## English

Unofficial Proton Drive client for Linux: keeps a local folder in sync with Proton Drive,
either as a desktop service or headless on a server (container/VM).

> **Note:** protonsync is **not** developed or supported by Proton. It uses the unofficial
> community crates `proton-sdk`/`proton-drive-rs` and honestly identifies itself to Proton as a
> third-party app (`external-drive-protonsync@0.1.0-alpha`). The project is in alpha stage.
> Please only use it with data for which another backup exists.

Status: Phase 1 (core): login with 2FA, two-way sync, conflict queue, trash, daemon and CLI.
Detailed guide: [**User manual**](docs/manual.md) (Deutsch: [Benutzerhandbuch](docs/handbuch.md)).
Status and open items: [`docs/STATUS.md`](docs/STATUS.md), architecture: [`docs/adr/`](docs/adr/).

### Building

Requirement: Rust ≥ 1.96 (`rustup` installs the matching version via `rust-toolchain.toml`).

```sh
cargo build --release      # binaries: target/release/protonsyncd and target/release/protonsync
cargo test                 # all tests (no Proton account needed)
```

### Getting started (desktop)

1. Install and create the configuration:
   ```sh
   cargo install --path crates/daemon && cargo install --path crates/cli
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
| `protonsync config example` | Print an example configuration |

`--json` produces machine-readable output. The language follows `LC_ALL`, `LC_MESSAGES` or `LANG` (German/English).

Exit codes: `0` OK · `1` error · `2` invalid usage · `3` daemon not reachable ·
`4` not logged in · `5` login rejected · `6` API version mismatch ·
`7` partially failed.

### License

[PolyForm Noncommercial 1.0.0](LICENSE.md): private use allowed, commercial use not.
Dependencies are under their own licenses (MIT/Apache and others), checked with `cargo deny`.

Required Notice: Copyright (c) 2026 Krischan (https://github.com/kafast)

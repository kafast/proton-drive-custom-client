# protonsync

Inoffizieller Proton-Drive-Client für Linux: hält einen lokalen Ordner mit Proton Drive synchron,
als Desktop-Dienst oder headless auf einem Server (Container/VM).

> **Hinweis:** protonsync ist **nicht** von Proton entwickelt oder unterstützt. Es nutzt die
> inoffiziellen Community-Crates `proton-sdk`/`proton-drive-rs` und meldet sich gegenüber Proton
> ehrlich als Drittanbieter-App (`external-drive-protonsync@0.1.0-alpha`). Das Projekt ist im
> Alpha-Stadium. Bitte nur mit Daten verwenden, von denen es eine weitere Sicherung gibt.

Stand: Phase 1 (Kern) – Login mit 2FA, Zwei-Wege-Sync, Konfliktwarteschlange, Papierkorb,
Daemon und CLI. Ausführliche Anleitung: [**Benutzerhandbuch**](docs/handbuch.md).
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

Required Notice: Copyright (c) 2026 Krischan (https://github.com/kafast)

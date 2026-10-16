# ADR 0006: Geheimnisse im Secret Service bzw. in einer AES-256-GCM-Datei

- Status: angenommen
- Datum: 2026-10-16

## Kontext
Für die Wiederaufnahme ohne Passwort braucht der Daemon die Sitzungstokens und die
Schlüssel-Passphrasen des Kontos (`CLAUDE.md`, 3.1 und 9). Beide sind so schutzwürdig wie das
Passwort. Auf dem Desktop gibt es einen Secret Service (GNOME Keyring, `KWallet`), auf dem
Server (Container, headless) nicht.

## Entscheidung
- Port `SecretStore` in der Anwendungsschicht, zwei Adapter in `protonsync-adapter-secrets`:
  - `KeyringStore` (Crate `keyring`, reine Rust-Anbindung an den Secret Service über D-Bus),
    hinter dem Cargo-Feature `keyring` (Standard; der Server-Build lässt es weg).
  - `EncryptedFileStore`: eine Datei pro Schlüssel im Datenverzeichnis, AES-256-GCM
    (Format `PSS1` + 12-Byte-Nonce + Chiffrat, Schlüsselname als zusätzliche authentifizierte
    Daten), Rechte 0600 in einem 0700-Verzeichnis, atomar geschrieben.
- Der Hauptschlüssel der Datei kommt aus `$CREDENTIALS_DIRECTORY/protonsync-key`
  (systemd-Credentials), `PROTONSYNC_KEY_FILE` (Rechte höchstens 0600) oder `PROTONSYNC_KEY`.
- Gespeichert wird nie das Passwort, nur Tokens und Passphrasen (`StoredSession`); erneuerte
  Tokens schreibt der Adapter sofort zurück.

## Folgen
- Server-Images brauchen keinen D-Bus und keine Keyring-Bibliotheken.
- Wer Datenverzeichnis **und** Hauptschlüssel hat, kann die Sitzung übernehmen; deshalb gehören
  beide getrennt abgelegt (Schlüssel als Secret bzw. Credential, nicht im Volume).
- Der `keyring`-Zugriff ist blockierend; bei Bedarf später in `spawn_blocking` verlagern.

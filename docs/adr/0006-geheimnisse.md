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

---

## English

**ADR 0006: Secrets in the Secret Service or in an AES-256-GCM file**

- Status: accepted
- Date: 2026-10-16

### Context
To resume without a password, the daemon needs the session tokens and the
key passphrases of the account (`CLAUDE.md`, 3.1 and 9). Both are as sensitive as the
password. On the desktop there is a Secret Service (GNOME Keyring, `KWallet`); on the
server (container, headless) there is not.

### Decision
- Port `SecretStore` in the application layer, two adapters in `protonsync-adapter-secrets`:
  - `KeyringStore` (crate `keyring`, pure Rust binding to the Secret Service via D-Bus),
    behind the Cargo feature `keyring` (default; the server build leaves it out).
  - `EncryptedFileStore`: one file per key in the data directory, AES-256-GCM
    (format `PSS1` + 12-byte nonce + ciphertext, key name as additional authenticated
    data), permissions 0600 in a 0700 directory, written atomically.
- The file's master key comes from `$CREDENTIALS_DIRECTORY/protonsync-key`
  (systemd credentials), `PROTONSYNC_KEY_FILE` (permissions at most 0600) or `PROTONSYNC_KEY`.
- The password is never stored, only tokens and passphrases (`StoredSession`); the adapter
  writes refreshed tokens back immediately.

### Consequences
- Server images need neither D-Bus nor keyring libraries.
- Anyone who has the data directory **and** the master key can take over the session; therefore
  both must be stored separately (the key as a secret or credential, not in the volume).
- The `keyring` access is blocking; move it to `spawn_blocking` later if needed.

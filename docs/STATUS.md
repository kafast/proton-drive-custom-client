# Projektstand

Diese Datei wird am Ende jeder Sitzung aktualisiert (siehe `CLAUDE.md`, Abschnitt 8).

## Aktuelle Phase
**Phase 1 – Kern (MVP): abgeschlossen** (2026-10-16), Branch `feature/phase-1-kern`, PR #1.
Vor Phase 2 Rückmeldung des Auftraggebers abwarten.

## Fertig
### Phase 0 – Fundament
- Faktencheck (`CLAUDE.md`, Abschnitt 10), Workspace, Lint-Regeln, VS-Code-Konfiguration, CI.
- Domäne (Knoten, Sync-Paare, Drei-Wege-Entscheidung mit Property-Tests, Papierkorb, Freigaberegel),
  Ports, `FakeRemote`, Contract-Tests, ADRs 0001–0004.

### Phase 1 – Kern
- **Anwendungsschicht:** Ports `LocalStorage`, `StateRepo`, `Account`; Sync-Engine (zwei Wege,
  nur Download, nur Upload) mit Plan/Ausführung, Konfliktwarteschlange (beide Versionen bleiben),
  Papierkorb mit Wiederherstellung und Ablauf, `ensure_folder`.
- **`adapter-proton`:** Login mit SRP, TOTP-2FA und Mailbox-Passwort; Sitzung ohne Passwort
  fortsetzen (Tokens + Schlüssel-Passphrasen), Token-Refresh wird gespeichert; `RemoteStorage`
  über `proton-drive-rs` 0.9.1; App-Kennung `external-drive-protonsync@0.1.0-alpha`.
- **`adapter-state`:** SQLite mit Journal; **`adapter-fs`:** lokales Dateisystem, atomares
  Schreiben, inotify-Beobachter (`notify`).
- **`adapter-secrets`:** Secret Service (`keyring`) und AES-256-GCM-Datei für Server (ADR 0006).
- **`api`:** versionierte Daemon-API, zeilenweises JSON über Unix-Socket (ADR 0005).
- **`daemon` (`protonsyncd`):** TOML-Konfiguration, Zeitplaner (lokale Änderungen mit Entprellung,
  Proton-Events mindestens alle 30 s, voller Abgleich als Sicherheitsnetz, tägliches Leeren des
  Papierkorbs), Ereignisse an Abonnenten, `dry_run = true` als Voreinstellung.
- **`cli` (`protonsync`):** login/logout, status, sync (`--apply`), conflicts, trash, config example;
  `--json`, Exit-Codes, Deutsch/Englisch.
- **Verpackung:** Server-Image (musl, distroless, nicht root), `compose.yaml`, systemd-Units für
  Desktop (User-Dienst) und Server (gehärtet, systemd-Credentials).
- Dokumentation: README mit ersten Schritten, Benutzerhandbuch (`docs/handbuch.md`), ADR 0005 und 0006.

## Tests
- 224 Tests ohne Konto (`cargo test`), 18 als `#[ignore]` markierte Live-Tests.
- Gezielt: `cargo test -p protonsync-application` (Sync-Szenarien), `-p protonsync-daemon`,
  `-p protonsync-cli` (Snapshot-Tests mit `insta`).
- Live-Tests gegen ein echtes Konto (nur im Testordner, Standard `protonsync-live-tests`):
  ```sh
  PROTONSYNC_LIVE_USERNAME=… PROTONSYNC_LIVE_PASSWORD=… [PROTONSYNC_LIVE_DATA_PASSWORD=…] \
  [PROTONSYNC_LIVE_TOTP=…] cargo test -p protonsync-adapter-proton --test live_contract \
    -- --ignored --test-threads=1
  ```
- Abdeckung in der CI ≥ 80 % (zuletzt gemessen 81,8 %, seither weitere Tests).

## Als Nächstes (Phase 2: Upload-Jobs und Web-Oberfläche)
1. Domäne `Job` (Quelle, Glob, Ziel, Cron/„bei neuer Datei“, Aufbewahrung, Verifikation).
2. Use Case `RunJob` mit Aufbewahrung über den Papierkorb, Szenariotests.
3. Daemon: Job-Zeitplaner, API-Erweiterung (`jobs`), CLI-Befehle.
4. Web-Oberfläche (axum), standardmäßig aus, mit Anmeldung.

## Offene Punkte / bekannte Probleme
- **Noch nicht gegen ein echtes Konto getestet:** Live-Tests und Server-Image laufen in der
  Bauumgebung nicht (kein Konto, Paketquellen gesperrt). Das Image wird in der CI gebaut.
- Jeder Abgleich liest den ganzen Remote-Baum; Events lösen nur aus, wann abgeglichen wird.
  Inkrementeller Abgleich auf Basis der Events ist eine spätere Optimierung.
- Proton meldet neue Dateien als `Updated`, nicht als `Created` (Contract-Test lässt beides zu).
- Meldungen des Daemons nur auf Deutsch, Hilfetexte der CLI ebenfalls; die Ausgaben der CLI sind
  zweisprachig.
- `keyring`-Zugriffe blockieren kurz den Tokio-Thread; ggf. in `spawn_blocking` verlagern.
- RUSTSEC-2023-0071 (`rsa`, Marvin-Angriff) in `deny.toml` begründet ignoriert: kein
  Netzwerk-Orakel, kein Patch verfügbar.
- Pin/Unpin, Platzhalter, Upload-Jobs, Fotos folgen in späteren Phasen.
- Windows-Lauffähigkeit als untergeordneter Punkt nach Phase 6 (`CLAUDE.md`, Abschnitt 9).

---

## English

**Project status**

This file is updated at the end of every session (see `CLAUDE.md`, section 8).

### Current phase
**Phase 1, core (MVP): completed** (2026-10-16), branch `feature/phase-1-kern`, PR #1.
Wait for feedback from the client before starting phase 2.

### Done
#### Phase 0, foundation
- Fact check (`CLAUDE.md`, section 10), workspace, lint rules, VS Code configuration, CI.
- Domain (nodes, sync pairs, three-way decision with property tests, trash, eviction rule),
  ports, `FakeRemote`, contract tests, ADRs 0001 to 0004.

#### Phase 1, core
- **Application layer:** ports `LocalStorage`, `StateRepo`, `Account`; sync engine (two-way,
  download only, upload only) with plan/execute, conflict queue (both versions are kept),
  trash with restore and expiry, `ensure_folder`.
- **`adapter-proton`:** login with SRP, TOTP 2FA and mailbox password; resume a session without
  the password (tokens + key passphrases), token refresh is persisted; `RemoteStorage`
  via `proton-drive-rs` 0.9.1; app identifier `external-drive-protonsync@0.1.0-alpha`.
- **`adapter-state`:** SQLite with journal; **`adapter-fs`:** local file system, atomic
  writes, inotify watcher (`notify`).
- **`adapter-secrets`:** Secret Service (`keyring`) and AES-256-GCM file for servers (ADR 0006).
- **`api`:** versioned daemon API, line-delimited JSON over a Unix socket (ADR 0005).
- **`daemon` (`protonsyncd`):** TOML configuration, scheduler (local changes with debouncing,
  Proton events at least every 30 s, full sync as a safety net, daily emptying of the
  trash), events to subscribers, `dry_run = true` as the default.
- **`cli` (`protonsync`):** login/logout, status, sync (`--apply`), conflicts, trash, config example;
  `--json`, exit codes, German/English.
- **Packaging:** server image (musl, distroless, non-root), `compose.yaml`, systemd units for
  desktop (user service) and server (hardened, systemd credentials).
- Documentation: README with getting started, user manual (`docs/handbuch.md`), ADR 0005 and 0006.

### Tests
- 224 tests without an account (`cargo test`), 18 live tests marked `#[ignore]`.
- Targeted: `cargo test -p protonsync-application` (sync scenarios), `-p protonsync-daemon`,
  `-p protonsync-cli` (snapshot tests with `insta`).
- Live tests against a real account (only in the test folder, default `protonsync-live-tests`):
  ```sh
  PROTONSYNC_LIVE_USERNAME=… PROTONSYNC_LIVE_PASSWORD=… [PROTONSYNC_LIVE_DATA_PASSWORD=…] \
  [PROTONSYNC_LIVE_TOTP=…] cargo test -p protonsync-adapter-proton --test live_contract \
    -- --ignored --test-threads=1
  ```
- Coverage in CI ≥ 80 % (last measured 81.8 %, more tests added since).

### Next (phase 2: upload jobs and web interface)
1. Domain `Job` (source, glob, target, cron/"on new file", retention, verification).
2. Use case `RunJob` with retention via the trash, scenario tests.
3. Daemon: job scheduler, API extension (`jobs`), CLI commands.
4. Web interface (axum), off by default, with login.

### Open items / known issues
- **Not yet tested against a real account:** live tests and the server image do not run in the
  build environment (no account, package sources blocked). The image is built in CI.
- Every sync reads the whole remote tree; events only trigger when a sync happens.
  Incremental sync based on events is a later optimization.
- Proton reports new files as `Updated`, not as `Created` (the contract test accepts both).
- Daemon messages are German only, and so are the CLI help texts; the CLI output is
  bilingual.
- `keyring` calls briefly block the Tokio thread; move them to `spawn_blocking` if needed.
- RUSTSEC-2023-0071 (`rsa`, Marvin attack) is ignored in `deny.toml` with a justification: no
  network oracle, no patch available.
- Pin/unpin, placeholders, upload jobs and photos follow in later phases.
- Windows support as a lower-priority item after phase 6 (`CLAUDE.md`, section 9).

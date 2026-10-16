# Projektstand

Diese Datei wird am Ende jeder Sitzung aktualisiert (siehe `CLAUDE.md`, Abschnitt 8).

## Aktuelle Phase
**Phase 2 – Upload-Jobs und Web-Oberfläche: in Arbeit.** Upload-Jobs sind fertig
(Branch `feature/phase-2-upload-jobs`), als Nächstes kommt die Web-Oberfläche.
Phase 1 wurde mit PR #1 abgeschlossen, der inkrementelle Abgleich mit PR #3.
Seit 2026-10-16 arbeitet Claude selbstständig weiter und mergt eigene PRs bei grüner CI.

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
- Inkrementeller Abgleich über Proton-Ereignisse (ADR 0007, PR #3).

### Phase 2 – Upload-Jobs (ADR 0008)
- **Domäne:** `UploadJob`, eigener Cron-Parser (`CronSchedule`, Ortszeit), Dateimuster
  (`GlobPattern`), Aufbewahrung (`Retention`, neueste Datei bleibt immer).
- **Anwendung:** `UploadJobRunner` (Ruhezeit, unverändert per Größe/SHA-1, neue Revisionen,
  Prüfung nach Upload, Aufbewahrung nur nach fehlerfreiem Lauf über den Papierkorb);
  Port `JobLog` mit SQLite-Tabelle `job_runs`.
- **Daemon:** `[[job]]` in der Konfiguration, Zeitplan per Cron (Zeitzone über `jiff`, `TZ`) oder
  „bei neuer Datei“ (Watcher + Prüfung alle 5 Minuten ohne Proton-Zugriff), Wiederholung mit
  Backoff, Ereignisse `JobStarted`/`JobFinished`/`JobGaveUp`, Papierkorb-Besitzer `job:<id>`.
- **API/CLI:** `jobs`, `run_job`, `job_history`; `protonsync jobs [list|run|history]`.
- Handbuch (de/en) Abschnitt 4.5, `compose.yaml` mit `TZ`.

## Tests
- 290 Tests ohne Konto (`cargo test`), 18 als `#[ignore]` markierte Live-Tests.
- Gezielt: `cargo test -p protonsync-application` (Sync-Szenarien), `-p protonsync-daemon`,
  `-p protonsync-cli` (Snapshot-Tests mit `insta`).
- Live-Tests gegen ein echtes Konto (nur im Testordner, Standard `protonsync-live-tests`):
  ```sh
  PROTONSYNC_LIVE_USERNAME=… PROTONSYNC_LIVE_PASSWORD=… [PROTONSYNC_LIVE_DATA_PASSWORD=…] \
  [PROTONSYNC_LIVE_TOTP=…] cargo test -p protonsync-adapter-proton --test live_contract \
    -- --ignored --test-threads=1
  ```
- Abdeckung in der CI ≥ 80 % (zuletzt gemessen 81,8 %, seither weitere Tests).

## Als Nächstes (Phase 2: Web-Oberfläche)
1. Crate `web` (axum), standardmäßig aus, nur lokal/Heimnetz, Anmeldung erforderlich,
   TLS-fähig oder hinter Reverse Proxy; spricht nur mit der Daemon-API.
2. Seiten: Status, Konflikte, Papierkorb, Upload-Jobs (Verlauf, jetzt ausführen), Einstellungen.
3. Danach Bericht zu Phase 2.

## Offene Punkte / bekannte Probleme
- **Noch nicht gegen ein echtes Konto getestet:** Live-Tests und Server-Image laufen in der
  Bauumgebung nicht (kein Konto, Paketquellen gesperrt). Das Image wird in der CI gebaut.
- Erledigt (ADR 0007): Abgleiche lesen den Remote-Baum nicht mehr vollständig, sondern holen nur
  die Proton-Ereignisse nach. Nach einem Neustart wird einmal vollständig gelesen.
- Proton meldet neue Dateien als `Updated`, nicht als `Created` (Contract-Test lässt beides zu).
- Meldungen des Daemons nur auf Deutsch, Hilfetexte der CLI ebenfalls; die Ausgaben der CLI sind
  zweisprachig.
- `keyring`-Zugriffe blockieren kurz den Tokio-Thread; ggf. in `spawn_blocking` verlagern.
- RUSTSEC-2023-0071 (`rsa`, Marvin-Angriff) in `deny.toml` begründet ignoriert: kein
  Netzwerk-Orakel, kein Patch verfügbar.
- Pin/Unpin, Platzhalter, Fotos folgen in späteren Phasen.
- Upload-Jobs holen verpasste Cron-Termine nicht nach (Rechner aus); Benachrichtigung bei
  aufgegebenen Jobs vorerst nur über Protokoll und Ereignisstrom.
- Windows-Lauffähigkeit als untergeordneter Punkt nach Phase 6 (`CLAUDE.md`, Abschnitt 9).

---

## English

**Project status**

This file is updated at the end of every session (see `CLAUDE.md`, section 8).

### Current phase
**Phase 2, upload jobs and web interface: in progress.** Upload jobs are done
(branch `feature/phase-2-upload-jobs`); the web interface comes next.
Phase 1 was completed with PR #1, incremental sync with PR #3.
Since 2026-10-16 Claude continues on its own and merges its own PRs when CI is green.

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
- Incremental sync through Proton events (ADR 0007, PR #3).

#### Phase 2, upload jobs (ADR 0008)
- **Domain:** `UploadJob`, own cron parser (`CronSchedule`, local time), file patterns
  (`GlobPattern`), retention (`Retention`, the newest file always stays).
- **Application:** `UploadJobRunner` (settle time, unchanged by size/SHA-1, new revisions,
  verification after upload, retention through the trash only after a run without errors);
  port `JobLog` with the SQLite table `job_runs`.
- **Daemon:** `[[job]]` in the configuration, schedule by cron (time zone via `jiff`, `TZ`) or
  "on new file" (watcher plus a check every 5 minutes without contacting Proton), retries with
  backoff, events `JobStarted`/`JobFinished`/`JobGaveUp`, trash owner `job:<id>`.
- **API/CLI:** `jobs`, `run_job`, `job_history`; `protonsync jobs [list|run|history]`.
- Manual (de/en) section 4.5, `compose.yaml` with `TZ`.

### Tests
- 290 tests without an account (`cargo test`), 18 live tests marked `#[ignore]`.
- Targeted: `cargo test -p protonsync-application` (sync scenarios), `-p protonsync-daemon`,
  `-p protonsync-cli` (snapshot tests with `insta`).
- Live tests against a real account (only in the test folder, default `protonsync-live-tests`):
  ```sh
  PROTONSYNC_LIVE_USERNAME=… PROTONSYNC_LIVE_PASSWORD=… [PROTONSYNC_LIVE_DATA_PASSWORD=…] \
  [PROTONSYNC_LIVE_TOTP=…] cargo test -p protonsync-adapter-proton --test live_contract \
    -- --ignored --test-threads=1
  ```
- Coverage in CI ≥ 80 % (last measured 81.8 %, more tests added since).

### Next (phase 2: web interface)
1. Crate `web` (axum), off by default, local network only, login required, TLS-capable or
   behind a reverse proxy; talks only to the daemon API.
2. Pages: status, conflicts, trash, upload jobs (history, run now), settings.
3. Then the phase 2 report.

### Open items / known issues
- **Not yet tested against a real account:** live tests and the server image do not run in the
  build environment (no account, package sources blocked). The image is built in CI.
- Done (ADR 0007): syncs no longer read the whole remote tree but only catch up on Proton
  events. After a restart the tree is read in full once.
- Proton reports new files as `Updated`, not as `Created` (the contract test accepts both).
- Daemon messages are German only, and so are the CLI help texts; the CLI output is
  bilingual.
- `keyring` calls briefly block the Tokio thread; move them to `spawn_blocking` if needed.
- RUSTSEC-2023-0071 (`rsa`, Marvin attack) is ignored in `deny.toml` with a justification: no
  network oracle, no patch available.
- Pin/unpin, placeholders and photos follow in later phases.
- Upload jobs do not make up missed cron times (computer off); for now, notification about jobs
  that gave up only goes through the log and the event stream.
- Windows support as a lower-priority item after phase 6 (`CLAUDE.md`, section 9).

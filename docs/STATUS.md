# Projektstand

Diese Datei wird am Ende jeder Sitzung aktualisiert (siehe `CLAUDE.md`, Abschnitt 8).

## Aktuelle Phase
**Phase 3 – Platzhalter: fertig** (ADR 0010, PR #7). Als Nächstes Phase 4 (Qt-Tray-App).
Phase 2 wurde mit PR #5 (Upload-Jobs) und PR #6 (Web-Oberfläche) abgeschlossen.
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

### Phase 2 – Web-Oberfläche (ADR 0009)
- **Crate `web`:** axum 0.8 (ohne Standard-Features), serverseitig erzeugtes HTML ohne
  JavaScript, Deutsch/Englisch; Seiten Übersicht, Abgleich (planen/ausführen), Konflikte,
  Papierkorb (wiederherstellen, abgelaufene löschen), Upload-Jobs (Verlauf, ausführen),
  Einstellungen (nur lesend). Spricht nur über den Trait `Backend` mit dem Daemon.
- **Schutz:** Argon2id-Passwort, Sitzungs-Cookie (HttpOnly, SameSite=Strict, Secure mit TLS),
  CSRF-Kennung, Sperre nach 5 Fehlversuchen, Netzfilter (Standard: private Netze), CSP und weitere
  Sicherheits-Header, Zeitlimits für Kopfzeilen und TLS-Handschlag, HTTPS über rustls.
- **Daemon:** Abschnitt `[web]` (standardmäßig aus, `127.0.0.1:8470`), `protonsyncd hash-password`;
  scheitert der Start der Oberfläche, läuft der Daemon weiter.
- Handbuch (de/en) Abschnitt 7.3, Beispiele in `config example`, `config.toml` und `compose.yaml`.

### Phase 3 – Platzhalter (ADR 0010)
- **Domäne:** Zwischenspeicher-Einträge mit Blockbitmap, Anheft-Regeln (nächstgelegene Regel gilt),
  Freigaberegel (Alter und Größenlimit).
- **Anwendung:** FUSE-unabhängiges `VirtualDrive` (Nachschlagen, Auflisten, Lesen per
  Byte-Bereich mit Vorauslesen, Schreiben, Anlegen, Löschen in den Papierkorb, Umbenennen,
  Kürzen), Hochladen beim Schließen mit Konfliktkopie, Wiederholung offener Uploads, Vorabladen
  angehefteter Dateien, Freigabe nach Regel und sofort für einen Pfad (`free`).
  Ports `ContentCache` und `DriveRepo`.
- **Adapter:** `FsContentCache` (dünn besetzte Dateien, 0700/0600), SQLite-Tabellen
  `cache_records` und `pin_rules` (Migration 3), Contract-Tests für beide Ports.
- **Crate `fuse`:** `fuser` 0.18 (MIT, ohne libfuse), eine Tokio-Aufgabe pro Anfrage, Abbildung der
  Fehler auf errno, Aufräumen verwaister Mounts, echte Mount-Tests (`#[ignore]`, laufen in der CI).
- **Daemon:** `[[mount]]` und `[cache]`, Einhängen nach Anmeldung, Aushängen beim Abmelden/Beenden,
  Pflege alle `event_poll_seconds`, stündliche Freigabe (im Dry-Run nur geplant),
  Papierkorb-Besitzer `mount:<id>`; Feature `fuse` (Server-Build ohne).
- **API/CLI:** `mounts`, `pin`, `free`; `protonsync mounts|pin|unpin|free [--apply]`.
- Handbuch (de/en) Abschnitte 4.7 und 6.5.

## Tests
- 385 Tests ohne Konto (`cargo test`), 23 als `#[ignore]` markierte Tests (Live-Tests und echte
  FUSE-Mounts: `cargo test -p protonsync-fuse --test mount -- --ignored`).
- Gezielt: `cargo test -p protonsync-application` (Sync-Szenarien), `-p protonsync-daemon`,
  `-p protonsync-cli` (Snapshot-Tests mit `insta`), `-p protonsync-web` (Seiten gegen einen
  geskripteten Daemon).
- Live-Tests gegen ein echtes Konto (nur im Testordner, Standard `protonsync-live-tests`):
  ```sh
  PROTONSYNC_LIVE_USERNAME=… PROTONSYNC_LIVE_PASSWORD=… [PROTONSYNC_LIVE_DATA_PASSWORD=…] \
  [PROTONSYNC_LIVE_TOTP=…] cargo test -p protonsync-adapter-proton --test live_contract \
    -- --ignored --test-threads=1
  ```
- Abdeckung in der CI ≥ 80 % (zuletzt gemessen 81,8 %, seither weitere Tests).

## Als Nächstes (Phase 4: Qt-Tray-App)
1. ADR zur Qt-Anbindung (`cxx-qt` mit QML, Qt nur dynamisch gelinkt).
2. Tray-Icon mit Status, Einstellungsfenster, Konfliktdialog, Fortschritt, Benachrichtigungen,
   Deutsch/Englisch; spricht nur über die Daemon-API.

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
- Fotos folgen in Phase 5.
- Laufwerke: Die Web-Oberfläche zeigt sie nicht an (sie ist für den Server gedacht, FUSE nur für den
  Desktop). Freigeben prüft vor dem Entfernen nur, ob die Datei in Proton noch existiert (lokal
  unveränderte Inhalte liegen dort sicher). Der Zwischenspeicher wird nicht verschlüsselt (er liegt
  im Datenverzeichnis des Benutzers mit Rechten 0700); Festplattenverschlüsselung empfohlen.
  Noch nicht mit einem echten Konto getestet.
- Upload-Jobs holen verpasste Cron-Termine nicht nach (Rechner aus); Benachrichtigung bei
  aufgegebenen Jobs vorerst nur über Protokoll und Ereignisstrom.
- Web-Oberfläche: Einstellungen nur lesend (Ändern bräuchte eine API zum Schreiben und Neuladen
  der Konfiguration); keine Proton-Anmeldung über das Web; Sitzungen nur im Speicher.
- Windows-Lauffähigkeit als untergeordneter Punkt nach Phase 6 (`CLAUDE.md`, Abschnitt 9).

---

## English

**Project status**

This file is updated at the end of every session (see `CLAUDE.md`, section 8).

### Current phase
**Phase 3, placeholders: done** (ADR 0010, PR #7). Next is phase 4 (Qt tray app).
Phase 2 was completed with PR #5 (upload jobs) and PR #6 (web interface).
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

#### Phase 2, web interface (ADR 0009)
- **Crate `web`:** axum 0.8 (without default features), server-rendered HTML without
  JavaScript, German/English; pages overview, sync (plan/run), conflicts, trash (restore, purge
  expired), upload jobs (history, run), settings (read-only). Talks to the daemon only through
  the `Backend` trait.
- **Protection:** Argon2id password, session cookie (HttpOnly, SameSite=Strict, Secure with TLS),
  CSRF token, lockout after 5 failed attempts, network filter (default: private networks), CSP and
  other security headers, timeouts for headers and the TLS handshake, HTTPS via rustls.
- **Daemon:** section `[web]` (off by default, `127.0.0.1:8470`), `protonsyncd hash-password`;
  if the interface fails to start, the daemon keeps running.
- Manual (de/en) section 7.3, examples in `config example`, `config.toml` and `compose.yaml`.

#### Phase 3, placeholders (ADR 0010)
- **Domain:** cache records with a block bitmap, pin rules (the closest rule wins), eviction
  policy (age and size limit).
- **Application:** FUSE-independent `VirtualDrive` (lookup, listing, byte-range reads with
  read-ahead, writing, creating, deleting into the trash, renaming, truncating), upload on close
  with conflict copy, retry of pending uploads, prefetch of pinned files, eviction by policy and
  immediately for a path (`free`). Ports `ContentCache` and `DriveRepo`.
- **Adapters:** `FsContentCache` (sparse files, 0700/0600), SQLite tables `cache_records` and
  `pin_rules` (migration 3), contract tests for both ports.
- **Crate `fuse`:** `fuser` 0.18 (MIT, without libfuse), one Tokio task per request, errors mapped
  to errno, cleanup of stale mounts, real mount tests (`#[ignore]`, run in CI).
- **Daemon:** `[[mount]]` and `[cache]`, mounting after login, unmounting on logout/shutdown,
  maintenance every `event_poll_seconds`, hourly eviction (only planned in dry run), trash owner
  `mount:<id>`; feature `fuse` (the server build has none).
- **API/CLI:** `mounts`, `pin`, `free`; `protonsync mounts|pin|unpin|free [--apply]`.
- Manual (de/en) sections 4.7 and 6.5.

### Tests
- 385 tests without an account (`cargo test`), 23 tests marked `#[ignore]` (live tests and real
  FUSE mounts: `cargo test -p protonsync-fuse --test mount -- --ignored`).
- Targeted: `cargo test -p protonsync-application` (sync scenarios), `-p protonsync-daemon`,
  `-p protonsync-cli` (snapshot tests with `insta`), `-p protonsync-web` (pages against a
  scripted daemon).
- Live tests against a real account (only in the test folder, default `protonsync-live-tests`):
  ```sh
  PROTONSYNC_LIVE_USERNAME=… PROTONSYNC_LIVE_PASSWORD=… [PROTONSYNC_LIVE_DATA_PASSWORD=…] \
  [PROTONSYNC_LIVE_TOTP=…] cargo test -p protonsync-adapter-proton --test live_contract \
    -- --ignored --test-threads=1
  ```
- Coverage in CI ≥ 80 % (last measured 81.8 %, more tests added since).

### Next (phase 4: Qt tray app)
1. ADR on the Qt binding (`cxx-qt` with QML, Qt linked dynamically only).
2. Tray icon with status, settings window, conflict dialog, progress, notifications,
   German/English; talks only through the daemon API.

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
- Photos follow in phase 5.
- Drives: the web interface does not show them (it is meant for servers, FUSE is desktop only).
  Before removing, freeing only checks that the file still exists in Proton (unchanged local
  content is safely stored there). The cache is not encrypted (it lives in the user's data
  directory with permissions 0700); disk encryption is recommended. Not yet tested with a real
  account.
- Upload jobs do not make up missed cron times (computer off); for now, notification about jobs
  that gave up only goes through the log and the event stream.
- Web interface: settings are read-only (changing them would need an API for writing and
  reloading the configuration); no Proton login over the web; sessions in memory only.
- Windows support as a lower-priority item after phase 6 (`CLAUDE.md`, section 9).

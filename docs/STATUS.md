# Projektstand

Diese Datei wird am Ende jeder Sitzung aktualisiert (siehe `CLAUDE.md`, Abschnitt 8).

## Aktuelle Phase
**Phase 6 – Dateimanager-Integration, Pakete, Feinschliff: fertig** (PR #10, #11 und #12).
Damit sind alle Phasen aus `CLAUDE.md` umgesetzt; als Nächstes stehen die Punkte aus Abschnitt 9
an (siehe „Als Nächstes“). Phase 5 (Fotos) wurde mit PR #9 abgeschlossen, Phase 4 (Tray-App) mit PR #8, Phase 3 (Platzhalter) wurde mit PR #7 abgeschlossen.
Phase 2 wurde mit PR #5 (Upload-Jobs) und PR #6 (Web-Oberfläche) abgeschlossen.
Phase 1 wurde mit PR #1 abgeschlossen, der inkrementelle Abgleich mit PR #3.
Seit 2026-10-16 arbeitet Claude selbstständig weiter. Seit 2026-10-20 mergt nur noch der
PR-Review-Thread (siehe `CLAUDE.md`, Abschnitt 8); alle anderen Sitzungen öffnen nur PRs.

## Fertig

### Nach Phase 6 – Versionsarchiv auf beiden Seiten (ADR 0020, 2026-10-29)
- Neuer Schlüssel `archive_versions = true` je `[[pair]]` (jede Richtung): nichts wird gelöscht
  oder überschrieben. Löschungen und ersetzte Versionen wandern lokal und in Proton nach
  `_archiv/` im selben Ordner (Namensregel aus ADR 0019); in Proton wird der Knoten verschoben.
  Die Archive werden nicht synchronisiert, gleicher Inhalt wird nicht archiviert, `protect_remote`
  hat Vorrang. Domänenregel `conflict::archive_instead` mit Property-Tests, 21 Szenario-Tests in
  `crates/application/tests/archive_versions.rs`. API-Werte `archive_local`, `archive_remote`,
  `archive_remote_and_upload`, Zahl `archived`; Handbuch (de/en) 1, 4.3, 4.4, 6.4; Beispielkonfig.
- Offen: keine Aufbewahrungsregel für die Archive; Konfliktlösung nutzt lokal weiter den Papierkorb.

### Nach Phase 6 – Sicherheitstests der Sync-Modi (2026-10-29)
- Testkit: `RecordingRemote` und `RecordingPhotoLibrary` protokollieren jeden ändernden Aufruf an
  Proton (anlegen, hochladen, umbenennen, verschieben, Papierkorb, wiederherstellen, löschen);
  Downloads können gezielt fehlschlagen.
- `crates/application/tests/read_only_modes.rs`: `download_only` und `backup_only` ändern Proton nie
  (geprüft nach jedem Lauf: kein ändernder Aufruf, Remote-Baum unverändert). Szenarien: lokale
  Änderungen, Löschungen, Umbenennungen, Konflikte, Netzfehler mitten im Lauf, abgelaufene
  Anmeldung, Neustart, Ereignisse verloren oder mit Lücke, Probelauf, Konfliktlösung, Papierkorb;
  dazu Property-Tests mit zufälligen Änderungen auf beiden Seiten (`backup_only` verliert dabei auch
  nie eine lokale Version).
- `tests/mode_matrix.rs`: Tabelle mit 54 Fällen (jede Richtung, mit und ohne `protect_remote`, jede
  Einzeländerung und die typischen Konflikte) mit erwarteten Proton-Aufrufen und lokalem Ergebnis;
  Property-Tests: `protect_remote` fügt in Proton nur hinzu.
- `tests/photo_modes.rs`: dasselbe für Fotos (`download_only` ändert nichts, `protect_remote`
  entfernt nichts).
- Dabei gefunden und behoben (`download_only` hat Proton verändert):
  lokal geändert + in Proton gelöscht lud die Datei wieder hoch; ein in Proton gelöschter Ordner mit
  lokalen Änderungen wurde in Proton neu angelegt und befüllt. Außerdem scheiterte der Download in
  einem lokal gelöschten Ordner, in dem sich in Proton etwas geändert hatte, bei jedem Lauf; der
  Ordner wird jetzt lokal neu angelegt. Handbuch (de/en) 1 und 6.3.

### Nach Phase 6 – Backup-Modus mit Versionsarchiv (ADR 0019, 2026-10-28)
- Neue Richtung `direction = "backup_only"` je `[[pair]]`: alles aus Proton lokal speichern, nie
  hochladen, nie löschen. Vor jeder neuen Version wandert die alte lokale Datei nach `_archiv/`
  im selben Ordner (`JJJJ-MM-TT_HH-MM-SS_name`, UTC); in Proton gelöschte Dateien bleiben lokal.
  Domänenregel `conflict::decide_backup` mit Property-Tests, Namensregel `archive::archived_name`,
  Szenario-Tests in `crates/application/tests/backup_only.rs`; in `[photos]` abgelehnt.
  API-Werte `backup_only`, `archive_and_download`, `keep_local_copy`; Handbuch (de/en) 1 und 4.3.

### Nach Phase 6 – Windows neu bewertet (ADR 0018, 2026-10-23)
- Entscheidung des Auftraggebers: nur portabel halten (Option A). Der empfohlene Port ohne
  Platzhalter in sechs Stufen bleibt als Plan in Reserve im ADR; WinFsp scheidet wegen der
  Lizenz aus.
- Stufe 0 umgesetzt: `FsLocal::absolute` lässt nur normale Pfadkomponenten zu (unter Windows
  sonst Verlassen des Sync-Ordners per `..\x` oder `C:x`), Standardpfade ohne `/` im Text,
  CI-Job `windows` (clippy für die portablen Crates, Tests für Domäne, Anwendung, `adapter-fs`).

### Nach Phase 6 – Wettlauf beim Überschreiben im Laufwerk behoben (Issue #35)
- Ursache des wackelnden FUSE-Tests `large_files_are_read_and_overwritten_correctly`: Der Upload
  nach dem Schließen lief noch, während die Datei erneut geschrieben oder per Hilfsdatei
  ersetzt wurde. Folge: unnötige Konfliktkopie oder der ältere Inhalt gewann (betraf auch echte
  Nutzer). Jetzt eine Sperre je Datei für Uploads und Umbenennungen (Nachtrag zu ADR 0010).
  Zwei Szenario-Tests in `crates/application/tests/virtual_drive.rs`; der FUSE-Test lief lokal
  vorher in 140 von 200 Läufen rot, danach 200 von 200 grün.

### Nach Phase 6 – Löschschutz (ADR 0016)
- Schlüssel `protect_remote` je `[[pair]]` und in `[photos]`: lokale Löschungen gehen nicht nach
  Proton; ersetzt eine lokale Datei eine vorhandene in Proton, entsteht ein Konflikt
  `local_modified_remote_protected` (beide Versionen bleiben, `keep-local` gesperrt).
  Domänenregel `conflict::protect_remote` mit Property-Test, Szenario-Tests in
  `crates/application/tests/protect_remote.rs`. Handbuch (de/en) 4.3, 4.8, 6.3.

### Nach Phase 6 – Benachrichtigungen und Wochenbericht (ADR 0017)
- Port `Notifier`, Crate `adapter-notify` (E-Mail per SMTP mit `lettre`, Webhook als JSON oder
  Text für ntfy), Tests gegen Fake-Server auf localhost.
- Daemon-Modul `notify` und Abschnitt `[notify]`: Konflikte (gebündelt), Sync-Fehler (einmal bis
  zur Erholung), Job-Fehler, abgelaufene Anmeldung und Wochenbericht (Cron, Standard Montag 8:00),
  jedes Thema je Kanal schaltbar; Wiederholung bei vorübergehenden Fehlern; Geheimnisse aus
  systemd-Credential, Datei (0600) oder Umgebungsvariable.
- Tray-App: Desktop-Meldungen je Thema abschaltbar (`[notification_topics]` in `tray.toml`).
- Handbuch (de/en) 4.9 und 6.6, `config example`.

### Phase 6 – Dateimanager-Integration (ADR 0013)
- **Laufwerk:** `VirtualDrive::path_status` liefert den Zustand eines Pfads (Platzhalter,
  teilweise, lokal, wartet auf Upload, angeheftet) nur aus dem Speicher, ohne Proton zu fragen.
- **API/Daemon:** Anfrage `PathStatus` für beliebig viele Pfade in Laufwerken, Sync-Paaren und im
  Fotoordner (`synced`, `pending`, `conflict` gegen den letzten Abgleich); `StateRepo::base_entry`.
- **CLI:** `protonsync file-status PFAD…`; `pin` und `unpin` nehmen mehrere Pfade.
- **Dateimanager** (`packaging/filemanager/`): Hilfsprogramm `protonsync-filemanager`,
  Dolphin-Service-Menü, Nautilus-Erweiterung mit Emblemen und Menü (Python, spricht direkt mit dem
  Socket, Zwischenspeicher je Ordner); Python-Tests und `shellcheck` in der CI.
- Handbuch (de/en) Abschnitt 6.9.

### Phase 6 – Feinschliff
- Secret Service über `block_in_place`, damit D-Bus-Wartezeiten keine anderen Aufgaben aufhalten.
- Upload-Jobs holen einen verpassten Termin nach dem Start einmal nach (Nachtrag zu ADR 0008).
- Web-Oberfläche: Seite „Fotos“ mit Abgleich und Duplikaten; in den Papierkorb erst nach einer
  zweiten Bestätigung.
- Tray-App, Übersicht: im Probelauf-Modus die Befehle für einen einmaligen Abgleich
  (`protonsync sync [--pair PAAR] [--apply]`) und das dauerhafte Abschalten; neuer Kasten
  „Was kann ich tun?“ mit Handgriffen, weiteren Befehlen und Links ins Handbuch.
- Krypto-Abhängigkeiten aktualisiert (2026-10-20): `aes-gcm` 0.11, `argon2` 0.6, `getrandom` 0.4
  (ersetzt Dependabot-PRs #21–#23). Alte Secret-Dateien und Passwort-Hashes bleiben gültig
  (Tests mit Daten der Vorversionen). `webpki-roots` 1.0.10 wegen RUSTSEC-2026-0381.

### Phase 6 – Pakete und Ressourcen
- **Arch/CachyOS:** `packaging/arch/PKGBUILD` (`protonsync-git`) mit Daemon, CLI, Tray-App,
  systemd-User- und System-Dienst, `sysusers.d`-Benutzer, Startmenü-Eintrag und
  Dateimanager-Dateien; die CI baut das Paket in `archlinux:base-devel`, installiert es und prüft
  den Inhalt.
- **Lizenztexte:** `packaging/licenses/third_party.py` sammelt die Lizenzen aller gelinkten
  Bibliotheken (`THIRD-PARTY-LICENSES.txt`) für Paket und Container.
- **Ressourcen:** im Leerlauf rund 13 MB RAM; die CI misst den Container bei jedem Lauf.
  Handbuch 3.5 (Paket) und 7.4 (Messwerte).

### Phase 5 – Fotos und Duplikate (ADR 0012)
- **Domäne:** `Photo`, Monatsordner `JJJJ/MM` mit eindeutigen Namen, Erkennung von Mediendateien,
  Duplikaterkennung (gleicher Inhalt bzw. gleicher Name und Aufnahmesekunde) mit Vorschlag zum
  Behalten und Prüfung der Löschauswahl (nie eine ganze Gruppe); Property-Tests.
- **Port `PhotoLibrary`** mit `FakePhotoLibrary` und Contract-Tests; `Calendar`-Port für Ortszeit;
  `RemoteBin`, damit derselbe Papierkorb-Use-Case auch Fotos wiederherstellt.
- **Use Cases:** Foto-Abgleich mit der Drei-Wege-Regel (Basis unter `photos:library`), Verknüpfen
  statt doppelt übertragen, lokal bearbeitete Fotos werden gemeldet statt hochgeladen, lokal wird
  nur gelöscht, was in Proton nachweislich fehlt; Index im Speicher über Ereignisse.
- **Proton-Adapter:** `ProtonPhotos` (Zeitleiste, Metadaten in Stapeln, zugehörige Fotos,
  Ereignisse des Foto-Volumes, Herunter- und Hochladen, Papierkorb); Live-Test nur lesend.
- **Daemon, API, CLI:** Abschnitt `[photos]`, Abgleich beim Start, nach lokalen Änderungen, nach
  Proton-Ereignissen und täglich vollständig; `protonsync photos [status|sync|duplicates [trash]]`
  mit Probelauf; Papierkorb `photos:library`; Tray meldet gescheiterte Foto-Abgleiche.
- Handbuch (de/en) Abschnitte 4.8 und 6.8.

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

### Phase 4 – Tray-App (ADR 0011)
- **Crate `client`:** Socket-Verbindung, Ereignis-Abonnement (`EventSubscription`) und Sprache,
  aus der CLI herausgelöst; CLI und Tray-App nutzen es gemeinsam.
- **Crate `tray-model` (ohne Qt):** Gesamtzustand und Icon, Tooltip, beschriftete Ansicht für
  Sync-Paare, Laufwerke und Konflikte, Benachrichtigungen, Anmeldeschritte (2FA,
  Mailbox-Passwort), Texte de/en, eigene Einstellungen (`tray.toml`) und Autostart.
- **Crate `gui-qt` (`protonsync-tray`):** cxx-qt 0.10 mit QML; Tray-Icon mit Menü, Statusfenster
  (Übersicht, Konflikte, Einstellungen), Anmeldedialog, Benachrichtigungen; Befehle und
  Ereignisse in je einem Hintergrund-Thread; Neuverbindung nach Daemon-Neustart.
  `unsafe` nur in den erzeugten Brückenmodulen.
- **CI:** Qt-Pakete, Bau und `protonsync-tray --check` ohne Bildschirm (QML-Fehler lassen die CI
  scheitern); Abdeckung ohne die Qt-Brücke.
- Handbuch (de/en) Abschnitte 3.4 und 6.6.

## Tests
- 598 Tests ohne Konto (`cargo test`, Stand 2026-10-29), 25 als `#[ignore]` markierte Tests (Live-Tests und echte
  FUSE-Mounts: `cargo test -p protonsync-fuse --test mount -- --ignored`).
- Tray-App ohne Bildschirm: `QT_QPA_PLATFORM=offscreen cargo run -p protonsync-gui-qt -- --check`.
- Dateimanager-Dateien: `python3 -m unittest discover -s packaging/filemanager/tests` (25 Tests).
- Gezielt: `cargo test -p protonsync-application` (Sync-Szenarien), `-p protonsync-daemon`,
  `-p protonsync-cli` (Snapshot-Tests mit `insta`), `-p protonsync-web` (Seiten gegen einen
  geskripteten Daemon).
- Live-Tests gegen ein echtes Konto (nur im Testordner, Standard `protonsync-live-tests`):
  ```sh
  PROTONSYNC_LIVE_USERNAME=… PROTONSYNC_LIVE_PASSWORD=… [PROTONSYNC_LIVE_DATA_PASSWORD=…] \
  [PROTONSYNC_LIVE_TOTP=…] cargo test -p protonsync-adapter-proton --test live_contract \
    -- --ignored --test-threads=1
  ```
- Abdeckung in der CI ≥ 80 % gesamt, Domäne ≥ 95 %, Use Cases ≥ 90 %
  (`scripts/coverage_gate.py`; gemessen am 2026-10-29: 85,6 % gesamt, Domäne 99,4 %,
  Anwendungsschicht 94,2 %, Sync-Planung 98,4 %, Sync-Ausführung 96,9 %).
- Sicherheit der Modi: `read_only_modes`, `mode_matrix` und `photo_modes` in
  `crates/application/tests` prüfen mit `RecordingRemote`, dass `download_only`, `backup_only` und
  `protect_remote` Proton nicht verändern (Szenarien und Property-Tests).
- Alle Prüfungen der CI lokal: `scripts/check.sh` (mit `--coverage` auch die Abdeckung).
- Qualitätsschranken in der CI (2026-10-20 ergänzt): `cargo doc` mit `-D warnings`, `--locked`
  (Cargo.lock muss passen), eigener Job für die Mindestversion Rust 1.96, Dependabot für Crates
  und GitHub Actions (wöchentlich).

## Erprobung mit dem echten Konto (2026-10-20, erster Durchgang)
Auf der CachyOS-VM des Auftraggebers, Sync-Paar `~/ProtonTest` ↔ `My files/04_Diverses/ProtonTest`,
`dry_run = true`.
- **Funktioniert:** Anmeldung (mit dem echten Konto) und Hochladen einer Datei.
- **Proton-Grenze:** Neue Ordner direkt in „Meine Dateien“ lehnt Proton mit HTTP 422 ab
  (Ordnerlimit; auch in der offiziellen App). Abhilfe: Sync-Ordner in einem bestehenden
  Unterordner wählen. Hinweis steht jetzt in der Beispielkonfiguration.
- **Behoben:** `protonsync sync` meldete „nicht angemeldet“, obwohl die Anmeldung stand und nur
  das Öffnen des Proton-Ordners gescheitert war. Jetzt nennt der Abgleich die echte Ursache und
  versucht die Verbindung bei jedem Abgleich neu (auch nach einem Netzausfall beim Start).
- **Neu:** `protonsync config example` zeigt jede Einstellung mit Standardwert, Erklärung und
  Auswahl, kommentiert auf Deutsch und Englisch; Tests halten die Datei mit dem Code im Gleichschritt.
- Offen aus der Checkliste [`docs/erprobung.md`](erprobung.md): Abgleich mit `--apply`,
  Konflikte, Papierkorb, FUSE-Laufwerk, Tray-App auf KDE/GNOME, Dolphin/Nautilus, Arch-Paket.

## Als Nächstes (nach Phase 6)
1. Erprobung mit dem echten Konto auf dem Rechner des Auftraggebers (Live-Tests, FUSE, Tray-App,
   Dolphin/Nautilus, Arch-Paket); Fehler daraus haben Vorrang.
   Checkliste dafür: [`docs/erprobung.md`](erprobung.md).
2. Erledigt: Code auf Englisch umgestellt (PR #13, #14, #15; ADR 0014).
3. Erledigt: Windows neu bewertet (ADR 0018): nur portabel halten, Stufe 0 umgesetzt, kein
   Windows-Port geplant.
4. Fertige Pakete für Dritte (2026-10-20): `.github/workflows/release.yml` baut bei einem Tag
   `vX.Y.Z` das Arch-Paket `protonsync-bin` (`packaging/arch-bin/PKGBUILD`), statische
   musl-Binaries und das Image `ghcr.io/kafast/protonsync` und legt ein GitHub-Release an.
   Offen: ersten Tag setzen (Entscheidung des Auftraggebers), danach ggf. AUR-Paket
   `protonsync-bin` aus dem angehängten `PKGBUILD`/`.SRCINFO`; Sichtbarkeit des ghcr-Pakets auf
   „öffentlich“ prüfen.

## Offene Punkte / bekannte Probleme
- Beim Öffnen eines Sync-Paars legt protonsync den Proton-Ordner an, falls er fehlt (`ensure_folder`),
  auch bei `download_only` und `backup_only`. Das ist die einzige verbleibende Änderung in Proton in
  diesen Modi (harmlos, aber z. B. bei einem Tippfehler im Pfad ein leerer Ordner). Entscheidung offen.
- **Erst teilweise mit einem echten Konto getestet:** Am 2026-10-20 liefen Anmeldung und
  Hochladen auf der CachyOS-VM des Auftraggebers; der Rest der Checkliste
  [`docs/erprobung.md`](erprobung.md) steht noch aus. Live-Tests und Server-Image laufen in der
  Bauumgebung nicht (kein Konto, Paketquellen gesperrt). Das Image wird in der CI gebaut.
- Proton hat am 2026-10-20 neue Ordner direkt in „Meine Dateien“ abgelehnt (HTTP 422, Ordnerlimit). Sync-Paare,
  Upload-Jobs und Laufwerke sollten deshalb auf einen bestehenden Proton-Ordner zeigen
  (Handbuch 4.3 und 9.2).
- Erledigt (ADR 0007): Abgleiche lesen den Remote-Baum nicht mehr vollständig, sondern holen nur
  die Proton-Ereignisse nach. Nach einem Neustart wird einmal vollständig gelesen.
- Proton meldet neue Dateien als `Updated`, nicht als `Created` (Contract-Test lässt beides zu).
- Meldungen des Daemons, Hilfetexte der CLI und Fehler, die nicht über die Übersetzungsschicht
  laufen, sind seit ADR 0014 nur auf Englisch; die übrigen Ausgaben der CLI sind zweisprachig.
- RUSTSEC-2023-0071 (`rsa`, Marvin-Angriff) in `deny.toml` begründet ignoriert: kein
  Netzwerk-Orakel, kein Patch verfügbar.
- RUSTSEC-2026-0370/0371/0372 (`zvariant`, `zvariant_derive`, `zbus_names`) ebenfalls ignoriert:
  nur „nicht mehr gepflegt“ (in zbus 6 aufgegangen); `secret-service` braucht noch zbus 5.
  Entfernen, sobald `keyring` auf zbus 6 umstellt.
- Fotos: Aufnahmezeit beim Hochladen ist die Änderungszeit der Datei (noch kein EXIF); Alben
  werden nicht lokal abgebildet; ohne Foto-Volume (Konto ohne Photos) ist kein Upload möglich,
  bis eine offizielle App es anlegt. Noch nicht mit einem echten Konto getestet.
- Tray-App: Fortschritt pro Sync-Paar, nicht pro Datei (dafür fehlen API-Ereignisse);
  Daemon-Einstellungen nur lesend (wie im Web); Meldungen des Daemons nur auf Englisch.
  Noch nicht auf einem echten KDE- bzw. GNOME-Desktop getestet, nur ohne Bildschirm und unter Xvfb.
- Laufwerke: Die Web-Oberfläche zeigt sie nicht an (sie ist für den Server gedacht, FUSE nur für den
  Desktop). Freigeben prüft vor dem Entfernen nur, ob die Datei in Proton noch existiert (lokal
  unveränderte Inhalte liegen dort sicher). Der Zwischenspeicher wird nicht verschlüsselt (er liegt
  im Datenverzeichnis des Benutzers mit Rechten 0700); Festplattenverschlüsselung empfohlen.
  Noch nicht mit einem echten Konto getestet.
- Benachrichtigungen (ADR 0017): noch nicht mit einem echten Mailserver erprobt; der Schalter je
  Thema für Desktop-Meldungen steht nur in `tray.toml`, noch nicht im Einstellungsfenster; die
  Zähler des Wochenberichts beginnen nach einem Neustart neu.
- Web-Oberfläche: Einstellungen nur lesend (Ändern bräuchte eine API zum Schreiben und Neuladen
  der Konfiguration); keine Proton-Anmeldung über das Web; Sitzungen nur im Speicher.
- Windows (ADR 0015, 0018): `daemon`, `client`, `cli` und die Tray-App bauen dort nicht
  (Unix-Socket); Platzhalter bräuchten die Cloud Files API. Nur portabel gehalten.

---

## English

**Project status**

This file is updated at the end of every session (see `CLAUDE.md`, section 8).

### Current phase
**Phase 6, file manager integration, packages, polish: done** (PR #10, #11 and #12). All phases
from `CLAUDE.md` are now implemented; next are the items from section 9 (see "Next"). Phase 5
(photos) was
completed with PR #9, phase 4 (tray app) with PR #8, phase 3
(placeholders) with PR #7.
Phase 2 was completed with PR #5 (upload jobs) and PR #6 (web interface).
Phase 1 was completed with PR #1, incremental sync with PR #3.
Since 2026-10-16 Claude continues on its own. Since 2026-10-20 only the PR review thread
merges (see `CLAUDE.md`, section 8); all other sessions only open PRs.

### Done

#### After phase 6, version archive on both sides (ADR 0020, 2026-10-29)
- New key `archive_versions = true` per `[[pair]]` (any direction): nothing is deleted or
  overwritten. Deletions and replaced versions move into `_archiv/` in the same folder, locally and
  in Proton (naming rule from ADR 0019); in Proton the node is moved. The archives are not synced,
  identical content is not archived, `protect_remote` takes precedence. Domain rule
  `conflict::archive_instead` with property tests, 21 scenario tests in
  `crates/application/tests/archive_versions.rs`. API values `archive_local`, `archive_remote`,
  `archive_remote_and_upload`, count `archived`; manual (de/en) 1, 4.3, 4.4, 6.4; example config.
- Open: no retention rule for the archives; resolving conflicts still uses the local trash.

#### After phase 6, safety tests of the sync modes (2026-10-29)
- Testkit: `RecordingRemote` and `RecordingPhotoLibrary` record every changing call to Proton
  (create, upload, rename, move, trash, restore, delete); downloads can fail on request.
- `crates/application/tests/read_only_modes.rs`: `download_only` and `backup_only` never change
  Proton (checked after every run: no changing call, remote tree unchanged). Scenarios: local
  edits, deletions, renames, conflicts, network errors in the middle of a run, expired login,
  restart, lost events or event gaps, dry run, conflict resolution, trash; plus property tests with
  random changes on both sides (`backup_only` also never loses a local version).
- `tests/mode_matrix.rs`: table with 54 cases (every direction, with and without `protect_remote`,
  every single change and the typical conflicts) with expected Proton calls and local result;
  property tests: `protect_remote` only ever adds to Proton.
- `tests/photo_modes.rs`: the same for photos (`download_only` changes nothing, `protect_remote`
  removes nothing).
- Found and fixed on the way (`download_only` changed Proton): edited locally + deleted in Proton
  uploaded the file again; a folder deleted in Proton with local changes was recreated and filled
  in Proton. Also, the download inside a locally deleted folder whose content changed in Proton
  failed on every run; the folder is now recreated locally. Manual (de/en) 1 and 6.3.

#### After phase 6, backup mode with version archive (ADR 0019, 2026-10-28)
- New direction `direction = "backup_only"` per `[[pair]]`: store everything from Proton locally,
  never upload, never delete. Before each new version the old local file moves to `_archiv/` in
  the same folder (`YYYY-MM-DD_HH-MM-SS_name`, UTC); files deleted in Proton stay locally.
  Domain rule `conflict::decide_backup` with property tests, naming rule `archive::archived_name`,
  scenario tests in `crates/application/tests/backup_only.rs`; refused in `[photos]`.
  API values `backup_only`, `archive_and_download`, `keep_local_copy`; manual (de/en) 1 and 4.3.

#### After phase 6, Windows re-evaluated (ADR 0018, 2026-10-23)
- The client's decision: only stay portable (option A). The recommended port without
  placeholders in six stages stays in the ADR as a plan in reserve; WinFsp is ruled out by its
  licence.
- Stage 0 done: `FsLocal::absolute` only allows normal path components (otherwise on Windows
  `..\x` or `C:x` would leave the sync folder), default paths without `/` in text, CI job
  `windows` (clippy for the portable crates, tests for domain, application, `adapter-fs`).

#### After phase 6, race when overwriting in the drive fixed (issue #35)
- Cause of the flaky FUSE test `large_files_are_read_and_overwritten_correctly`: the upload after
  closing was still running while the file was written again or replaced via a helper file.
  Result: a needless conflict copy, or the older content won (this also affected real users).
  Now a gate per file for uploads and renames (addendum to ADR 0010). Two scenario tests in
  `crates/application/tests/virtual_drive.rs`; locally the FUSE test failed in 140 of 200 runs
  before and passed 200 of 200 after.

#### After phase 6, protection for Proton (ADR 0016)
- Key `protect_remote` per `[[pair]]` and in `[photos]`: local deletions do not reach Proton; if a
  local file would replace an existing one in Proton, a conflict `local_modified_remote_protected`
  occurs (both versions are kept, `keep-local` is refused). Domain rule `conflict::protect_remote`
  with a property test, scenario tests in `crates/application/tests/protect_remote.rs`. Manual
  (de/en) 4.3, 4.8, 6.3.

#### After phase 6, notifications and weekly report (ADR 0017)
- `Notifier` port, crate `adapter-notify` (email over SMTP with `lettre`, webhook as JSON or text
  for ntfy), tests against fake servers on localhost.
- Daemon module `notify` and section `[notify]`: conflicts (batched), sync failures (once until
  recovery), job failures, expired login and weekly report (cron, default Monday 8:00), every topic
  switchable per channel; retries on temporary errors; secrets from a systemd credential, a file
  (0600) or an environment variable.
- Tray app: desktop notifications can be switched off per topic (`[notification_topics]` in
  `tray.toml`).
- Manual (de/en) 4.9 and 6.6, `config example`.

#### Phase 6, file manager integration (ADR 0013)
- **Drive:** `VirtualDrive::path_status` gives the state of a path (placeholder, partial, local,
  waiting for upload, pinned) from memory only, without asking Proton.
- **API/daemon:** request `PathStatus` for any number of paths in drives, sync pairs and the photo
  folder (`synced`, `pending`, `conflict` against the last sync); `StateRepo::base_entry`.
- **CLI:** `protonsync file-status PATH…`; `pin` and `unpin` take several paths.
- **File managers** (`packaging/filemanager/`): helper `protonsync-filemanager`, Dolphin service
  menu, Nautilus extension with emblems and menu (Python, talks to the socket directly, cache per
  folder); Python tests and `shellcheck` in CI.
- Manual (de/en) section 6.9.

#### Phase 6, polish
- Secret Service through `block_in_place`, so that D-Bus waits do not hold up other tasks.
- Upload jobs make up one missed time after the start (addendum to ADR 0008).
- Web interface: "Photos" page with sync and duplicates; moved to the trash only after a second
  confirmation.
- Tray app, overview: in dry-run mode the commands for a one-off sync
  (`protonsync sync [--pair PAIR] [--apply]`) and how to turn it off for good; new box
  "What can I do?" with common actions, more commands and links to the manual.
- Crypto dependencies updated (2026-10-20): `aes-gcm` 0.11, `argon2` 0.6, `getrandom` 0.4
  (replaces Dependabot PRs #21–#23). Existing secret files and password hashes stay valid
  (tests with data from the previous versions). `webpki-roots` 1.0.10 because of RUSTSEC-2026-0381.

#### Phase 6, packages and resources
- **Arch/CachyOS:** `packaging/arch/PKGBUILD` (`protonsync-git`) with daemon, CLI, tray app,
  systemd user and system service, `sysusers.d` user, application menu entry and file manager
  files; CI builds the package in `archlinux:base-devel`, installs it and checks its content.
- **License texts:** `packaging/licenses/third_party.py` collects the licenses of all linked
  libraries (`THIRD-PARTY-LICENSES.txt`) for the package and the container.
- **Resources:** about 13 MB RAM when idle; CI measures the container on every run. Manual 3.5
  (package) and 7.4 (measurements).

#### Phase 5, photos and duplicates (ADR 0012)
- **Domain:** `Photo`, month folders `YYYY/MM` with unique names, media file detection, duplicate
  detection (same content, or same name and capture second) with a suggestion which photo to
  keep and a check of the deletion selection (never a whole group); property tests.
- **`PhotoLibrary` port** with `FakePhotoLibrary` and contract tests; `Calendar` port for local
  time; `RemoteBin`, so the same trash use case also restores photos.
- **Use cases:** photo sync with the three-way rule (base under `photos:library`), linking instead
  of transferring twice, locally edited photos are reported instead of uploaded, local copies are
  only deleted when the photo is verifiably gone in Proton; in-memory index kept up to date by
  events.
- **Proton adapter:** `ProtonPhotos` (timeline, metadata in batches, related photos, events of the
  photo volume, download and upload, trash); live test read-only.
- **Daemon, API, CLI:** `[photos]` section, sync at startup, after local changes, after Proton
  events and in full once a day; `protonsync photos [status|sync|duplicates [trash]]` with a dry
  run; trash `photos:library`; the tray reports failed photo syncs.
- Manual (de/en) sections 4.8 and 6.8.

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

#### Phase 4, tray app (ADR 0011)
- **Crate `client`:** socket connection, event subscription (`EventSubscription`) and language,
  extracted from the CLI; the CLI and the tray app share it.
- **Crate `tray-model` (without Qt):** overall state and icon, tooltip, labelled view of sync
  pairs, drives and conflicts, notifications, login steps (2FA, mailbox password), texts de/en,
  the app's own settings (`tray.toml`) and autostart.
- **Crate `gui-qt` (`protonsync-tray`):** cxx-qt 0.10 with QML; tray icon with menu, status window
  (overview, conflicts, settings), login dialog, notifications; commands and events each in a
  background thread; reconnects after a daemon restart. `unsafe` only in the generated bridge
  modules.
- **CI:** Qt packages, build and `protonsync-tray --check` without a screen (QML errors fail CI);
  coverage without the Qt bridge.
- Manual (de/en) sections 3.4 and 6.6.

### Tests
- 598 tests without an account (as of 2026-10-29) (`cargo test`), 25 tests marked `#[ignore]` (live tests and real
  FUSE mounts: `cargo test -p protonsync-fuse --test mount -- --ignored`).
- Tray app without a screen: `QT_QPA_PLATFORM=offscreen cargo run -p protonsync-gui-qt -- --check`.
- File manager files: `python3 -m unittest discover -s packaging/filemanager/tests` (25 tests).
- Targeted: `cargo test -p protonsync-application` (sync scenarios), `-p protonsync-daemon`,
  `-p protonsync-cli` (snapshot tests with `insta`), `-p protonsync-web` (pages against a
  scripted daemon).
- Live tests against a real account (only in the test folder, default `protonsync-live-tests`):
  ```sh
  PROTONSYNC_LIVE_USERNAME=… PROTONSYNC_LIVE_PASSWORD=… [PROTONSYNC_LIVE_DATA_PASSWORD=…] \
  [PROTONSYNC_LIVE_TOTP=…] cargo test -p protonsync-adapter-proton --test live_contract \
    -- --ignored --test-threads=1
  ```
- Coverage in CI ≥ 80 % overall, domain ≥ 95 %, use cases ≥ 90 %
  (`scripts/coverage_gate.py`; measured on 2026-10-29: 85.6 % overall, domain 99.4 %,
  application layer 94.2 %, sync planning 98.4 %, sync execution 96.9 %).
- Safety of the modes: `read_only_modes`, `mode_matrix` and `photo_modes` in
  `crates/application/tests` use `RecordingRemote` to check that `download_only`, `backup_only` and
  `protect_remote` do not change Proton (scenarios and property tests).
- All CI checks locally: `scripts/check.sh` (with `--coverage` also coverage).
- Quality gates in CI (added 2026-10-20): `cargo doc` with `-D warnings`, `--locked`
  (Cargo.lock must match), a separate job for the minimum Rust version 1.96, Dependabot for crates
  and GitHub Actions (weekly).

### Trial with the real account (2026-10-20, first round)
On the client's CachyOS VM, sync pair `~/ProtonTest` ↔ `My files/04_Diverses/ProtonTest`,
`dry_run = true`.
- **Works:** login (with the real account) and uploading a file.
- **Proton limit:** Proton refuses new folders directly in "My files" with HTTP 422 (folder
  limit; also in the official app). Workaround: put the sync folder inside an existing subfolder.
  The example configuration now mentions this.
- **Fixed:** `protonsync sync` reported "not logged in" although the login was fine and only
  opening the Proton folder had failed. Now the sync names the real cause and retries the
  connection on every sync (also after a network outage at startup).
- **New:** `protonsync config example` shows every setting with its default, an explanation and
  the choices, commented in German and English; tests keep the file in step with the code.
- Still open from the checklist [`docs/erprobung.md`](erprobung.md): sync with `--apply`,
  conflicts, trash, FUSE drive, tray app on KDE/GNOME, Dolphin/Nautilus, Arch package.

### Next (after phase 6)
1. Trial with the real account on the client's computer (live tests, FUSE, tray app,
   Dolphin/Nautilus, Arch package); bugs found there come first.
   Checklist for it: [`docs/erprobung.md`](erprobung.md).
2. Done: code switched to English (PRs #13, #14, #15; ADR 0014).
3. Done: Windows re-evaluated (ADR 0018): only stay portable, stage 0 done, no Windows port
   planned.
4. Ready-made packages for third parties (2026-10-20): on a tag `vX.Y.Z`,
   `.github/workflows/release.yml` builds the Arch package `protonsync-bin`
   (`packaging/arch-bin/PKGBUILD`), static musl binaries and the image `ghcr.io/kafast/protonsync`
   and creates a GitHub release. Open: push the first tag (the owner's decision), then possibly an
   AUR package `protonsync-bin` from the attached `PKGBUILD`/`.SRCINFO`; check that the ghcr package
   is public.

### Open items / known issues
- When opening a sync pair, protonsync creates the Proton folder if it is missing (`ensure_folder`),
  also with `download_only` and `backup_only`. This is the only remaining change in Proton in these
  modes (harmless, but e.g. an empty folder after a typo in the path). Decision pending.
- **Only partly tested with a real account:** on 2026-10-20 login and upload worked on the
  client's CachyOS VM; the rest of the checklist [`docs/erprobung.md`](erprobung.md) is still
  open. Live tests and the server image do not run in the build environment (no account, package
  sources blocked). The image is built in CI.
- On 2026-10-20 Proton refused new folders directly in "My files" (HTTP 422, folder limit). Sync pairs, upload
  jobs and drives should therefore point to an existing Proton folder (manual 4.3 and 9.2).
- Done (ADR 0007): syncs no longer read the whole remote tree but only catch up on Proton
  events. After a restart the tree is read in full once.
- Proton reports new files as `Updated`, not as `Created` (the contract test accepts both).
- Daemon messages, CLI help texts and errors that do not go through the translation layer are
  English only since ADR 0014; the rest of the CLI output is bilingual.
- RUSTSEC-2023-0071 (`rsa`, Marvin attack) is ignored in `deny.toml` with a justification: no
  network oracle, no patch available.
- RUSTSEC-2026-0370/0371/0372 (`zvariant`, `zvariant_derive`, `zbus_names`) are ignored too:
  only "unmaintained" (merged into zbus 6); `secret-service` still needs zbus 5. Remove once
  `keyring` moves to zbus 6.
- Photos: the capture time on upload is the file's modification time (no EXIF yet); albums are
  not mirrored locally; without a photo volume (account without Photos) no upload is possible
  until an official app creates it. Not yet tested with a real account.
- Tray app: progress per sync pair, not per file (the API lacks the events); daemon settings
  read-only (as in the web interface); daemon messages English only. Not yet tested on a real
  KDE or GNOME desktop, only without a screen and under Xvfb.
- Drives: the web interface does not show them (it is meant for servers, FUSE is desktop only).
  Before removing, freeing only checks that the file still exists in Proton (unchanged local
  content is safely stored there). The cache is not encrypted (it lives in the user's data
  directory with permissions 0700); disk encryption is recommended. Not yet tested with a real
  account.
- Notifications (ADR 0017): not yet tried with a real mail server; the per-topic switch for desktop
  notifications is only in `tray.toml`, not yet in the settings window; the weekly report's
  counters start again after a restart.
- Web interface: settings are read-only (changing them would need an API for writing and
  reloading the configuration); no Proton login over the web; sessions in memory only.
- Windows (ADR 0015, 0018): `daemon`, `client`, `cli` and the tray app do not build there (Unix
  socket); placeholders would need the Cloud Files API. Only kept portable.

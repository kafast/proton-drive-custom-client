# Projektstand

Diese Datei wird am Ende jeder Sitzung aktualisiert (siehe `CLAUDE.md`, Abschnitt 8).

## Aktuelle Phase
**Phase 0 – Fundament: abgeschlossen** (2026-10-16). Vor Phase 1 Rückmeldung des Auftraggebers abwarten.

## Fertig
- Faktencheck der Beschreibung, Korrekturen in `CLAUDE.md` (Abschnitt 10).
- Cargo-Workspace (Edition 2024, Rust ≥ 1.96), Lint-Regeln (kein `unwrap`/`expect`/`panic` im
  Produktivcode, `unsafe` verboten), rustfmt, VS-Code-Empfehlungen und Debug-Konfiguration.
- `protonsync-domain`: Knoten und Namen (validiert), Fingerprint/Revision, Sync-Paare
  (Richtung, Speichermodus), Drei-Wege-Entscheidung `conflict::decide` mit Property-Tests,
  Konfliktkopie-Namen, Papierkorb mit Frist (Standard 30 Tage), Freigaberegel für Platzhalter.
- `protonsync-application`: Ports `RemoteStorage`, `Clock` (+ `SystemClock`), `SecretStore`
  (Geheimnisse werden im `Debug` verborgen).
- `protonsync-testkit`: `FakeRemote` (deterministisch, Fehlerinjektion), `FixedClock`,
  `InMemorySecretStore`, 12 Contract-Tests für `RemoteStorage` als Makro.
- CI: fmt, clippy, Tests, Abdeckung ≥ 80 % (`cargo-llvm-cov`), `cargo deny`, Secret-Scan (gitleaks).
- ADRs 0001–0003.

## Tests
`cargo test` (alle), gezielt z. B. `cargo test -p protonsync-domain`.

## Als Nächstes (Phase 1, in kleinen Sitzungen)
1. Ports `LocalStorage`, `StateRepo`, `Notifier` entwerfen (mit Contract-Tests).
2. `adapter-state`: SQLite-Zustand mit Journal.
3. `adapter-proton`: Login (SRP, TOTP, Mailbox-Passwort), Contract-Tests als `#[ignore]`-Live-Tests;
   dabei `RemoteStorage` auf blockweises Streaming umstellen (ADR 0002).
4. `adapter-fs` (inotify via `notify`), Sync-Engine (zwei Wege), Konfliktwarteschlange, Papierkorb.
5. Daemon + CLI, Server-Container.

## Offene Punkte / bekannte Probleme
- Domänentypen für Upload-Jobs (`Job`) und Photos folgen mit Phase 2 bzw. 5.
- Mailbox-Passwort: Die Proton-Crates brauchen es für Lese-/Schreibzugriffe. Klären, ob der
  abgeleitete Schlüssel sicher im `SecretStore` abgelegt wird, damit der Server ohne erneute
  Passworteingabe neu starten kann.
- Das Repository soll in `proton-drive-custom-client` umbenannt werden (Wunsch des Auftraggebers,
  in den GitHub-Einstellungen des Repos; danach `repository` in `Cargo.toml` anpassen).

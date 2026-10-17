# ADR 0014: Code auf Englisch, Nutzertexte zweisprachig

- Status: angenommen
- Datum: 2026-10-16

## Kontext
Bis Phase 6 waren Kommentare, Doc-Kommentare, Log-Meldungen und interne Fehlertexte deutsch,
die Bezeichner schon englisch. Der Auftraggeber hat am 2026-10-16 entschieden, den Code auf
Englisch umzustellen (CLAUDE.md Abschnitt 9). Was Nutzer in CLI, Tray-App, Web und
Dateimanager sehen, bleibt zweisprachig.

## Entscheidung
- **Englisch:** Bezeichner, Kommentare, Doc-Kommentare (auch die Hilfe von `clap`),
  Log-Meldungen, Fehlertexte aus `#[error(...)]` und interne Meldungen, Testdaten und
  Testmeldungen, `description` in den `Cargo.toml`-Dateien.
- **Zweisprachig (unverändert):** alle Texte über die Übersetzungsschicht (`lang.pick(de, en)`,
  die `TEXTS`-Tabelle der Tray-App, `Name[de]` in `.desktop`-Dateien), die Beispielkonfiguration
  von `protonsync config example` (je eine Fassung pro Sprache) und alle Dokumente für Menschen
  (README, Handbuch, ADRs, STATUS, Commits, PRs).
- **Bleibt deutsch, weil es gespeichert oder sichtbar ist:** der Name der Konfliktkopie
  `Datei (Konflikt …).ext` und der gespeicherte Ereignis-Cursor `keine-fotos`. Eine Änderung würde
  vorhandene Daten oder Gewohnheiten brechen; sie bekäme einen eigenen Schritt mit Migration.
- Die Umstellung lief Crate für Crate in drei PRs, ohne Logik zu ändern.

## Folgen
- Fehlermeldungen, die ohne Übersetzungsschicht bis zum Nutzer durchreichen (z. B. Fehler beim
  Verbinden zum Daemon oder beim Lesen der Schlüsseldatei), sind jetzt englisch statt deutsch.
  Wo das stört, kommen sie künftig über `lang.pick`.
- Neue Beiträge schreiben Code und Kommentare englisch; Clippy und Review achten darauf.

---

## English

# ADR 0014: Code in English, user texts bilingual

- Status: accepted
- Date: 2026-10-16

## Context
Until Phase 6, comments, doc comments, log messages and internal error texts were German, while
identifiers were already English. On 2026-10-16 the client decided to switch the code to English
(CLAUDE.md section 9). What users see in the CLI, tray app, web and file manager stays bilingual.

## Decision
- **English:** identifiers, comments, doc comments (including the `clap` help), log messages,
  error texts from `#[error(...)]` and internal messages, test data and test messages, the
  `description` in the `Cargo.toml` files.
- **Bilingual (unchanged):** every text through the translation layer (`lang.pick(de, en)`, the
  tray app's `TEXTS` table, `Name[de]` in `.desktop` files), the example configuration of
  `protonsync config example` (one version per language) and all documents for people (README,
  manual, ADRs, STATUS, commits, PRs).
- **Stays German because it is stored or visible:** the conflict copy name
  `file (Konflikt …).ext` and the stored event cursor `keine-fotos`. Changing them would break
  existing data or habits; that would be a separate step with a migration.
- The switch went crate by crate in three PRs, without logic changes.

## Consequences
- Error messages that reach users without the translation layer (for example errors connecting to
  the daemon or reading the key file) are now English instead of German. Where that is a problem,
  they will go through `lang.pick` in the future.
- New contributions write code and comments in English; Clippy and review keep an eye on it.

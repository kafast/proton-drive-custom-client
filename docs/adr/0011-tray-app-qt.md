# ADR 0011: Tray-App mit Qt 6/QML über cxx-qt, Logik in reinem Rust

- Status: angenommen; das Ändern wichtiger Daemon-Einstellungen regelt ADR 0021
- Datum: 2026-10-16

## Kontext
Phase 4 verlangt eine Qt-Tray-App (Abschnitt 3.6): Tray-Icon mit Status, Einstellungsfenster,
Konfliktdialog, Fortschritt, Benachrichtigungen, Deutsch und Englisch. Sie ist wie CLI und Web
ein gleichberechtigter Client der Daemon-API ohne eigene Geschäftslogik. Qt darf nur dynamisch
gelinkt werden (LGPL). Der Workspace verbietet `unsafe` (`unsafe_code = "forbid"`), die
Anbindung an C++ braucht aber zwangsläufig Übergänge, die Rust als `unsafe` markiert.

## Entscheidung
- **cxx-qt 0.10 (KDAB, MIT/Apache-2.0) mit QML.** Rust-Objekte erscheinen in QML als
  `QObject`s mit Eigenschaften, Signalen und aufrufbaren Methoden. Qt 6 (ab 6.4) kommt dynamisch
  aus dem System (`qt6-base`, `qt6-declarative`, `qt6-svg`). Die QML-Dateien werden beim Bauen
  mit `qmlcachegen` vorübersetzt und ins Programm eingebettet.
- **Drei Teile:**
  - `client`: Socket-Verbindung, Ereignis-Abonnement und Sprache, gemeinsam für CLI und Tray-App
    (aus der CLI herausgelöst).
  - `tray-model`: die gesamte Anzeigelogik **ohne Qt**: Gesamtzustand und Tooltip aus Status und
    Ereignissen, Benachrichtigungen, Anmeldeschritte, Konfliktliste, Texte in beiden Sprachen,
    eigene Einstellungen und Autostart. Vollständig mit Unit-Tests gegen einen geskripteten
    Daemon geprüft.
  - `gui-qt` (Programm `protonsync-tray`): nur Brücke und QML-Ansichten. Ein Hintergrund-Thread
    hält das Ereignis-Abonnement, ein zweiter schickt Anfragen; Ergebnisse gelangen über
    `cxx_qt::Threading` in den Qt-Thread. Die Oberfläche blockiert nie.
- **`unsafe` nur in der Brücke.** `gui-qt` setzt `unsafe_code = "deny"` und erlaubt es nur für
  die zwei Brückenmodule, deren Code cxx-qt/cxx erzeugen, sowie eine kleine C++-Datei, die eine
  `QApplication` anlegt. Alle anderen Crates bleiben bei `forbid`.
- **Tray über `Qt.labs.platform`.** Unter KDE spricht Qt das StatusNotifierItem-Protokoll über
  D-Bus; GNOME zeigt es mit der Erweiterung „AppIndicator and KStatusNotifierItem Support“.
  Ohne StatusNotifier-Dienst fällt Qt auf das klassische Tray (Qt Widgets) zurück; deshalb
  `QApplication` statt `QGuiApplication`.
- **Texte:** Katalog in `tray-model`; QML ruft `tray.text("schluessel")` (nicht `tr`, das gibt es in `QObject` schon). Ein Test stellt sicher, dass
  jeder in QML benutzte Schlüssel in beiden Sprachen existiert. Die Sprache folgt der Umgebung oder
  der eigenen Einstellung.
- **Einstellungen:** Die Tray-App hat eigene Einstellungen (Sprache, Benachrichtigungen,
  Autostart) in `~/.config/protonsync/tray.toml`. Die Daemon-Einstellungen zeigt sie nur an und
  öffnet die Konfigurationsdatei im Editor; Ändern bräuchte wie beim Web eine API zum Schreiben
  und Neuladen der Konfiguration.
- **Anmeldung in der Tray-App:** Benutzername, Passwort, 2FA-Code und Mailbox-Passwort laufen
  direkt über den lokalen Socket (Rechte 0600) zum Daemon, wie bei der CLI. Nichts davon wird
  gespeichert oder protokolliert; die Eingabefelder werden danach geleert.
- **Prüfung:** `tray-model` mit Unit-Tests; `gui-qt` wird in der CI gebaut und mit
  `protonsync-tray --check` ohne Bildschirm (`QT_QPA_PLATFORM=offscreen`) gestartet, das alle
  QML-Dateien lädt und bei Fehlern mit Exit-Code ungleich 0 endet.

## Folgen
- Zum Bauen sind die Qt-Entwicklerpakete nötig (unter Arch/CachyOS in `qt6-base` und
  `qt6-declarative` enthalten); der Server-Build (`-p protonsync-daemon`) braucht kein Qt.
- Der erste Bau dauert wegen der C++-Teile von cxx-qt einige Minuten länger.
- Fortschritt zeigt die App pro Sync-Paar (läuft, fertig mit Zählern, Fehler), nicht pro Datei;
  dafür bräuchte die API zusätzliche Ereignisse.
- Dateimanager-Integration (Dolphin/Nautilus) bleibt Phase 6.

---

## English

**ADR 0011: Tray app with Qt 6/QML via cxx-qt, logic in pure Rust**

- Status: accepted; changing important daemon settings is covered by ADR 0021
- Date: 2026-10-16

### Context
Phase 4 asks for a Qt tray app (section 3.6): tray icon with status, settings window, conflict
dialog, progress, notifications, German and English. Like the CLI and the web interface it is an
equal client of the daemon API without business logic of its own. Qt may only be linked
dynamically (LGPL). The workspace forbids `unsafe` (`unsafe_code = "forbid"`), but binding to C++
inevitably needs transitions that Rust marks as `unsafe`.

### Decision
- **cxx-qt 0.10 (KDAB, MIT/Apache-2.0) with QML.** Rust objects appear in QML as `QObject`s with
  properties, signals and invokable methods. Qt 6 (6.4 or later) is linked dynamically from the
  system (`qt6-base`, `qt6-declarative`, `qt6-svg`). The QML files are precompiled with
  `qmlcachegen` at build time and embedded into the program.
- **Three parts:**
  - `client`: socket connection, event subscription and language, shared by the CLI and the tray
    app (extracted from the CLI).
  - `tray-model`: all display logic **without Qt**: overall state and tooltip from status and
    events, notifications, login steps, conflict list, texts in both languages, its own settings
    and autostart. Fully covered by unit tests against a scripted daemon.
  - `gui-qt` (program `protonsync-tray`): only the bridge and the QML views. One background thread
    holds the event subscription, a second one sends requests; results reach the Qt thread through
    `cxx_qt::Threading`. The interface never blocks.
- **`unsafe` only in the bridge.** `gui-qt` sets `unsafe_code = "deny"` and allows it only for the
  two bridge modules whose code cxx-qt/cxx generate, plus one small C++ file that creates a
  `QApplication`. All other crates stay at `forbid`.
- **Tray via `Qt.labs.platform`.** On KDE, Qt speaks the StatusNotifierItem protocol over D-Bus;
  GNOME shows it with the "AppIndicator and KStatusNotifierItem Support" extension. Without a
  StatusNotifier service Qt falls back to the classic tray (Qt Widgets); hence `QApplication`
  instead of `QGuiApplication`.
- **Texts:** catalog in `tray-model`; QML calls `tray.text("key")` (not `tr`, which `QObject` already has). A test makes sure every key used
  in QML exists in both languages. The language follows the environment or the app's own setting.
- **Settings:** the tray app has its own settings (language, notifications, autostart) in
  `~/.config/protonsync/tray.toml`. It only displays the daemon settings and opens the
  configuration file in an editor; changing them would, as for the web interface, need an API for
  writing and reloading the configuration.
- **Login in the tray app:** username, password, 2FA code and mailbox password go directly over the
  local socket (permissions 0600) to the daemon, as with the CLI. None of it is stored or logged;
  the input fields are cleared afterwards.
- **Verification:** `tray-model` with unit tests; `gui-qt` is built in CI and started without a
  screen (`QT_QPA_PLATFORM=offscreen`) as `protonsync-tray --check`, which loads all QML files and
  exits with a non-zero code on errors.

### Consequences
- Building needs the Qt development packages (on Arch/CachyOS included in `qt6-base` and
  `qt6-declarative`); the server build (`-p protonsync-daemon`) needs no Qt.
- The first build takes a few minutes longer because of cxx-qt's C++ parts.
- The app shows progress per sync pair (running, finished with counters, error), not per file;
  that would need additional API events.
- File manager integration (Dolphin/Nautilus) stays in phase 6.

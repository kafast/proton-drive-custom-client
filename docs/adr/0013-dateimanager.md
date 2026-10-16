# ADR 0013: Dateimanager-Integration über Kontextmenüs und eine Nautilus-Erweiterung

- Status: angenommen
- Datum: 2026-10-16

## Kontext
Phase 6 verlangt (Abschnitt 3.6) eine Integration in Dolphin (KDE) und Nautilus (GNOME):
Statusanzeige an den Dateien und ein Kontextmenü „Immer lokal halten“ / „Speicher freigeben“,
„soweit machbar“. Die Geschäftslogik bleibt im Daemon; Oberflächen sind nur Clients der API
(Abschnitt 3.6, ADR 0005).

Was die Dateimanager anbieten:
- **Dolphin:** Kontextmenüs als *Service Menus* (`.desktop`-Dateien unter
  `~/.local/share/kio/servicemenus/`, KDE Frameworks 6), die ein Programm aufrufen. Symbole an
  Dateien (Overlays) gibt es nur über ein `KOverlayIconPlugin`, ein in C++ gegen KIO gebautes
  Plugin; dafür bräuchte das Projekt eine zweite C++-Bauumgebung.
- **Nautilus:** Erweiterungen in Python über `nautilus-python` (`MenuProvider` für Menüs,
  `InfoProvider` für Embleme an Dateien). Nautilus ruft sie für jede angezeigte Datei auf.

## Entscheidung
- **Neue API-Anfrage `PathStatus`** mit beliebig vielen absoluten Pfaden; die Antwort nennt je
  Pfad einen Zustand:
  - in einem Laufwerk: `online_only` (Platzhalter), `partial` (teilweise geladen), `local`
    (vollständig geladen), `pending_upload` (lokal geändert, noch nicht hochgeladen), `folder`;
    dazu `pinned` („Immer lokal halten“ gilt);
  - in einem Sync-Ordner: `synced` (seit dem letzten Abgleich unverändert), `pending`
    (neu oder geändert, der nächste Abgleich überträgt es), `conflict` (offener Konflikt);
  - sonst `outside`.
  Der Daemon beantwortet sie aus seinem Zustand ohne Proton zu fragen; ein unbekannter Pfad in
  einem Laufwerk gilt als `online_only`, damit das Anzeigen eines Ordners nie Netzverkehr auslöst.
- **CLI:** `protonsync file-status PFAD…` (mit `--json`) zeigt den Zustand; `pin` und `unpin`
  nehmen mehrere Pfade. Damit können Kontextmenüs alles über die CLI erledigen.
- **Dolphin:** ein Service Menu mit „Immer lokal halten“, „Nur bei Bedarf“ und
  „Speicher freigeben“, das die CLI aufruft und Fehler über `notify-send` meldet. **Keine
  Overlays** in dieser Phase: Ein C++-Plugin lohnt den zusätzlichen Bau- und Pflegeaufwand nicht;
  der Zustand steht in der Tray-App und über `protonsync file-status`.
- **Nautilus:** eine Python-Erweiterung mit denselben Menüpunkten und Emblemen. Sie spricht direkt
  mit dem Socket des Daemons (eine JSON-Zeile je Anfrage, ADR 0005), fragt je Ordner einmal für
  alle angezeigten Dateien und merkt sich die Antwort einige Sekunden. Ist der Daemon nicht
  erreichbar, zeigt sie nichts an und blockiert Nautilus nicht (kurze Zeitüberschreitung).
- Beide Dateien liegen unter `packaging/filemanager/` und werden vom PKGBUILD installiert; das
  Handbuch beschreibt die Installation von Hand.

## Folgen
- Die Integration bleibt klein und ohne neue Bauumgebung; Dolphin bekommt nur Menüs.
- Die Nautilus-Erweiterung ist ein zweiter Client des Protokolls außerhalb von Rust; ihre
  Nachrichten sind durch die Vertragstests der API abgedeckt (gleiche JSON-Form).
- Offene Punkte: Overlays in Dolphin (C++-Plugin), Zustand während eines laufenden Uploads.

## English

# ADR 0013: File manager integration through context menus and a Nautilus extension

- Status: accepted
- Date: 2026-10-16

## Context
Phase 6 (section 3.6) asks for integration with Dolphin (KDE) and Nautilus (GNOME): status shown
on files and a context menu "always keep local" / "free up space", "as far as feasible". The
business logic stays in the daemon; user interfaces are only API clients (section 3.6, ADR 0005).

What the file managers offer:
- **Dolphin:** context menus as *service menus* (`.desktop` files in
  `~/.local/share/kio/servicemenus/`, KDE Frameworks 6) that call a program. Icons on files
  (overlays) only exist through a `KOverlayIconPlugin`, a C++ plugin built against KIO; that would
  need a second C++ build environment for the project.
- **Nautilus:** extensions in Python through `nautilus-python` (`MenuProvider` for menus,
  `InfoProvider` for emblems on files). Nautilus calls them for every file it shows.

## Decision
- **A new API request `PathStatus`** with any number of absolute paths; the answer gives a state
  per path:
  - in a drive: `online_only` (placeholder), `partial` (partly downloaded), `local` (fully
    downloaded), `pending_upload` (changed locally, not uploaded yet), `folder`; plus `pinned`
    ("always keep local" applies);
  - in a sync folder: `synced` (unchanged since the last sync), `pending` (new or changed, the
    next sync transfers it), `conflict` (open conflict);
  - otherwise `outside`.
  The daemon answers from its own state without asking Proton; an unknown path in a drive counts
  as `online_only`, so that showing a folder never causes network traffic.
- **CLI:** `protonsync file-status PATH…` (with `--json`) shows the state; `pin` and `unpin`
  take several paths. Context menus can therefore do everything through the CLI.
- **Dolphin:** a service menu with "Always keep local", "On demand only" and "Free up space" that
  calls the CLI and reports errors through `notify-send`. **No overlays** in this phase: a C++
  plugin is not worth the extra build and maintenance effort; the state is shown in the tray app
  and through `protonsync file-status`.
- **Nautilus:** a Python extension with the same menu items and emblems. It talks to the daemon's
  socket directly (one JSON line per request, ADR 0005), asks once per folder for all files shown
  and remembers the answer for a few seconds. If the daemon is unreachable it shows nothing and
  does not block Nautilus (short timeout).
- Both files live in `packaging/filemanager/` and are installed by the PKGBUILD; the manual
  describes installing them by hand.

## Consequences
- The integration stays small and needs no new build environment; Dolphin only gets menus.
- The Nautilus extension is a second client of the protocol outside Rust; its messages are covered
  by the API contract tests (same JSON shape).
- Open items: overlays in Dolphin (C++ plugin), state while an upload is running.

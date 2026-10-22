# Architekturentscheidungen (ADR)

Jede wichtige Weichenstellung bekommt eine kurze Datei: Kontext, Entscheidung, Folgen.
Entscheidungen werden nicht nachträglich umgeschrieben, sondern durch ein neues ADR ersetzt.
Alle ADRs sind zweisprachig: zuerst Deutsch, darunter eine englische Übersetzung (Abschnitt „English“).

| Nr. | Titel | Status |
|---|---|---|
| [0001](0001-clean-architecture.md) | Clean Architecture mit Cargo-Workspace | angenommen |
| [0002](0002-proton-anbindung.md) | Proton-Anbindung über Community-Crates hinter einem Port | angenommen |
| [0003](0003-async-ports.md) | Asynchrone Ports ohne `async-trait` | angenommen |
| [0004](0004-kein-eigenes-sdk.md) | Vorerst kein eigenes Proton-SDK | angenommen |
| [0005](0005-daemon-api.md) | Daemon-API als zeilenweises JSON über einen Unix-Socket | angenommen |
| [0006](0006-geheimnisse.md) | Geheimnisse im Secret Service bzw. in einer AES-256-GCM-Datei | angenommen |
| [0007](0007-inkrementeller-abgleich.md) | Entfernter Baum im Zwischenspeicher, nachgeführt über Proton-Ereignisse | angenommen |
| [0008](0008-upload-jobs.md) | Upload-Jobs mit eigenem Cron-Parser, Ortszeit über `jiff` und vorsichtiger Aufbewahrung | angenommen |
| [0009](0009-web-oberflaeche.md) | Web-Oberfläche im Daemon-Prozess, serverseitig erzeugt, ohne JavaScript | angenommen |
| [0010](0010-platzhalter-fuse.md) | Platzhalter über einen FUSE-Mount mit Block-Zwischenspeicher | angenommen |
| [0011](0011-tray-app-qt.md) | Tray-App mit Qt 6/QML über cxx-qt, Logik in reinem Rust | angenommen |
| [0012](0012-fotos.md) | Fotos als eigener Abgleich mit Drei-Wege-Regel, Duplikate nur als Vorschlag | angenommen |
| [0013](0013-dateimanager.md) | Dateimanager-Integration über Kontextmenüs und eine Nautilus-Erweiterung | angenommen |
| [0014](0014-code-sprache.md) | Code auf Englisch, Nutzertexte zweisprachig | angenommen |
| [0015](0015-windows.md) | Windows vorerst zurückgestellt, Architektur bleibt offen | angenommen |
| [0016](0016-loeschschutz.md) | Löschschutz für Proton (`protect_remote`) | angenommen |

---

## English

**Architecture decision records (ADR)**

Every important design decision gets a short file: context, decision, consequences.
Decisions are not rewritten afterwards but superseded by a new ADR.
All ADRs are bilingual: German first, followed by an English translation (section "English").

| No. | Title | Status |
|---|---|---|
| [0001](0001-clean-architecture.md) | Clean Architecture with a Cargo workspace | accepted |
| [0002](0002-proton-anbindung.md) | Proton integration via community crates behind a port | accepted |
| [0003](0003-async-ports.md) | Asynchronous ports without `async-trait` | accepted |
| [0004](0004-kein-eigenes-sdk.md) | No custom Proton SDK for now | accepted |
| [0005](0005-daemon-api.md) | Daemon API as line-delimited JSON over a Unix socket | accepted |
| [0006](0006-geheimnisse.md) | Secrets in the Secret Service or in an AES-256-GCM file | accepted |
| [0007](0007-inkrementeller-abgleich.md) | Remote tree cached and kept current through Proton events | accepted |
| [0008](0008-upload-jobs.md) | Upload jobs with an own cron parser, local time via `jiff` and cautious retention | accepted |
| [0009](0009-web-oberflaeche.md) | Web interface in the daemon process, rendered on the server, without JavaScript | accepted |
| [0010](0010-platzhalter-fuse.md) | Placeholders through a FUSE mount with a block cache | accepted |
| [0011](0011-tray-app-qt.md) | Tray app with Qt 6/QML via cxx-qt, logic in pure Rust | accepted |
| [0012](0012-fotos.md) | Photos as a separate sync with the three-way rule, duplicates only as suggestions | accepted |
| [0013](0013-dateimanager.md) | File manager integration through context menus and a Nautilus extension | accepted |
| [0014](0014-code-sprache.md) | Code in English, user texts bilingual | accepted |
| [0015](0015-windows.md) | Windows deferred for now, architecture stays open | accepted |
| [0016](0016-loeschschutz.md) | Protection for Proton (`protect_remote`) | accepted |

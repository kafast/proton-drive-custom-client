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

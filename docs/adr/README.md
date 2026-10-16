# Architekturentscheidungen (ADR)

Jede wichtige Weichenstellung bekommt eine kurze Datei: Kontext, Entscheidung, Folgen.
Entscheidungen werden nicht nachträglich umgeschrieben, sondern durch ein neues ADR ersetzt.

| Nr. | Titel | Status |
|---|---|---|
| [0001](0001-clean-architecture.md) | Clean Architecture mit Cargo-Workspace | angenommen |
| [0002](0002-proton-anbindung.md) | Proton-Anbindung über Community-Crates hinter einem Port | angenommen |
| [0003](0003-async-ports.md) | Asynchrone Ports ohne `async-trait` | angenommen |
| [0004](0004-kein-eigenes-sdk.md) | Vorerst kein eigenes Proton-SDK | angenommen |
| [0005](0005-daemon-api.md) | Daemon-API als zeilenweises JSON über einen Unix-Socket | angenommen |
| [0006](0006-geheimnisse.md) | Geheimnisse im Secret Service bzw. in einer AES-256-GCM-Datei | angenommen |
| [0007](0007-inkrementeller-abgleich.md) | Entfernter Baum im Zwischenspeicher, nachgeführt über Proton-Ereignisse / Remote tree cached, kept current through events | angenommen / accepted |

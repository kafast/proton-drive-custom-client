# Prompt: Proton Drive Client für Linux (Arbeitstitel „protonsync“)

> Dieses Dokument ist die Arbeitsgrundlage für den Bau des Clients. Es beschreibt Ziel, Anforderungen,
> Architektur, Qualitätsregeln und Vorgehen. Bei Widersprüchen oder Lücken: nachfragen, nicht raten.

## 1. Ziel

Ein inoffizieller Proton-Drive-Client für Linux, der sich im Kern wie der Windows-Client verhält:
Der Nutzer wählt einen Ordner (oder Mountpunkt), der mit Proton Drive synchron gehalten wird.
Dateien können vollständig lokal liegen **oder** nur als Platzhalter (bei Bedarf geladen).
Der Client läuft in zwei Rollen mit demselben Kern:

- **Desktop** (CachyOS/Arch, KDE/GNOME, Wayland und X11): Daemon + Qt-Tray-App + CLI.
- **Server** (Container oder VM auf Proxmox, headless): Daemon + CLI + kleine Web-Oberfläche.
  Ziel: möglichst wenig Overhead; dient als Backup-Spiegel der Proton-Daten.

Nur **ein** Proton-Konto. GUI und Meldungen zweisprachig (Deutsch/Englisch).

## 2. Rahmenbedingungen

- Sprache: **Rust** (stable, Edition 2024, mindestens Rust 1.96, weil die Proton-Crates das voraussetzen). Der Auftraggeber kennt Rust nicht, ist aber erfahrener
  Java-Entwickler. Code deshalb bewusst einfach, gut benannt, mit Kommentaren dort, wo Rust-Eigenheiten
  (Ownership, Lifetimes, `async`) nicht offensichtlich sind. Keine unnötige Cleverness.
- Entwicklungsumgebung des Auftraggebers: VS Code mit rust-analyzer. Projekt muss dort ohne
  Sonderkonfiguration bau- und testbar sein (`cargo build`, `cargo test`). `.vscode/`-Empfehlungen
  (extensions.json, launch.json für CodeLLDB) mitliefern.
- Lizenz: **PolyForm Noncommercial 1.0.0** (private Nutzung erlaubt, kommerzielle nicht).
  Abhängigkeitslizenzen prüfen (`cargo-deny`), Qt nur dynamisch (LGPL).
- Proton-Anbindung: Das **offizielle** Proton-Drive-SDK ist [`ProtonDriveApps/sdk`](https://github.com/ProtonDriveApps/sdk)
  (MIT, C# und TypeScript, Status „Preview“, laut Proton „not yet ready for third-party production use“).
  Ein offizielles Rust-SDK gibt es nicht. Deshalb nutzt dieses Projekt die inoffiziellen Community-Crates
  `proton-sdk` und `proton-drive-rs` (Repo [`narrrl/proton-sdk-rs`](https://github.com/narrrl/proton-sdk-rs),
  MIT, reine Rust-Neuimplementierung des offiziellen SDK inkl. Photos, nicht von Proton unterstützt).
  Version beim Faktencheck: 0.9.1 (2026-10-15), die API ändert sich noch häufig, daher Version exakt pinnen.
  **Diese Crates dürfen nur in einem einzigen Adapter-Crate vorkommen** (siehe Architektur), damit ein
  Wechsel auf das offizielle SDK (z. B. über eine spätere native Schnittstelle) oder einen anderen
  Backend leicht möglich ist. Details und Risiken: Abschnitt 10 und ADR 0002.
- Proton-Regeln einhalten: nur offizielle Endpunkte; ehrliche Kennung als Drittanbieter-App
  über den HTTP-Header `x-pm-appversion` im offiziellen Schema
  `external-drive-{name}@{semver}[-{channel}][+{suffix}]`, hier also z. B.
  `external-drive-protonsync@0.1.0-alpha` (`{name}` nur Kleinbuchstaben und `_`, Kanal `stable|beta|alpha`).
  Sich niemals als offizieller Client ausgeben, keine Proton-Logos/-Markennamen im Produktnamen.
  Event-basierte Synchronisation statt häufigem Polling oder rekursivem Durchlaufen (Vorgabe von Proton).
  Die Software ist nicht von Proton unterstützt; das im README deutlich sagen.
- Arbeitsumgebung: Der Bau erfolgt bevorzugt mit Claude Code auf dem Rechner des Auftraggebers
  (CachyOS, FUSE verfügbar, echtes Proton-Konto vorhanden). Alles Testbare trotzdem gegen
  Fakes/Mock-Server testen, damit die Tests schnell, reproduzierbar und ohne Konto laufen.
  Live-Tests (echtes Konto, echter FUSE-Mount) als `#[ignore]`-Tests mit Umgebungsvariablen anlegen.
  Läuft die Arbeit stattdessen in einer Umgebung ohne Konto/FUSE, gilt nur der erste Teil.
- Schutz echter Daten: Live-Tests nur in einem eigens dafür angelegten Testordner in Proton und in
  einem temporären lokalen Verzeichnis. Zerstörerische Operationen (Löschen, Überschreiben,
  Aufbewahrungsregeln) haben einen **Dry-Run-Modus**, der beim ersten Einsatz Standard ist.
  Zugangsdaten, Tokens und Testkonten nie committen (`.gitignore`, Secret-Scan in der CI).

## 3. Funktionen

### 3.1 Authentifizierung
- Login mit Benutzername, Passwort, **2FA (TOTP)**; Proton-Zwei-Passwort-Modus (Mailbox-Passwort)
  unterstützen.
- Desktop: Tokens im Secret Service (GNOME Keyring/KWallet, Crate `keyring`).
- Server: einmaliger interaktiver Login per CLI (`protonsync login`), danach Session-Token verschlüsselt
  im Datenverzeichnis (Schlüssel aus systemd-Credentials oder Umgebungsvariable/Datei mit Rechten 0600).
  Keine Passwörter in Logs, Konfigdateien oder Prozesslisten.
- Token-Refresh automatisch; bei dauerhaftem Fehlschlag Nutzer benachrichtigen (GUI-Meldung / Log / Web).

### 3.2 Synchronisation
- Richtung pro Sync-Paar einstellbar: **zwei Wege (Standard)**, nur Download, nur Upload.
- **Alle** Daten werden berücksichtigt (kein selektiver Sync erforderlich), Ignore-Muster optional.
- Änderungserkennung lokal (inotify, Crate `notify`) und remote (Proton-Events), mit periodischem
  Abgleich als Sicherheitsnetz. Crash-sicher: Zustand in SQLite (Journal), Operationen idempotent.
- **Konflikte: nachfragen.** Desktop: Dialog (GUI) bzw. CLI. Server (kein Nutzer anwesend):
  Konflikt in eine Warteschlange legen, **beide Versionen behalten** (Konfliktkopie mit Zeitstempel),
  Entscheidung später per CLI/Web. Nichts wird bei Konflikten stillschweigend überschrieben.
- **Papierkorb:** Löschungen (lokal wie remote) landen zuerst in einem Papierkorb, der sich nach
  X Tagen leert (Standard 30, global einstellbar). Wiederherstellung per CLI/GUI/Web.
- Bandbreitenlimit, Cache-Größen-Limit, parallele Transfers einstellbar.

### 3.3 Platzhalter („nur bei Bedarf“) und lokale Kopien
Zwei getrennte Betriebsarten, die der Nutzer einstellen kann:
1. **Sync-Ordner (voll lokal):** normale Dateien auf der Platte, bidirektional synchron.
2. **FUSE-Mount (on-demand):** virtuelles Laufwerk; Dateien erscheinen sofort, Inhalt wird beim
   Öffnen geladen und lokal zwischengespeichert. (Nur Desktop; auf dem Server nicht nötig.)

Zusätzlich:
- Pro Ordner/Datei „**Immer lokal halten**“ (angeheftet).
- Automatische **Freigabe nach Zeit X** (global einstellbar): lokal geladene, nicht angeheftete Dateien,
  die X Tage nicht benutzt wurden, werden lokal entfernt und bleiben nur als Platzhalter erhalten.
  Vor dem Entfernen sicherstellen, dass die Remote-Version aktuell und vollständig ist.
- Standard: beides verfügbar, die Voreinstellung (alles lokal vs. nur bei Bedarf) ist einstellbar.

### 3.4 Fotos
- Anbindung an den **separaten Proton-Photos-Bereich** (enthält die Handy-Bilder).
- Synchronisation (Download und/oder Upload je nach Einstellung) in einen lokalen Fotoordner.
- **Duplikaterkennung als eigene, getrennte Funktion:** liefert nur eine Vorschlagsliste
  (z. B. per Hash und Metadaten), löscht niemals automatisch. Löschen nur nach ausdrücklicher
  Bestätigung und über den Papierkorb.

### 3.5 Upload-Jobs (allgemeine Backup-Funktion, nur Server-relevant, aber nicht darauf beschränkt)
Allgemein gültig, **nicht** Home-Assistant-spezifisch. Anwendungsfall: Home-Assistant-Backups
regelmäßig nach Proton kopieren; muss aber für jedes Quellverzeichnis funktionieren.
- Job-Definition: Quellordner, Dateimuster (Glob), Zielordner in Proton, Zeitplan (Cron-Ausdruck) oder
  „bei neuer Datei“, Aufbewahrung (letzte N Dateien und/oder maximales Alter), Verifikation nach Upload
  (Größe/Hash), Verhalten bei Fehlern (Wiederholung mit Backoff, Benachrichtigung).
- Quellordner kann ein gemounteter Netzwerkpfad (SMB/NFS) oder ein lokaler Pfad sein.
- Löschungen durch die Aufbewahrungsregel laufen über den Papierkorb (3.2).
- Konfiguration per Datei, CLI und Web-Oberfläche; Status und Verlauf der letzten Läufe einsehbar.

### 3.6 Bedienung
- **CLI** (`protonsync`): login/logout, status, sync, pin/unpin, conflicts, trash, photos, jobs, config.
  Maschinenlesbare Ausgabe (`--json`), sinnvolle Exit-Codes.
- **Qt-Tray-App** (Qt 6/QML): Tray-Icon mit Status, Einstellungsfenster, Konfliktdialog,
  Fortschritt, Benachrichtigungen. Dateimanager-Integration (Dolphin/Nautilus): Statusanzeige und
  Kontextmenü „Immer lokal halten“ / „Speicher freigeben“, soweit machbar (spätere Phase).
- **Web-Oberfläche** (Server, optional aktivierbar, standardmäßig aus): schlankes Statusboard
  (Sync, Konflikte, Papierkorb, Upload-Jobs, Einstellungen). Nur lokal/im Heimnetz erreichbar,
  Anmeldung erforderlich, TLS-fähig oder hinter Reverse Proxy.
- GUI, CLI und Web sind **gleichberechtigte Clients derselben Daemon-API** (siehe 4.3), keine
  eigene Geschäftslogik in den Oberflächen.

## 4. Architektur (Clean Architecture nach Robert C. Martin)

### 4.1 Schichten und Abhängigkeitsregel
Abhängigkeiten zeigen **nur nach innen**. Der Kern kennt weder Proton, noch das Dateisystem, noch Qt.

```
┌─────────────────────────────────────────────────────────────────┐
│ Frameworks & Treiber (außen)                                    │
│  Qt-GUI · CLI · Web (axum) · FUSE · SQLite · inotify · Keyring  │
│  ┌───────────────────────────────────────────────────────────┐  │
│  │ Interface-Adapter                                         │  │
│  │  ProtonRemoteAdapter · FsLocalAdapter · SqliteStateRepo   │  │
│  │  Daemon-API (Controller/Presenter, DTOs)                  │  │
│  │  ┌─────────────────────────────────────────────────────┐  │  │
│  │  │ Use Cases (Anwendungsschicht)                       │  │  │
│  │  │  SyncEngine · ResolveConflict · PinFile · Evict ·   │  │  │
│  │  │  TrashPolicy · PhotoSync · FindDuplicates · RunJob  │  │  │
│  │  │  Ports: RemoteStorage, LocalStorage, StateRepo,     │  │  │
│  │  │         SecretStore, Clock, Notifier                │  │  │
│  │  │  ┌───────────────────────────────────────────────┐  │  │  │
│  │  │  │ Domäne (Entities)                             │  │  │  │
│  │  │  │  Node, SyncPair, Conflict, TrashEntry, Job,   │  │  │  │
│  │  │  │  PinPolicy, EvictionPolicy, Version           │  │  │  │
│  │  │  └───────────────────────────────────────────────┘  │  │  │
│  │  └─────────────────────────────────────────────────────┘  │  │
│  └───────────────────────────────────────────────────────────┘  │
└─────────────────────────────────────────────────────────────────┘
```

### 4.2 Cargo-Workspace (Vorschlag)

```
protonsync/
├─ crates/
│  ├─ domain/          # Entities, Value Objects, Regeln (keine I/O-Abhängigkeiten)
│  ├─ application/     # Use Cases + Port-Traits (nur domain als Abhängigkeit)
│  ├─ adapter-proton/  # EINZIGES Crate, das proton-sdk/proton-drive-rs kennt
│  ├─ adapter-fs/      # lokales Dateisystem, inotify
│  ├─ adapter-state/   # SQLite-Zustand, Journal
│  ├─ adapter-secrets/ # Secret Service / verschlüsselte Token-Datei
│  ├─ fuse/            # FUSE-Dateisystem (nur Desktop)
│  ├─ daemon/          # Zusammenbau (Composition Root), Scheduler, API-Server
│  ├─ cli/             # `protonsync`
│  ├─ web/             # HTTP-Oberfläche (axum), nutzt die Daemon-API
│  └─ gui-qt/          # Qt/QML-Tray-App (cxx-qt), nutzt die Daemon-API
├─ docs/               # Architekturentscheidungen (ADR), Benutzerhandbuch
├─ packaging/          # PKGBUILD (Arch/CachyOS), Dockerfile (Server), systemd-Units
└─ .github/workflows/  # CI
```

### 4.3 Daemon-API
Der Daemon ist der einzige Ort mit Geschäftsablauf. GUI, CLI und Web sprechen mit ihm über eine
versionierte API (Unix-Domain-Socket lokal; optional HTTP im Server-Modus). Ereignisse
(Fortschritt, Konflikte, Fehler) per Stream/Subscription. Die API-Typen leben in einem gemeinsamen Crate
und werden bei Änderungen per Vertragstest geprüft.

### 4.4 Austauschbarkeit der Proton-Anbindung
Der Port `RemoteStorage` beschreibt abstrakt: Knoten auflisten, Metadaten lesen, Datei hoch-/herunterladen
(auch in Teilen), verschieben/umbenennen, in Papierkorb, Ereignisse abonnieren, Photos. Es gibt:
- `adapter-proton` (echte Implementierung),
- `FakeRemote` (in-memory, deterministisch, für alle Tests),
- gemeinsame **Contract-Tests**, die jede Implementierung bestehen muss.

## 5. Qualitätsregeln

- **Clean Code:** sprechende Namen, kleine Funktionen, eine Verantwortung pro Modul, kein toter Code,
  keine Magic Numbers, Fehler als `Result` mit eigenen Fehlertypen (`thiserror`), kein `unwrap()` im
  Produktivcode. Öffentliche API dokumentiert.
- **Prinzipien:** SOLID, Dependency Inversion über Traits, Komposition statt Vererbung.
- **Tests (TDD wo sinnvoll):**
  - Domäne und Use Cases: reine Unit-Tests gegen `FakeRemote`/`FakeLocal`, schnell und ohne I/O.
  - Adapter: Integrationstests (SQLite, Dateisystem in temporären Verzeichnissen, Mock-HTTP-Server).
  - Synchronisation: Szenariotests (parallele Änderungen, Konflikte, Abbruch mitten im Lauf,
    Netzwerkfehler, Löschen/Wiederherstellen, Eviction, Papierkorb-Ablauf, Upload-Job-Aufbewahrung).
  - Property-Based-Tests (`proptest`) für Konfliktlogik und Zustandsautomat.
  - API: Vertragstests; CLI: Snapshot-Tests der Ausgabe.
  - Live-Tests und FUSE-Tests als `#[ignore]` (lokal ausführbar).
  - **Zielwert: mindestens 80 % Abdeckung** (`cargo-llvm-cov`), Domäne und Use Cases deutlich höher.
- **CI (GitHub Actions):** `cargo fmt --check`, `cargo clippy -- -D warnings`, `cargo test`,
  Abdeckungsbericht, `cargo deny` (Lizenzen/Sicherheitslücken), Build der Release-Artefakte
  und des Server-Containers.
- **Sicherheit:** Zero-Knowledge-Prinzip von Proton nicht aufweichen; Geheimnisse nie loggen;
  Dateirechte restriktiv; Eingaben validieren; Abhängigkeiten minimal halten.
- **Dokumentation:** README, Architekturentscheidungen (ADR) für wichtige Weichenstellungen,
  Benutzerhandbuch (de/en), Hinweis „inoffiziell, nicht von Proton unterstützt“.
- **Sprache aller Texte (Vorgabe vom 2026-10-16):** Alles, was Menschen lesen, gibt es auf
  **Deutsch und Englisch**: README, `docs/STATUS.md`, ADRs, Handbuch, Commit-Messages, PR-Texte,
  Release-Notes. Commit-Betreff im Schema `typ(bereich): deutsch / english`, im Text erst der
  deutsche, dann der englische Absatz. Längere Dokumente: deutscher Teil, dann `## English`
  (oder eine eigene englische Datei, wie beim Handbuch `docs/manual.md`).

## 6. Verpackung und Betrieb

- Desktop: `PKGBUILD` für Arch/CachyOS, systemd-User-Service für den Daemon, Autostart der Tray-App.
- Server: statisch gelinktes Binary (musl), schlankes OCI-Image (läuft als Container oder in LXC/VM
  auf Proxmox), nicht als root, Konfiguration per Datei und Umgebungsvariablen, systemd-Unit.
  Ressourcenverbrauch dokumentieren (Leerlauf-RAM, Image-Größe).

## 7. Vorgehen (Phasen)

Pro Phase: zuerst Tests und Schnittstellen, dann Implementierung, am Ende kurzer Bericht
(was ist fertig, wie wird es getestet, was ist offen). Vor Beginn der nächsten Phase Rückmeldung abwarten.

1. **Phase 0 – Fundament:** Workspace, CI, Lint-Regeln, Domäne, Ports, `FakeRemote`, Contract-Tests, ADRs.
2. **Phase 1 – Kern (MVP):** Login mit 2FA, `adapter-proton`, SQLite-Zustand, Sync-Engine
   (zwei Wege), Konfliktwarteschlange, Papierkorb, Daemon + CLI. Server-Container.
3. **Phase 2 – Upload-Jobs und Web-Oberfläche.**
4. **Phase 3 – Platzhalter:** FUSE-Mount, „Immer lokal halten“, automatische Freigabe nach Zeit X.
5. **Phase 4 – Qt-Tray-App** inklusive Konfliktdialog und Zweisprachigkeit.
6. **Phase 5 – Fotos** und Duplikaterkennung (eigene Funktion).
7. **Phase 6 – Dateimanager-Integration, Feinschliff, Pakete, Doku.**

## 8. Arbeitsweise über mehrere Sitzungen (wegen Nutzungslimits)

Die Arbeit verteilt sich über viele Sitzungen und kann jederzeit unterbrochen werden. Deshalb liegt der
Projektstand **im Repository**, nicht im Chatverlauf:

- Diese Datei liegt als `CLAUDE.md` im Repository-Wurzelverzeichnis. Jede Sitzung beginnt damit,
  `CLAUDE.md` und `docs/STATUS.md` zu lesen.
- `docs/STATUS.md` enthält: aktuelle Phase, was fertig ist, was als Nächstes ansteht, bekannte Probleme,
  getroffene Entscheidungen (Verweis auf ADRs). Jede Sitzung endet mit einem Update dieser Datei.
- Kleine, abgeschlossene Schritte; nach jedem grünen Schritt ein Git-Commit mit aussagekräftiger Nachricht
  (Conventional Commits, zweisprachig wie in Abschnitt 5 beschrieben). Nie mit rotem Build aufhören; sonst als `WIP` kennzeichnen und in
  `STATUS.md` vermerken.
- **Merges nur durch den PR-Review-Thread (Vorgabe vom 2026-10-20):** Alle anderen Sitzungen/Threads
  öffnen Pull Requests, mergen sie aber nie selbst. Ein eigener Review-Thread prüft jeden offenen PR
  (auch Dependabot) gegen diese Spezifikation, `docs/STATUS.md`, die ADRs und die Konventionen
  (zweisprachige Commit-/PR-Texte, Code auf Englisch, kleine Commits, grüne CI) und mergt, kommentiert
  oder schließt ihn.
- Pro Sitzung eine klar umrissene Aufgabe (ein Use Case, ein Adapter, ein Teilbereich) statt ganzer Phasen.
- Verbrauch schonen: gezielte Tests (`cargo test -p <crate>`) statt immer der ganzen Suite, den vollen
  Lauf vor jedem Commit nur in der CI; keine großen Dateien oder Logs ausgeben; nicht ganze Dateien neu
  schreiben, wenn eine kleine Änderung reicht.
- Modellwahl (Empfehlung): Standardmodell für Routinearbeit, Tests und Adapter; ein stärkeres Modell
  gezielt für Architekturentscheidungen, Sync-Engine, Konfliktlogik und schwierige Fehlersuche.

## 9. Offene Punkte für die Umsetzung

- **Lauffähigkeit unter Windows (untergeordnet, nach Phase 6):** Linux bleibt Zielplattform, die
  Architektur soll Windows aber nicht verbauen. Plattformabhängiges gehört deshalb immer in Adapter
  hinter Ports, nie in Domäne, Use Cases oder Daemon-Ablauf.
  - Ohne Änderung portabel: Domäne, Use Cases, Sync-Engine, `adapter-proton`, `adapter-state` (SQLite), CLI.
  - Mit kleinen Anpassungen: `adapter-fs` (`notify` nutzt unter Windows `ReadDirectoryChangesW`),
    `adapter-secrets` (`keyring` nutzt die Windows-Anmeldeinformationsverwaltung), Daemon-API
    (Unix-Socket ab Windows 10, alternativ Named Pipe), Qt-Tray-App.
  - Eigener Adapter nötig: Platzhalter-Modus über die Windows Cloud Files API statt FUSE;
    Paketierung (MSI/winget) und Autostart statt PKGBUILD/systemd.
  - Pfade nie als Text mit `/` zusammensetzen, sondern mit `Path`/`PathBuf`; Windows-Namensregeln
    (verbotene Zeichen, reservierte Namen wie `CON`, Groß-/Kleinschreibung) im lokalen Adapter beachten.
  - Hinweis: Proton bietet für Windows bereits einen offiziellen Client; Nutzen vor Beginn neu bewerten.

- **Code nur auf Englisch (erledigt 2026-10-16, ADR 0014):** Bezeichner, Kommentare,
  Doc-Kommentare und interne Log-/Fehlertexte im Code sind englisch. Was Nutzer sehen (CLI, GUI,
  Web), bleibt zweisprachig über die Übersetzungsschicht (`i18n`). Neuer Code wird englisch
  geschrieben.
- **Qt-Anbindung (erledigt 2026-10-16, ADR 0011):** `cxx-qt` 0.10 mit QML, Logik in reinem Rust
  (`tray-model`).
- Prüfen, ob die Community-Crates für Photos, Events und Teil-Downloads (für FUSE) ausreichen.
  Stand Faktencheck (Quellcode 0.9.1 geprüft): Photos, Events (inkl. `EventManager`) und
  **Byte-Range-Downloads** (`download_range`, `open_revision` → `RevisionReader::read_at`) sind vorhanden;
  damit ist FUSE „bei Bedarf“ möglich. Dateien tragen `active_revision_id` und `content_sha1`
  (SHA-1 des Klartexts) als Änderungs- bzw. Inhaltsmerkmal.
- Wiederaufnahme ohne gespeichertes Passwort: `AccountClient::key_passphrases()` nach dem Login sichern,
  später `ProtonDriveClient::with_key_passphrases()`. Diese Passphrasen sind so schutzwürdig wie das
  Passwort (nur im `SecretStore`).
- Es gibt bereits einen inoffiziellen Linux-Client desselben Crate-Autors,
  [`narrrl/proton-drive-linux`](https://github.com/narrrl/proton-drive-linux) (MIT, v1.0 vom Juli 2026,
  FUSE, Sync-Ordner, GTK-App). Bei Bedarf als Referenz nutzen; Code nur unter Beachtung der MIT-Lizenz
  übernehmen.
- Proton baut einen offiziellen Linux-Sync-Client (angekündigt Juni 2026, Stand Oktober 2026 ohne
  Termin und ohne Beta). Bei Veröffentlichung Nutzen und Abgrenzung neu bewerten.
- Proton stellt das SDK auf ein neues Kryptografie-Modell um (Ziel Ende 2026/Anfang 2027). Danach
  funktionieren alte SDK-Versionen nicht mehr; die Community-Crates müssen dann nachziehen.
  Abhängigkeit beobachten, Update-Fähigkeit des Adapters einplanen.

## 10. Faktencheck (Stand 2026-10-16)

Geprüft vor Beginn der Umsetzung. Ergebnis: Die Beschreibung stimmte im Kern, wurde aber in diesen Punkten
korrigiert bzw. ergänzt (die Korrekturen sind oben bereits eingearbeitet):

| Punkt | Ergebnis |
|---|---|
| Offizielle Schnittstelle | Offiziell und aktuell ist das **Proton Drive SDK** `ProtonDriveApps/sdk` (MIT, C#/TypeScript, Preview, für Drittanbieter-Produktion noch nicht freigegeben). Module: Client (verfügbar), Sync und Search (angekündigt). |
| `proton-sdk` / `proton-drive-rs` | Existieren, sind aber **inoffiziell** (Community, `narrrl/proton-sdk-rs`, MIT). Neueste Version 0.9.1 vom 2026-10-15, Edition 2024, Rust ≥ 1.96, `tokio`/`reqwest`, rPGP. Unterstützt SRP-Login, TOTP-2FA, Drive-Operationen, Events, Photos (Album-Schreiben und Photo-Volume-Anlage noch nicht portiert), Byte-Range-Downloads, keine Moves zwischen Volumes. Lese-/Schreibzugriffe brauchen zusätzlich das Mailbox-Passwort bzw. den daraus abgeleiteten Schlüssel. Signaturprüfung meldet Fehler nur als Status. |
| App-Kennung | Offizielles Schema `external-drive-{name}@{semver}-{channel}+{suffix}` im Header `x-pm-appversion`; Proton kann einzelne Versionen sperren. Beschreibung entsprechend präzisiert. |
| Offizielle Linux-Software | Es gibt seit Juni 2026 die offizielle **Proton Drive CLI** (TypeScript, Teil des SDK-Repos, Version 0.8.x) für Upload/Download/Verwaltung, aber **ohne Synchronisation und ohne Mount**. Ein offizieller Linux-Sync-Client ist angekündigt, aber noch nicht erschienen. Das Projekt bleibt damit sinnvoll. |
| Lizenz | PolyForm Noncommercial 1.0.0 ist mit den MIT-lizenzierten Proton-Crates vereinbar (MIT-Hinweise mitliefern). |

Quellen: [Proton Drive SDK (GitHub)](https://github.com/ProtonDriveApps/sdk),
[Proton-Blog: Drive SDK Mai 2026](https://proton.me/blog/drive-sdk-may-2026),
[Proton-Blog: SDK Preview](https://proton.me/blog/proton-drive-sdk-preview),
[proton-sdk auf docs.rs](https://docs.rs/crate/proton-sdk/0.3.2), [crates.io: proton-drive-rs](https://crates.io/crates/proton-drive-rs),
[StorageNewsletter: Proton Drive CLI](https://www.storagenewsletter.com/2026/07/10/proton-releases-drive-cli-for-windows-macos-and-linux/),
[OMG! Ubuntu: Linux-Client angekündigt](https://www.omgubuntu.co.uk/2026/06/proton-drive-linux-client).

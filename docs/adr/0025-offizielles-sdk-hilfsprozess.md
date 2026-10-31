# ADR 0025: Offizielles Proton-SDK über einen Hilfsprozess

- Status: angenommen (Umsetzung in Schritten, siehe „Folgen“)
- Datum: 2026-10-31
- Ergänzt: ADR 0002, ADR 0004

## Kontext
Krischan hat gefragt, ob das offizielle Proton Drive SDK
([`ProtonDriveApps/sdk`](https://github.com/ProtonDriveApps/sdk)) schon jetzt über einen
TypeScript-Hilfsprozess angebunden werden kann. Geprüft am 2026-10-31 im Quellcode
(`@protontech/drive-sdk` 0.23.0 vom 2026-10-19, Proton-CLI 0.9.0):

- **Lizenz und Reife:** MIT. Status „Preview“; für private, nicht-kommerzielle Projekte
  ausdrücklich erlaubt, für kommerzielle Drittanbieter noch nicht freigegeben. Die Schnittstelle
  ändert sich noch, deshalb wird die Version exakt gepinnt.
- **Umfang:** Auflisten, Hoch- und Herunterladen (auch Byte-Bereiche über einen „seekable
  stream“, also FUSE-tauglich), Umbenennen, Verschieben, Papierkorb, endgültiges Löschen,
  Revisionen, Events, Freigaben und Fotos (inklusive Alben und Timeline-Upload). Alles, was der
  Port `RemoteStorage` braucht, ist vorhanden.
- **Anmeldung:** Das SDK selbst enthält keine Anmeldung. Protons Konto-Modul
  (`incubating/account`, nicht veröffentlicht) kann Passwort-Login per SRP, aber **ohne 2FA und
  ohne Zwei-Passwort-Modus**, und den **Browser-Login** über eine abgeleitete Sitzung („session
  fork“) mit der Client-ID `external-drive`, die Proton für Drittanbieter vorsieht. Dabei laufen
  Passwort, 2FA und Mailbox-Passwort auf Protons Anmeldeseite; das Programm erhält nur Tokens und
  den verschlüsselt übergebenen Schlüssel. Der Link kann auf einem anderen Gerät geöffnet werden,
  das passt auch zum Server ohne Bildschirm.
- **Kryptografie:** Läuft vollständig im SDK (OpenPGP über `@protontech/crypto`). Beim neuen
  Kryptografie-Modell (Ende 2026/Anfang 2027) zieht Proton das eigene SDK zuerst nach.
- **Laufzeit:** Protons CLI läuft mit Bun (`bun:sqlite`, `Bun.secrets`). Mit
  `bun build --compile` entsteht eine einzelne ausführbare Datei mit eingebetteter Laufzeit;
  weder Node noch Bun müssen installiert sein. Kosten: rund 105 MB pro Datei, im Leerlauf
  geschätzt 40–80 MB RAM (statt wenige MB beim reinen Rust-Programm).

## Entscheidung
1. Neues Crate `adapter-sdk-helper`: implementiert `RemoteStorage` (später auch `Account` und
   `PhotoLibrary`), indem es einen Hilfsprozess startet und mit ihm über **JSON-Zeilen auf
   stdin/stdout** spricht (Protokoll: `docs/sdk-helfer-protokoll.md`). Das Crate kennt kein
   Proton-Crate; die Regel aus ADR 0002 („Proton-Crates nur in `adapter-proton`“) bleibt gültig.
2. Der Hilfsprozess (`helper/`, TypeScript) nutzt das offizielle SDK und wird mit Bun zu einer
   Datei kompiliert. Er trägt die eigene App-Kennung (`external-drive-protonsync@…`).
3. Anmeldung mit diesem Backend nur über den Browser-Login (session fork).
4. **Auf Wunsch von Krischan (2026-10-31) wird das offizielle Backend der Standard**
   (`[proton] backend = "official"`), sobald Hilfsprozess und Browser-Login durchgängig
   funktionieren. Die Community-Crates bleiben wählbar (`backend = "community"`), etwa für
   schwache Server oder als Ausweg bei Problemen mit dem SDK.

## Folgen
- Umsetzung in Schritten: (1) Protokoll und Rust-Adapter, geprüft mit den Contract-Tests gegen
  einen Fake-Hilfsprozess; (2) TypeScript-Hilfsprozess mit dem SDK; (3) Browser-Login in API,
  CLI, Web und Tray, Backend-Auswahl im Daemon, Umstellung des Standards; (4) Fotos.
- Pakete werden größer (PKGBUILD und Container enthalten zusätzlich den Hilfsprozess).
- Dateiinhalte gehen als base64 in Stücken von 1 MiB über die Pipe (rund 33 % Mehraufwand),
  dafür liegen keine Klartext-Zwischendateien auf der Platte.
- Live-Tests mit echtem Konto kann nur Krischan ausführen.
- Der Hilfsprozess enthält MIT-lizenzierten Code von Proton; Pakete liefern den MIT-Hinweis mit.
- Auf dem Server speichert der Hilfsprozess die Sitzung vorerst wie Protons CLI als Datei mit
  Rechten 0600 (`unsafe_file`); eine Anbindung an protonsyncs verschlüsselten `SecretStore` folgt.

---

## English

**ADR 0025: Official Proton SDK through a helper process**

- Status: accepted (implemented in steps, see "Consequences")
- Date: 2026-10-31
- Extends: ADR 0002, ADR 0004

### Context
Krischan asked whether the official Proton Drive SDK
([`ProtonDriveApps/sdk`](https://github.com/ProtonDriveApps/sdk)) can already be connected
through a TypeScript helper process. Checked on 2026-10-31 in the source code
(`@protontech/drive-sdk` 0.23.0 of 2026-10-19, Proton CLI 0.9.0):

- **License and maturity:** MIT. Status "preview"; explicitly allowed for personal,
  non-commercial projects, not yet approved for commercial third parties. The interface still
  changes, so the version is pinned exactly.
- **Scope:** listing, upload and download (including byte ranges through a seekable stream, so
  usable for FUSE), rename, move, trash, permanent delete, revisions, events, sharing and photos
  (including albums and timeline upload). Everything the `RemoteStorage` port needs is there.
- **Sign-in:** the SDK itself has no sign-in. Proton's account module (`incubating/account`,
  unpublished) offers password login via SRP, but **without 2FA and without two-password mode**,
  and **browser login** through a session fork with the client id `external-drive`, which
  Proton provides for third parties. Password, 2FA and mailbox password are then entered on
  Proton's sign-in page; the program only receives tokens and the key, passed encrypted. The
  link can be opened on another device, which also suits a headless server.
- **Cryptography:** runs entirely inside the SDK (OpenPGP via `@protontech/crypto`). For the new
  cryptography model (end of 2026/early 2027) Proton updates its own SDK first.
- **Runtime:** Proton's CLI runs on Bun (`bun:sqlite`, `Bun.secrets`). `bun build --compile`
  produces a single executable with the runtime embedded; neither Node nor Bun has to be
  installed. Cost: about 105 MB per file and an estimated 40–80 MB RAM when idle (instead of a
  few MB for the pure Rust program).

### Decision
1. New crate `adapter-sdk-helper`: implements `RemoteStorage` (later also `Account` and
   `PhotoLibrary`) by starting a helper process and talking to it with **JSON lines over
   stdin/stdout** (protocol: `docs/sdk-helfer-protokoll.md`). The crate knows no Proton crate;
   the rule from ADR 0002 ("Proton crates only in `adapter-proton`") still holds.
2. The helper (`helper/`, TypeScript) uses the official SDK and is compiled into one file with
   Bun. It carries our own app identifier (`external-drive-protonsync@…`).
3. With this backend, sign-in is through browser login (session fork) only.
4. **At Krischan's request (2026-10-31) the official backend becomes the default**
   (`[proton] backend = "official"`) as soon as the helper and browser login work end to end.
   The community crates stay selectable (`backend = "community"`), e.g. for weak servers or as a
   way out of problems with the SDK.

### Consequences
- Implemented in steps: (1) protocol and Rust adapter, checked with the contract tests against
  a fake helper; (2) TypeScript helper with the SDK; (3) browser login in API, CLI, web and
  tray, backend selection in the daemon, switch of the default; (4) photos.
- Packages grow (PKGBUILD and container also contain the helper).
- File contents cross the pipe as base64 in 1 MiB pieces (about 33 % overhead); in exchange no
  plain-text temporary files land on disk.
- Only Krischan can run the live tests with a real account.
- The helper contains MIT-licensed code by Proton; packages ship the MIT notice.
- On the server the helper stores the session for now like Proton's CLI, as a file with mode
  0600 (`unsafe_file`); a connection to protonsync's encrypted `SecretStore` follows.

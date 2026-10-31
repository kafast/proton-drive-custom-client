# Protokoll des SDK-Hilfsprozesses

Beschreibt, wie protonsync mit dem Hilfsprozess für das offizielle Proton-SDK spricht (ADR 0025).
Maßgeblich im Code: `crates/adapter-sdk-helper/src/protocol.rs`. Version: **1**.

## Grundlagen
- protonsync startet den Hilfsprozess und schreibt Anfragen auf dessen **stdin**; Antworten
  kommen auf **stdout**. Jede Nachricht ist **eine Zeile JSON** (UTF-8, Ende `\n`).
- **stderr** ist für Logs des Hilfsprozesses frei; protonsync reicht es in das eigene Log durch.
  Auf stdout darf nichts anderes stehen (fremde Zeilen werden ignoriert).
- Anfrage: `{"id": 7, "method": "node", "params": {"node": "…"}}`.
- Erfolg: `{"id": 7, "ok": …}`, Fehler: `{"id": 7, "error": {"kind": "…", …}}`.
- Anfragen dürfen sich überlappen; Antworten dürfen in beliebiger Reihenfolge kommen.
- Endet stdin, beendet sich der Hilfsprozess.
- Dateiinhalte stehen als **base64** im Feld `data`, höchstens 1 MiB (vor base64) pro Nachricht.

## Knoten
```json
{"id": "…", "parent": "…", "name": "a.txt", "type": "file",
 "revision": "…", "size": 3, "modified": 1760000000, "sha1": "…"}
{"id": "…", "parent": "…", "name": "Ordner", "type": "folder"}
```
`parent` fehlt nur bei der Wurzel; `sha1` (SHA-1 des Klartexts, hex) ist optional,
`modified` in Unix-Sekunden.

## Fehlerarten (`kind`)
`not_found` (`node`), `already_exists` (`name`), `not_a_folder` (`node`), `not_a_file` (`node`),
`invalid_range` (`offset`, `size`), `invalid_move`, `unauthorized`, `transfer`, `rejected`,
`unavailable` (die vier letzten mit `message`). Sie entsprechen `RemoteError`.

## Methoden
| Methode | Parameter | Ergebnis |
|---|---|---|
| `hello` | `protocol` | `{"protocol": 1, "sdk_version": "…"}` |
| `node` | `node` | Knoten |
| `list_children` | `folder` | Liste von Knoten (ohne Papierkorb, Reihenfolge egal) |
| `create_folder` | `parent`, `name` | Knoten |
| `upload_open` | `parent`, `name`, `size`, `modified` | `{"upload": "…"}` |
| `revision_open` | `file`, `size`, `modified` | `{"upload": "…"}` |
| `upload_write` | `upload`, `data` | `null` |
| `upload_commit` | `upload` | Knoten; Fehler `transfer`, wenn die Größe nicht stimmt |
| `upload_abort` | `upload` | `null`; nichts bleibt zurück |
| `download_open` | `file` | `{"download": "…"}` |
| `download_read` | `download`, `max` | `{"data": "…", "eof": bool}` |
| `download_close` | `download` | `null` |
| `download_range` | `file`, `offset`, `length` | `{"data": "…"}`; am Dateiende kürzer, Start hinter dem Ende: `invalid_range` |
| `rename` | `node`, `name` | Knoten |
| `move` | `node`, `parent` | Knoten |
| `trash` | `node` | `null` |
| `restore` | `node` | Knoten |
| `delete` | `node` | `null`; nur für Knoten im Papierkorb |
| `events_since` | `root`, `cursor` (oder `null`) | `{"events": […], "cursor": "…"}`; Ereignisse des Volumes, in dem `root` liegt |
| `session` | – | `{"logged_in": bool, "user": "…"/null}` |
| `login_web_start` | – | `{"url": "…"}`: Link zur Anmeldung bei Proton (session fork) |
| `login_web_wait` | – | `{"done": bool, "user": …}`; wartet bis 30 s, bei `done: false` erneut fragen |
| `logout` | – | `null`; Sitzung und Caches gelöscht |
| `my_files_root` | – | Knoten „Meine Dateien“ |
| `shared_with_me` | – | Liste der mit dem Konto geteilten Knoten (ohne `parent`) |

Ereignisse: `{"type": "created"|"updated"|"trashed", "node": "…"}` oder `{"type": "gap"}`
(Lücke: alles neu einlesen). Mit `cursor: null` gibt es keine Ereignisse, nur den aktuellen Cursor.

Fotos folgen in einer späteren Protokollversion.

## Hilfsprozess bauen
`helper/build.sh` holt das gepinnte SDK (`helper/sdk-version.env`) nach `helper/vendor/sdk`,
installiert die Abhängigkeiten von Protons CLI, kopiert `helper/src/*.ts` daneben und baut mit
Protons eigenem Build-Skript eine Datei `helper/dist/protonsync-sdk-helper` (Bun eingebettet).
Die App-Kennung lautet dabei `external-drive-protonsync@<Version>-alpha`. Benötigt Bun in der
Version aus `sdk-version.env`. `bun test` in `helper/` prüft den Protokoll-Server ohne SDK.
Umgebungsvariablen zur Laufzeit: `PROTON_DRIVE_CACHE_DIR` (Datenordner),
`PROTON_DRIVE_CREDENTIALS_STORE` (`keychain` auf dem Desktop, `unsafe_file` auf dem Server).

## Prüfung
`cargo test -p protonsync-adapter-sdk-helper` lässt die Contract-Tests von `RemoteStorage`
gegen einen Fake-Hilfsprozess laufen (`tests/support/mod.rs`). Dieselben Tests prüfen später den
echten Hilfsprozess als Live-Tests.

---

## English

**Protocol of the SDK helper process**

Describes how protonsync talks to the helper process for the official Proton SDK (ADR 0025).
Authoritative in code: `crates/adapter-sdk-helper/src/protocol.rs`. Version: **1**.

### Basics
- protonsync starts the helper and writes requests to its **stdin**; answers come on **stdout**.
  Every message is **one line of JSON** (UTF-8, ending `\n`).
- **stderr** is free for the helper's logs; protonsync passes it into its own log. Nothing else
  may appear on stdout (foreign lines are ignored).
- Request: `{"id": 7, "method": "node", "params": {"node": "…"}}`.
- Success: `{"id": 7, "ok": …}`, failure: `{"id": 7, "error": {"kind": "…", …}}`.
- Requests may overlap; answers may come in any order.
- When stdin ends, the helper exits.
- File contents are **base64** in the field `data`, at most 1 MiB (before base64) per message.

### Nodes
See the JSON above: `parent` is missing only for the root; `sha1` (SHA-1 of the plain text, hex)
is optional, `modified` is in Unix seconds.

### Error kinds (`kind`)
`not_found` (`node`), `already_exists` (`name`), `not_a_folder` (`node`), `not_a_file` (`node`),
`invalid_range` (`offset`, `size`), `invalid_move`, `unauthorized`, `transfer`, `rejected`,
`unavailable` (the last four with `message`). They match `RemoteError`.

### Methods
The table above applies unchanged: `list_children` may return any order (the adapter sorts),
`upload_commit` fails with `transfer` if the size does not match, `upload_abort` leaves nothing
behind, `download_range` returns fewer bytes at the end of the file and `invalid_range` for a
start beyond the end, `delete` only works for nodes in the trash.

`events_since` also takes `root` and returns the events of the volume containing it.
Sign-in methods: `session`, `login_web_start` (returns the sign-in link, session fork),
`login_web_wait` (waits up to 30 s; ask again on `done: false`), `logout`; entry points
`my_files_root` and `shared_with_me`.

Events: `{"type": "created"|"updated"|"trashed", "node": "…"}` or `{"type": "gap"}` (gap: read
everything again). With `cursor: null` there are no events, only the current cursor.

Photos follow in a later protocol version.

### Building the helper
`helper/build.sh` fetches the pinned SDK (`helper/sdk-version.env`) into `helper/vendor/sdk`,
installs the dependencies of Proton's CLI, copies `helper/src/*.ts` next to it and builds one
file `helper/dist/protonsync-sdk-helper` (Bun embedded) with Proton's own build script. The app
identifier is `external-drive-protonsync@<version>-alpha`. Needs Bun in the version from
`sdk-version.env`. `bun test` in `helper/` checks the protocol server without the SDK.
Runtime environment: `PROTON_DRIVE_CACHE_DIR` (data folder), `PROTON_DRIVE_CREDENTIALS_STORE`
(`keychain` on the desktop, `unsafe_file` on the server).

### Testing
`cargo test -p protonsync-adapter-sdk-helper` runs the `RemoteStorage` contract tests against a
fake helper (`tests/support/mod.rs`). The same tests will check the real helper as live tests.

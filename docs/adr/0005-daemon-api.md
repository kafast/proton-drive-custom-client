# ADR 0005: Daemon-API als zeilenweises JSON über einen Unix-Socket

- Status: angenommen
- Datum: 2026-10-16

## Kontext
CLI, Web-Oberfläche und Qt-App sollen gleichberechtigte Clients desselben Daemons sein
(`CLAUDE.md`, 4.3). Die Schnittstelle muss lokal sicher sein, ohne Zusatzdienste laufen, von
Rust und Qt/C++ leicht anzusprechen sein und Ereignisse (Fortschritt, Konflikte) liefern.
Erwogen wurden gRPC, D-Bus und HTTP auf localhost.

## Entscheidung
- Transport: Unix-Domain-Socket (Rechte 0600, Verzeichnis 0700). Pfad aus `PROTONSYNC_SOCKET`,
  sonst `$XDG_RUNTIME_DIR/protonsync/daemon.sock`, sonst `/run/protonsync/daemon.sock`.
- Format: eine JSON-Nachricht pro Zeile (Höchstlänge 16 MiB). Jede Anfrage trägt
  `version` (`PROTOCOL_VERSION`, derzeit 1); bei Abweichung antwortet der Daemon mit
  `version_mismatch`.
- Anfrage/Antwort im Wechsel; nach `subscribe` bleibt die Verbindung offen und liefert Ereignisse.
- Die Typen liegen im Crate `protonsync-api`; Golden-JSON-Vertragstests schützen das Format.
- Passwörter laufen als eigener Typ `Password`, dessen `Debug` nichts verrät.

## Folgen
- Keine zusätzlichen Abhängigkeiten (kein Protobuf-Compiler, kein D-Bus-Broker), mit `socat`
  und `jq` von Hand testbar.
- Zugriffsschutz über Dateirechte: Nur der Besitzer des Daemons kann ihn bedienen.
- Die Web-Oberfläche (Phase 2) setzt als eigener Client auf dieselben Typen auf; HTTP nach außen
  bekommt dort eine eigene Anmeldung.
- Unter Windows ist ein Unix-Socket ab Windows 10 möglich, alternativ eine Named Pipe hinter
  derselben `Transport`-Abstraktion.

---

## English

**ADR 0005: Daemon API as line-delimited JSON over a Unix socket**

- Status: accepted
- Date: 2026-10-16

### Context
The CLI, web interface and Qt app should be equal clients of the same daemon
(`CLAUDE.md`, 4.3). The interface must be secure locally, run without additional services, be easy to use
from Rust and Qt/C++, and deliver events (progress, conflicts).
gRPC, D-Bus and HTTP on localhost were considered.

### Decision
- Transport: Unix domain socket (permissions 0600, directory 0700). Path from `PROTONSYNC_SOCKET`,
  otherwise `$XDG_RUNTIME_DIR/protonsync/daemon.sock`, otherwise `/run/protonsync/daemon.sock`.
- Format: one JSON message per line (maximum length 16 MiB). Every request carries
  `version` (`PROTOCOL_VERSION`, currently 1); on a mismatch the daemon responds with
  `version_mismatch`.
- Request and response alternate; after `subscribe` the connection stays open and delivers events.
- The types live in the crate `protonsync-api`; golden JSON contract tests protect the format.
- Passwords are passed as a dedicated type `Password`, whose `Debug` output reveals nothing.

### Consequences
- No additional dependencies (no Protobuf compiler, no D-Bus broker), testable by hand with `socat`
  and `jq`.
- Access control via file permissions: only the owner of the daemon can operate it.
- The web interface (phase 2) builds on the same types as a separate client; HTTP to the outside
  gets its own login there.
- On Windows a Unix socket is possible from Windows 10 onwards, alternatively a named pipe behind
  the same `Transport` abstraction.

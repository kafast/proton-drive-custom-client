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

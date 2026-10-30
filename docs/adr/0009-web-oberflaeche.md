# ADR 0009: Web-Oberfläche im Daemon-Prozess, serverseitig erzeugt, ohne JavaScript

- Status: angenommen; das Ändern wichtiger Daemon-Einstellungen regelt ADR 0021
- Datum: 2026-10-16

## Kontext
Phase 2 verlangt für den Server eine schlanke, optionale Web-Oberfläche (Abschnitt 3.6):
Sync, Konflikte, Papierkorb, Upload-Jobs, Einstellungen; standardmäßig aus, nur lokal/im
Heimnetz, Anmeldung erforderlich, TLS-fähig oder hinter einem Reverse Proxy. Sie soll wie CLI und
GUI ein gleichberechtigter Client der Daemon-API sein, ohne eigene Geschäftslogik, und auf dem
Server möglichst wenig Ressourcen brauchen.

## Entscheidung
- **Eigenes Crate `web`, aber im Daemon-Prozess.** Die Oberfläche kennt den Daemon nur über den
  Trait `Backend` (`handle(Request) -> Response`, dieselben API-Typen wie über den Socket). Der
  Daemon setzt ihn mit seinem Kern um. Ein eigener Prozess hätte ein zweites Programm, eine zweite
  Unit und einen Socket-Zugriff über Benutzergrenzen hinweg bedeutet, ohne Gewinn an Sicherheit:
  Wer die Oberfläche bedient, darf ohnehin alles, was die API kann.
- **axum 0.8 ohne Standard-Features, hyper direkt.** Nur HTTP/1.1, Formulare und Query. Die
  Annahme-Schleife ist selbst geschrieben, damit jede Verbindung ihre Absenderadresse mitbekommt
  (Netzfilter), Zeitlimits für Kopfzeilen und TLS-Handschlag gelten und das Beenden sauber ist.
- **HTML auf dem Server, kein JavaScript.** Jede Aktion ist ein Formular (POST, danach Umleitung).
  Die Übersicht lädt sich alle 30 s per `meta refresh` neu; kein Ereignisstrom (SSE) im Browser.
  Dadurch kann die Content-Security-Policy `default-src 'none'` sein.
- **Anmeldung:** ein Passwort nur für die Oberfläche, in der Konfiguration als Argon2id-Hash
  (`protonsyncd hash-password`). Sitzungen und CSRF-Kennungen zufällig (32 Bytes), nur im Speicher;
  Cookie `HttpOnly`, `SameSite=Strict`, mit TLS `Secure`. Nach 5 Fehlversuchen ist eine Adresse
  5 Minuten gesperrt.
- **Netzfilter vor allem anderen:** `allowed_networks` (Standard: private Netze) gilt für jede
  Anfrage, auch für die Anmeldeseite. `X-Forwarded-For` wird nicht ausgewertet; hinter einem Proxy
  filtert der Proxy.
- **TLS über rustls** (aws-lc-rs, wie `reqwest` ohnehin), PEM-Dateien aus der Konfiguration.
  Ohne TLS an einer Adresse außer Loopback warnt der Daemon beim Start.
- **Bewusst nicht enthalten:** Anmeldung bei Proton (bleibt `protonsync login`, damit Passwort und
  2FA-Code nicht über die Oberfläche laufen) und das Ändern der Konfiguration (Einstellungen nur
  lesend).

## Folgen
- Ein Programm, eine Unit, keine weitere Laufzeit; im Leerlauf kaum zusätzlicher Speicher.
- Fällt die Oberfläche beim Start aus (Port belegt, TLS-Dateien kaputt), läuft der Daemon weiter.
- Nach einem Neustart des Daemons ist man abgemeldet.
- Konfiguration über die Oberfläche bleibt ein offener Punkt; sie bräuchte eine API zum Schreiben
  der Konfiguration samt Neuladen.

---

## English

**ADR 0009: Web interface in the daemon process, rendered on the server, without JavaScript**

- Status: accepted; changing important daemon settings is covered by ADR 0021
- Date: 2026-10-16

### Context
Phase 2 asks for a lean, optional web interface for the server (section 3.6): sync, conflicts,
trash, upload jobs, settings; off by default, local/home network only, login required,
TLS-capable or behind a reverse proxy. Like the CLI and the GUI it should be an equal client of
the daemon API without business logic of its own, and need as few resources as possible.

### Decision
- **Own crate `web`, but inside the daemon process.** The interface knows the daemon only through
  the `Backend` trait (`handle(Request) -> Response`, the same API types as over the socket). The
  daemon implements it with its core. A separate process would have meant a second program, a
  second unit and socket access across user boundaries, without any security gain: whoever uses
  the interface may do everything the API can anyway.
- **axum 0.8 without default features, hyper directly.** HTTP/1.1, forms and query only. The
  accept loop is hand-written so every connection knows its peer address (network filter),
  timeouts apply to headers and the TLS handshake, and shutdown is clean.
- **HTML from the server, no JavaScript.** Every action is a form (POST, then a redirect). The
  overview reloads every 30 s via `meta refresh`; no event stream (SSE) in the browser. This lets
  the Content Security Policy be `default-src 'none'`.
- **Login:** a password just for the interface, stored in the configuration as an Argon2id hash
  (`protonsyncd hash-password`). Sessions and CSRF tokens are random (32 bytes) and in memory only;
  cookie `HttpOnly`, `SameSite=Strict`, `Secure` with TLS. After 5 failed attempts an address is
  locked for 5 minutes.
- **Network filter before anything else:** `allowed_networks` (default: private networks) applies
  to every request, including the login page. `X-Forwarded-For` is not evaluated; behind a proxy
  the proxy filters.
- **TLS via rustls** (aws-lc-rs, as `reqwest` uses anyway), PEM files from the configuration.
  Without TLS on any address other than loopback the daemon warns at startup.
- **Deliberately not included:** logging in to Proton (stays `protonsync login`, so the password
  and 2FA code never pass through the interface) and changing the configuration (settings are
  read-only).

### Consequences
- One program, one unit, no extra runtime; hardly any extra memory when idle.
- If the interface fails at startup (port in use, broken TLS files), the daemon keeps running.
- After a daemon restart you are logged out.
- Configuration through the interface remains an open item; it would need an API for writing the
  configuration plus reloading.

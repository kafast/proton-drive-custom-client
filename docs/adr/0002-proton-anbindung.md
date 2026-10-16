# ADR 0002: Proton-Anbindung über Community-Crates hinter einem Port

- Status: angenommen
- Datum: 2026-10-16

## Kontext
Faktencheck vom 2026-10-16 (Quellen in `CLAUDE.md`, Abschnitt 10):

- Das **offizielle** Proton Drive SDK ist [`ProtonDriveApps/sdk`](https://github.com/ProtonDriveApps/sdk):
  MIT-Lizenz, Implementierungen in C# und TypeScript, Status „Preview“, ausdrücklich noch
  **nicht** für Drittanbieter-Produktion freigegeben. Ein offizielles Rust-SDK gibt es nicht.
- Proton plant ein neues Kryptografie-Modell (Ziel Ende 2026/Anfang 2027); alte SDK-Versionen
  funktionieren danach nicht mehr.
- Die Community-Crates `proton-sdk` und `proton-drive-rs` ([`narrrl/proton-sdk-rs`](https://github.com/narrrl/proton-sdk-rs),
  MIT, inoffiziell) portieren das offizielle SDK nach Rust: SRP-Login, TOTP, Drive-Operationen,
  Events, Photos. Version 0.9.1 vom 2026-10-15, schnelle Releasefolge.
- Proton verlangt eine ehrliche App-Kennung im Header `x-pm-appversion` nach dem Schema
  `external-drive-{name}@{semver}-{channel}` und event-basierte Synchronisation.

## Entscheidung
1. Die Proton-Anbindung nutzt die Community-Crates, exakt gepinnt (`=0.9.x`), ausschließlich im
   Crate `adapter-proton`.
2. Der Rest des Systems kennt nur den Port `RemoteStorage` (Crate `protonsync-application`).
   Jede Implementierung muss die Contract-Tests in `protonsync-testkit` bestehen.
3. App-Kennung: `external-drive-protonsync@<CARGO_PKG_VERSION>-alpha`, Kanal wird mit der Reife
   angehoben.
4. Änderungserkennung remote über Proton-Events; vollständiges Durchlaufen nur als seltener
   Sicherheitsabgleich.

## Folgen
- Ein Wechsel auf das offizielle SDK (falls Proton z. B. eine native Schnittstelle anbietet) oder
  auf eine andere Bibliothek betrifft nur `adapter-proton`.
- Updates der Community-Crates (insbesondere zum neuen Kryptografie-Modell) müssen zeitnah
  nachgezogen werden; die Contract-Tests und Live-Tests (`#[ignore]`) sichern das ab.
- Der Port arbeitet in Phase 0 mit `Vec<u8>` für Dateiinhalte. Mit dem Adapter (Phase 1) wird auf
  blockweises Streaming umgestellt, weil Proton-Dateien in 4-MiB-Blöcken übertragen werden.
  `download_range` wird im Adapter über diese Blöcke umgesetzt.

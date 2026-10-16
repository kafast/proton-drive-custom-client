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
  Streaming umgestellt (`Read`/`Write`), weil Proton-Dateien in 4-MiB-Blöcken übertragen werden.
  `download_range` nutzt direkt `ProtonDriveClient::download_range` (Nachtrag nach Prüfung des
  Quellcodes von 0.9.1).

---

## English

**ADR 0002: Proton integration via community crates behind a port**

- Status: accepted
- Date: 2026-10-16

### Context
Fact check of 2026-10-16 (sources in `CLAUDE.md`, section 10):

- The **official** Proton Drive SDK is [`ProtonDriveApps/sdk`](https://github.com/ProtonDriveApps/sdk):
  MIT license, implementations in C# and TypeScript, status "Preview", explicitly
  **not** yet approved for third-party production use. There is no official Rust SDK.
- Proton plans a new cryptography model (target end of 2026/early 2027); old SDK versions
  will no longer work after that.
- The community crates `proton-sdk` and `proton-drive-rs` ([`narrrl/proton-sdk-rs`](https://github.com/narrrl/proton-sdk-rs),
  MIT, unofficial) port the official SDK to Rust: SRP login, TOTP, Drive operations,
  events, Photos. Version 0.9.1 of 2026-10-15, fast release cadence.
- Proton requires an honest app identifier in the `x-pm-appversion` header following the scheme
  `external-drive-{name}@{semver}-{channel}` and event-based synchronization.

### Decision
1. The Proton integration uses the community crates, pinned exactly (`=0.9.x`), only in the
   crate `adapter-proton`.
2. The rest of the system only knows the port `RemoteStorage` (crate `protonsync-application`).
   Every implementation must pass the contract tests in `protonsync-testkit`.
3. App identifier: `external-drive-protonsync@<CARGO_PKG_VERSION>-alpha`; the channel is raised as the
   project matures.
4. Remote change detection via Proton events; a full traversal only as a rare
   safety-net sync.

### Consequences
- Switching to the official SDK (for example if Proton offers a native interface) or
  to another library only affects `adapter-proton`.
- Updates of the community crates (especially for the new cryptography model) must be adopted
  promptly; the contract tests and live tests (`#[ignore]`) safeguard this.
- In phase 0 the port uses `Vec<u8>` for file contents. With the adapter (phase 1) it switches to
  streaming (`Read`/`Write`), because Proton files are transferred in 4 MiB blocks.
  `download_range` uses `ProtonDriveClient::download_range` directly (addendum after reviewing the
  source code of 0.9.1).

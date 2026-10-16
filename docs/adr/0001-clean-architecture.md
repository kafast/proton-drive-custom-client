# ADR 0001: Clean Architecture mit Cargo-Workspace

- Status: angenommen
- Datum: 2026-10-16

## Kontext
Der Client hat drei Oberflächen (Qt, CLI, Web), zwei Rollen (Desktop, Server) und eine
Proton-Anbindung, die sich wahrscheinlich ändert (siehe ADR 0002). Die Geschäftslogik
(Synchronisation, Konflikte, Papierkorb) muss ohne Proton-Konto und ohne Dateisystem testbar sein.

## Entscheidung
Clean Architecture nach Robert C. Martin, umgesetzt als Cargo-Workspace mit einem Crate pro Schicht
bzw. Adapter (siehe `CLAUDE.md`, Abschnitt 4.2). Die Abhängigkeitsregel wird durch die
`Cargo.toml`-Dateien erzwungen: `protonsync-domain` hat keine internen Abhängigkeiten,
`protonsync-application` hängt nur von der Domäne ab. Adapter implementieren die Port-Traits.
Test-Doubles und Contract-Tests liegen im Crate `protonsync-testkit`, das nur als
`dev-dependency` eingebunden wird.

## Folgen
- Ein Verstoß gegen die Abhängigkeitsregel ist ein Compilerfehler, kein Review-Thema.
- Mehr Crates bedeuten etwas mehr Boilerplate in den `Cargo.toml`-Dateien; gemeinsame Versionen
  und Lint-Regeln stehen deshalb zentral im Workspace.
- Crates werden erst angelegt, wenn ihre Phase beginnt (kein toter Code).

---

## English

**ADR 0001: Clean Architecture with a Cargo workspace**

- Status: accepted
- Date: 2026-10-16

### Context
The client has three user interfaces (Qt, CLI, web), two roles (desktop, server) and a
Proton integration that is likely to change (see ADR 0002). The business logic
(synchronization, conflicts, trash) must be testable without a Proton account and without a file system.

### Decision
Clean Architecture according to Robert C. Martin, implemented as a Cargo workspace with one crate per layer
or adapter (see `CLAUDE.md`, section 4.2). The dependency rule is enforced by the
`Cargo.toml` files: `protonsync-domain` has no internal dependencies,
`protonsync-application` depends only on the domain. Adapters implement the port traits.
Test doubles and contract tests live in the crate `protonsync-testkit`, which is only included as a
`dev-dependency`.

### Consequences
- A violation of the dependency rule is a compiler error, not a review topic.
- More crates mean a bit more boilerplate in the `Cargo.toml` files; shared versions
  and lint rules are therefore defined centrally in the workspace.
- Crates are only created when their phase begins (no dead code).

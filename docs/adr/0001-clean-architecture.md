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

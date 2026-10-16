# ADR 0003: Asynchrone Ports ohne `async-trait`

- Status: angenommen
- Datum: 2026-10-16

## Kontext
Netzwerk- und Dateizugriffe laufen asynchron (Tokio, wie in den Proton-Crates). Rust erlaubt
`async fn` in Traits seit Version 1.75, verlangt aber für öffentliche Traits eine Aussage darüber,
ob die Futures zwischen Threads wandern dürfen (`Send`).

## Entscheidung
Port-Methoden werden als `fn name(&self, ...) -> impl Future<Output = Result<..>> + Send`
deklariert. Implementierungen dürfen einfach `async fn` schreiben. Das Crate `async-trait` wird
nicht verwendet.

## Folgen
- Keine zusätzliche Abhängigkeit und keine versteckten Heap-Allokationen pro Aufruf.
- Diese Traits sind nicht „dyn-kompatibel“: Use Cases bekommen ihre Ports als generische
  Parameter (`SyncEngine<R: RemoteStorage>`), vergleichbar mit Generics in Java, statt als
  `Box<dyn RemoteStorage>`. Der Zusammenbau passiert einmal im Daemon (Composition Root).

---

## English

**ADR 0003: Asynchronous ports without `async-trait`**

- Status: accepted
- Date: 2026-10-16

### Context
Network and file access run asynchronously (Tokio, as in the Proton crates). Rust allows
`async fn` in traits since version 1.75, but for public traits it requires a statement about
whether the futures may move between threads (`Send`).

### Decision
Port methods are declared as `fn name(&self, ...) -> impl Future<Output = Result<..>> + Send`.
Implementations may simply write `async fn`. The crate `async-trait` is
not used.

### Consequences
- No additional dependency and no hidden heap allocations per call.
- These traits are not "dyn compatible": use cases receive their ports as generic
  parameters (`SyncEngine<R: RemoteStorage>`), comparable to generics in Java, instead of as
  `Box<dyn RemoteStorage>`. The wiring happens once in the daemon (composition root).

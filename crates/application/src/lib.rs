//! Anwendungsschicht von protonsync.
//!
//! Hier liegen die **Ports**: Traits (vergleichbar mit Java-Interfaces), über die
//! die Use Cases mit der Außenwelt sprechen. Implementiert werden sie in den
//! Adapter-Crates (z. B. `adapter-proton`) und für Tests im `testkit`.
//! Dieses Crate hängt nur von `protonsync-domain` ab.

pub mod ports;
pub mod sync;
pub mod use_cases;

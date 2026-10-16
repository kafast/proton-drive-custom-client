//! Contract-Tests: Verhalten, das jede Implementierung eines Ports zeigen muss.
//!
//! Pro Port gibt es ein Makro, das für jeden Contract-Test eine Testfunktion
//! erzeugt. So laufen dieselben Tests gegen Fakes und echte Adapter.

// Dies ist Test-Code: Ein fehlgeschlagenes `unwrap` soll den Test abbrechen.
#![allow(clippy::unwrap_used, clippy::missing_panics_doc)]

pub mod local;
pub mod remote;
pub mod state;

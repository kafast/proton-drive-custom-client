//! Optional Proton connection through a helper process (ADR 0025).
//!
//! Proton's official Drive SDK exists only in TypeScript and C#. This crate
//! starts a small helper program built on the TypeScript SDK and talks to it
//! with JSON lines over stdin/stdout ([`protocol`]). It knows **no** Proton
//! crate; the community crates stay in `adapter-proton` (ADR 0002), which
//! remains the default backend.
//!
//! - [`Connection`]: starts the helper and pairs requests with responses.
//! - [`HelperRemote`]: port `RemoteStorage` on a folder, served by the helper.
//! - [`HelperAccount`]: port `Account` with browser login (no photos yet).

mod account;
mod connection;
pub mod protocol;
mod remote;

pub use account::{HelperAccount, HelperConfig, NoPhotos, SessionStore};
pub use connection::{CALL_TIMEOUT, Connection};
pub use remote::HelperRemote;

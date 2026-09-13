//! Channels: payload-level access to the radios for ctrl clients.
//!
//! A client opens a channel with a profile; from then on the daemon frames,
//! codes, bundles and schedules that client's payloads, and hands it back the
//! payloads it decodes for the same channel. One engine thread per module runs
//! every channel bound to that module, so the daemon — the only process that
//! sees all traffic on a radio — is where airtime between channels is shared.

pub mod bundler;
pub mod engine;
pub mod profile;
pub mod table;

pub use engine::{spawn, Deliver};
pub use table::{SharedTable, Table};

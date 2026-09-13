//! Channels: a logical stream of payloads over one or more radio modules,
//! implemented by the radio daemon and driven over the ctrl protocol.
//!
//! [`spec`] holds the data that crosses the wire — ids, profiles, limits,
//! events. [`client`] holds the handles a program uses.

pub mod client;
pub mod spec;

pub use client::{Channel, ChannelBuilder, ChannelError, ChannelRx, ChannelTx, Received};
pub use spec::*;

#[cfg(feature = "machine-kaonic1s")]
#[path = "platform_kaonic1s.rs"]
mod platform_impl;

#[cfg(feature = "machine-kaonic1s")]
pub mod linux;

#[cfg(feature = "machine-kaonic1s")]
pub mod linux_rf215;

#[cfg(feature = "machine-host")]
#[path = "platform_dummy.rs"]
mod platform_impl;

pub use platform_impl::*;

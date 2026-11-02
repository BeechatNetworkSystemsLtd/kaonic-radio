#[cfg(any(feature = "machine-kaonic1s-us", feature = "machine-kaonic1s-ks"))]
#[path = "platform_kaonic1s.rs"]
mod platform_impl;

#[cfg(any(feature = "machine-kaonic1s-us", feature = "machine-kaonic1s-ks"))]
pub mod linux;

#[cfg(feature = "machine-kaonic1s-us")]
pub mod linux_rf215;

#[cfg(feature = "machine-host")]
#[path = "platform_dummy.rs"]
mod platform_impl;

pub use platform_impl::*;

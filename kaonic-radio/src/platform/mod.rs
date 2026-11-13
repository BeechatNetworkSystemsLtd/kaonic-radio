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

// The Kaonic boards' filter banks build without the hardware, so their tests run on the host too.
#[cfg(all(test, feature = "machine-host"))]
#[path = "kaonic1s/bank.rs"]
#[allow(dead_code)]
mod kaonic1s_bank;

pub use platform_impl::*;

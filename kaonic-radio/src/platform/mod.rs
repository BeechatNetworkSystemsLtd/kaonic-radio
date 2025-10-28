#[cfg(target_os = "linux")]
#[path = "platform_kaonic1s.rs"]
mod platform_impl;

#[cfg(target_os = "linux")]
pub mod linux;

#[cfg(target_os = "linux")]
pub mod linux_rf215;

pub use platform_impl::*;

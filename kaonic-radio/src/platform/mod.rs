#[cfg(target_os = "linux")]
#[path = "platform_kaonic1s.rs"]
mod platform_impl;

#[cfg(target_os = "linux")]
pub mod linux;

#[cfg(target_os = "linux")]
pub mod linux_rf215;

#[cfg(target_os = "macos")]
#[path = "platform_dummy.rs"]
mod platform_impl;

pub use platform_impl::*;

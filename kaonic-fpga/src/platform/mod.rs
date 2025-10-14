#[cfg(target_os = "linux")]
#[path = "platform_kaonic1s.rs"]
mod platform_impl;

pub use platform_impl::*;


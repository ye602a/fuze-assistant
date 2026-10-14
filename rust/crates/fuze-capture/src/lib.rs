//! Isolated native prototype. No desktop/process enumeration or foreign-window capture API.
#[cfg(windows)]
pub mod adapter;
#[cfg(windows)]
pub mod native;

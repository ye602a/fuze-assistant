//! Bounded native capture. Game attachment requires metadata-validated target discovery.
#[cfg(windows)]
pub mod adapter;
#[cfg(windows)]
pub mod native;
#[cfg(windows)]
pub mod pick_pixels;
#[cfg(windows)]
pub mod screen;
#[cfg(windows)]
mod selection_frame;
#[cfg(windows)]
pub mod target;

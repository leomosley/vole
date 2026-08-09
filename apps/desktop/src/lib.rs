pub mod config;

#[cfg(windows)]
pub mod audio;

#[cfg(windows)]
mod app;

#[cfg(windows)]
slint::include_modules!();

#[cfg(windows)]
pub use app::run;

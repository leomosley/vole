pub mod audio;
pub mod catalog;
pub mod config;

mod app;

slint::include_modules!();

pub use app::run;

pub mod model;
pub mod output;
pub mod tools;

#[cfg(windows)]
mod platform;

#[cfg(windows)]
pub use platform::collect;

#[cfg(windows)]
pub use platform::file_version;

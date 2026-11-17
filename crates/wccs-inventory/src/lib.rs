pub mod health;
pub mod model;
pub mod output;
pub mod tools;

#[cfg(windows)]
mod platform;

#[cfg(windows)]
pub use platform::collect;

#[cfg(windows)]
pub use platform::host_name;

#[cfg(windows)]
pub use platform::file_version;

#[cfg(windows)]
pub use platform::network;

#[cfg(windows)]
pub use platform::health::HealthCollector;

#[cfg(windows)]
pub use platform::volumes::collect as volumes;

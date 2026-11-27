#![forbid(unsafe_code)]

mod app;
mod auth;
mod contest_binding;
mod contest_control;
mod control;
mod database;
pub mod deployment;
mod device;
mod diagnostics;
mod distribution;
mod domjudge;
mod error;
mod events;
mod health;
mod image;
mod lifecycle;
mod maintenance;
mod management;
mod migrations;
mod operators;
mod preparation;
mod presentation;
mod recovery;
mod roster;
mod routes;
mod seating;
mod waiting;

pub use app::{App, AppState, Config, connect, listen_address};
pub use error::{ApiError, ApiResult};
pub use routes::router;

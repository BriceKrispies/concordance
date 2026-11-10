//! Application configuration and server lifecycle.

use std::env;
use std::fs;

/// Errors returned by [`load_config`].
#[derive(Debug)]
pub enum ConfigError {
    Missing,
    Invalid(String),
}

/// Settings read at startup.
#[derive(Debug)]
pub struct Config {
    pub mode: String,
    pub first_line: String,
}

/// A running server handle.
pub struct Server {
    started: bool,
}

impl Server {
    /// Marks the server as started.
    pub fn start(&mut self) {
        self.started = true;
    }
}

/// Loads configuration from `path`, using the `APP_MODE` environment variable.
pub fn load_config(path: &str) -> Result<Config, ConfigError> {
    let mode = env::var("APP_MODE").map_err(|_| ConfigError::Missing)?;
    if mode.is_empty() {
        return Err(ConfigError::Invalid(mode));
    }
    let body = fs::read_to_string(path).map_err(|e| ConfigError::Invalid(e.to_string()))?;
    let first_line = body.lines().next().unwrap().to_string();
    Ok(Config { mode, first_line })
}

/// Shuts the server down.
pub fn shutdown(handle: &mut Server) {
    if !handle.started {
        panic!("server not started");
    }
    handle.started = false;
}

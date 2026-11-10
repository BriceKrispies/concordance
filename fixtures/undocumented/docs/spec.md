# Application

## Overview

This application runs as a long-lived service. Operators configure it through a
settings file and an environment variable, and it is started and stopped by a
process supervisor.

## Startup

The application loads configuration at startup. The settings are held in a
`Config` value for the rest of the run.

## Example

```rust
use undocumented::{load_config, Server, shutdown};

let config = load_config("app.toml");
let mut server = Server::default();
```

## Operations

Operators read the logs to find out what happened during a run. The service
is intended to be run under a supervisor that restarts it after failures.

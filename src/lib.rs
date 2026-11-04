//! mediactl: a read-only, shadow-mode controller for a self-hosted media stack.
//!
//! Phase 1 never acts. Every module either parses data, makes a pure
//! decision, or performs a read-only query.

pub mod adapters;
pub mod config;
pub mod logging;
pub mod portsync;
pub mod procs;
pub mod qbt;
pub mod status;
pub mod tui;
pub mod watchdog;

//! Logging into the state folder (never the repo).

use anyhow::{Context, Result};
use std::fs::OpenOptions;
use std::io::Write;
use std::path::Path;
use std::sync::Mutex;

/// Log to `<state_dir>/mediactl.log`, and also to stderr when `tee` is set.
pub fn init(state_dir: &Path, tee: bool) -> Result<()> {
    std::fs::create_dir_all(state_dir).context("creating state folder")?;
    let file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(state_dir.join("mediactl.log"))
        .context("opening log file")?;
    tracing_subscriber::fmt()
        .with_ansi(false)
        .with_target(false)
        .with_max_level(tracing::Level::INFO)
        .with_writer(Mutex::new(Sink { file, tee }))
        .try_init()
        .ok();
    Ok(())
}

struct Sink {
    file: std::fs::File,
    tee: bool,
}

impl Write for Sink {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        if self.tee {
            std::io::stderr().write_all(buf).ok();
        }
        self.file.write_all(buf)?;
        Ok(buf.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        self.file.flush()
    }
}

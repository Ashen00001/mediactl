use anyhow::Result;
use clap::{Parser, Subcommand};
use mediactl::config::{real_env, Config};
use mediactl::{logging, portsync, status, tui};
use std::time::Duration;

/// Read-only, shadow-mode controller for a self-hosted media stack.
/// Phase 1 never changes anything.
#[derive(Parser)]
#[command(version, about)]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Show a snapshot of the stack.
    Status {
        /// Live view that refreshes every couple of seconds.
        #[arg(long, conflicts_with = "json")]
        watch: bool,
        /// Machine-readable output.
        #[arg(long)]
        json: bool,
    },
    /// Work out what the port sync would do, and log it.
    Portsync {
        /// Required. Phase 1 has no mode that acts.
        #[arg(long, required = true)]
        shadow: bool,
        /// Repeat every N seconds instead of running once.
        #[arg(long = "loop", value_name = "SECONDS")]
        interval: Option<u64>,
    },
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let cfg = Config::load(&real_env)?;
    match cli.cmd {
        Cmd::Status { watch, json } => {
            if watch {
                tui::run(&cfg)
            } else {
                let snap = status::gather(&cfg);
                if json {
                    println!("{}", serde_json::to_string_pretty(&snap)?);
                } else {
                    println!("{}", status::render_lines(&snap).join("\n"));
                }
                Ok(())
            }
        }
        Cmd::Portsync { interval, .. } => {
            logging::init(&cfg.state_dir, true)?;
            loop {
                let plan = portsync::decide(&portsync::observe(&cfg));
                tracing::info!("shadow portsync: {}", plan.summary());
                match interval {
                    Some(s) => std::thread::sleep(Duration::from_secs(s.max(1))),
                    None => return Ok(()),
                }
            }
        }
    }
}

//! Media stack process state (read-only, via `sysinfo`).

use serde::Serialize;
use sysinfo::{ProcessRefreshKind, ProcessesToUpdate, System, UpdateKind};

struct Target {
    label: &'static str,
    /// Process name prefix, lowercase, without `.exe`.
    exe: &'static str,
    /// Must appear in the command line when set.
    cmd_needle: Option<&'static str>,
}

const TARGETS: &[Target] = &[
    Target {
        label: "qbittorrent",
        exe: "qbittorrent",
        cmd_needle: None,
    },
    Target {
        label: "prowlarr",
        exe: "prowlarr",
        cmd_needle: None,
    },
    Target {
        label: "sonarr",
        exe: "sonarr",
        cmd_needle: None,
    },
    Target {
        label: "jellyfin",
        exe: "jellyfin",
        cmd_needle: None,
    },
    Target {
        label: "old portsync",
        exe: "python",
        cmd_needle: Some("portsync.py"),
    },
    Target {
        label: "auto-importer",
        exe: "powershell",
        cmd_needle: Some("autoimport.ps1"),
    },
    Target {
        label: "access hider",
        exe: "powershell",
        cmd_needle: Some("sloan-access.ps1"),
    },
];

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ProcState {
    pub label: String,
    pub running: bool,
}

fn norm(name: &str) -> String {
    let l = name.to_ascii_lowercase();
    l.strip_suffix(".exe").unwrap_or(&l).to_string()
}

/// Evaluate targets against (process name, command line) pairs. Pure.
pub fn evaluate(procs: &[(String, String)]) -> Vec<ProcState> {
    TARGETS
        .iter()
        .map(|t| ProcState {
            label: t.label.to_string(),
            running: procs.iter().any(|(name, cmd)| {
                norm(name).starts_with(t.exe) && t.cmd_needle.is_none_or(|n| cmd.contains(n))
            }),
        })
        .collect()
}

pub fn collect() -> Vec<ProcState> {
    let mut sys = System::new();
    sys.refresh_processes_specifics(
        ProcessesToUpdate::All,
        true,
        ProcessRefreshKind::nothing().with_cmd(UpdateKind::Always),
    );
    let list: Vec<(String, String)> = sys
        .processes()
        .values()
        .map(|p| {
            let cmd = p
                .cmd()
                .iter()
                .map(|s| s.to_string_lossy().to_string())
                .collect::<Vec<_>>()
                .join(" ");
            (p.name().to_string_lossy().to_string(), cmd)
        })
        .collect();
    evaluate(&list)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(n: &str, c: &str) -> (String, String) {
        (n.to_string(), c.to_string())
    }

    fn running(states: &[ProcState], label: &str) -> bool {
        states.iter().find(|s| s.label == label).unwrap().running
    }

    #[test]
    fn matches_names_case_insensitively() {
        let s = evaluate(&[p("qbittorrent.exe", ""), p("Sonarr.exe", "")]);
        assert!(running(&s, "qbittorrent"));
        assert!(running(&s, "sonarr"));
        assert!(!running(&s, "prowlarr"));
    }

    #[test]
    fn script_helpers_need_cmdline_match() {
        let s = evaluate(&[
            p("pythonw.exe", "pythonw.exe C:\\x\\portsync.py"),
            p("powershell.exe", "powershell -File autoimport.ps1"),
            p("powershell.exe", "powershell -File other.ps1"),
        ]);
        assert!(running(&s, "old portsync"));
        assert!(running(&s, "auto-importer"));
        assert!(!running(&s, "access hider"));
    }

    #[test]
    fn nothing_running() {
        assert!(evaluate(&[]).iter().all(|s| !s.running));
    }
}

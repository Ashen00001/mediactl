//! Status snapshot: gather read-only facts and render them.

use crate::adapters::{VpnMode, VpnPick};
use crate::config::Config;
use crate::portsync::{self, Plan};
use crate::procs::{self, ProcState};
use crate::qbt::Qbt;
use serde::Serialize;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct QbtStatus {
    pub listen_port: u16,
    pub download_bps: u64,
    pub upload_bps: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct Snapshot {
    pub taken_at_unix: u64,
    pub media: Vec<ProcState>,
    pub vpn: Option<VpnPick>,
    pub forwarded_port: Option<u16>,
    pub qbt: Option<QbtStatus>,
    pub qbt_error: Option<String>,
    pub jellyfin_running: bool,
    pub jellyfin_http: Option<bool>,
    pub portsync: Plan,
}

fn jellyfin_http_up(url: &str) -> bool {
    reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(3))
        .build()
        .and_then(|c| {
            c.get(format!("{}/System/Info/Public", url.trim_end_matches('/')))
                .send()
        })
        .map(|r| r.status().is_success())
        .unwrap_or(false)
}

pub fn gather(cfg: &Config) -> Snapshot {
    let media = procs::collect();
    let obs = portsync::observe(cfg);
    let plan = portsync::decide(&obs);
    let (qbt, qbt_error) = match &obs.qbt {
        Ok(view) => {
            let t = Qbt::new(cfg).and_then(|q| q.transfer());
            match t {
                Ok(t) => (
                    Some(QbtStatus {
                        listen_port: view.listen_port,
                        download_bps: t.dl_info_speed,
                        upload_bps: t.up_info_speed,
                    }),
                    None,
                ),
                Err(e) => (None, Some(e.to_string())),
            }
        }
        Err(e) => (None, Some(e.clone())),
    };
    let jellyfin_running = media.iter().any(|m| m.label == "jellyfin" && m.running);
    Snapshot {
        taken_at_unix: SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0),
        media,
        vpn: obs.vpn,
        forwarded_port: obs.forwarded,
        qbt,
        qbt_error,
        jellyfin_running,
        jellyfin_http: cfg.jellyfin_url.as_deref().map(jellyfin_http_up),
        portsync: plan,
    }
}

pub fn fmt_speed(bps: u64) -> String {
    const UNITS: [&str; 4] = ["B/s", "KiB/s", "MiB/s", "GiB/s"];
    let mut v = bps as f64;
    let mut i = 0;
    while v >= 1024.0 && i < UNITS.len() - 1 {
        v /= 1024.0;
        i += 1;
    }
    if i == 0 {
        format!("{bps} {}", UNITS[0])
    } else {
        format!("{v:.1} {}", UNITS[i])
    }
}

fn updown(b: bool) -> &'static str {
    if b {
        "up"
    } else {
        "down"
    }
}

pub fn render_lines(s: &Snapshot) -> Vec<String> {
    let mut out = vec!["Media stack".to_string()];
    for m in &s.media {
        out.push(format!(
            "  {:<14} {}",
            m.label,
            if m.running { "running" } else { "stopped" }
        ));
    }
    out.push(String::new());
    out.push("VPN".to_string());
    match &s.vpn {
        Some(v) => {
            let mode = match v.mode {
                VpnMode::App => "proton app",
                VpnMode::Wg => "standalone tunnel",
            };
            out.push(format!("  adapter        up ({}, {mode})", v.adapter));
        }
        None => out.push("  adapter        down (no proton adapter connected)".into()),
    }
    out.push(format!(
        "  forwarded port {}",
        s.forwarded_port
            .map(|p| p.to_string())
            .unwrap_or_else(|| "unknown".into())
    ));
    out.push(String::new());
    out.push("qBittorrent".to_string());
    match (&s.qbt, &s.qbt_error) {
        (Some(q), _) => {
            out.push(format!("  listen port    {}", q.listen_port));
            out.push(format!(
                "  speed          down {} / up {}",
                fmt_speed(q.download_bps),
                fmt_speed(q.upload_bps)
            ));
        }
        (None, Some(e)) => out.push(format!("  unavailable: {e}")),
        (None, None) => out.push("  unavailable".into()),
    }
    out.push(String::new());
    out.push("Jellyfin".to_string());
    out.push(format!(
        "  process        {}",
        if s.jellyfin_running {
            "running"
        } else {
            "stopped"
        }
    ));
    match s.jellyfin_http {
        Some(b) => out.push(format!("  http           {}", updown(b))),
        None => out.push("  http           not configured".into()),
    }
    out.push(String::new());
    out.push("Shadow decision (nothing is changed)".to_string());
    out.push(format!("  portsync       {}", s.portsync.summary()));
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapters::{AdapterKind, ProtonProtocol};
    use crate::portsync::Decision;

    fn snap() -> Snapshot {
        Snapshot {
            taken_at_unix: 0,
            media: vec![
                ProcState {
                    label: "qbittorrent".into(),
                    running: true,
                },
                ProcState {
                    label: "sonarr".into(),
                    running: false,
                },
                ProcState {
                    label: "jellyfin".into(),
                    running: true,
                },
            ],
            vpn: Some(VpnPick {
                adapter: "ProtonVPN".into(),
                mode: VpnMode::App,
                kind: AdapterKind::ProtonApp(ProtonProtocol::WireGuard),
            }),
            forwarded_port: Some(40001),
            qbt: Some(QbtStatus {
                listen_port: 40001,
                download_bps: 2_621_440,
                upload_bps: 512,
            }),
            qbt_error: None,
            jellyfin_running: true,
            jellyfin_http: Some(true),
            portsync: Plan {
                decision: Decision::InSync,
                rebind_interface: false,
                disable_upnp: false,
                warnings: vec![],
            },
        }
    }

    #[test]
    fn speed_formatting() {
        assert_eq!(fmt_speed(0), "0 B/s");
        assert_eq!(fmt_speed(1023), "1023 B/s");
        assert_eq!(fmt_speed(1536), "1.5 KiB/s");
        assert_eq!(fmt_speed(2_621_440), "2.5 MiB/s");
    }

    #[test]
    fn renders_full_snapshot() {
        let text = render_lines(&snap()).join("\n");
        assert!(text.contains("qbittorrent    running"));
        assert!(text.contains("sonarr         stopped"));
        assert!(text.contains("adapter        up (ProtonVPN, proton app)"));
        assert!(text.contains("forwarded port 40001"));
        assert!(text.contains("down 2.5 MiB/s / up 512 B/s"));
        assert!(text.contains("portsync       in sync"));
    }

    #[test]
    fn renders_degraded_snapshot() {
        let mut s = snap();
        s.vpn = None;
        s.forwarded_port = None;
        s.qbt = None;
        s.qbt_error = Some("qBittorrent unreachable".into());
        s.jellyfin_http = None;
        s.portsync.decision = Decision::Unknown("no vpn adapter connected".into());
        let text = render_lines(&s).join("\n");
        assert!(text.contains("down (no proton adapter connected)"));
        assert!(text.contains("forwarded port unknown"));
        assert!(text.contains("unavailable: qBittorrent unreachable"));
        assert!(text.contains("not configured"));
        assert!(text.contains("unknown: no vpn adapter connected"));
    }

    #[test]
    fn json_has_expected_shape() {
        let v = serde_json::to_value(snap()).unwrap();
        assert_eq!(v["forwarded_port"], 40001);
        assert_eq!(v["portsync"]["decision"]["decision"], "in_sync");
        assert_eq!(v["vpn"]["mode"], "app");
    }
}

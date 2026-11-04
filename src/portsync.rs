//! Shadow-mode port sync: work out what the old portsync would do, never do it.

use crate::adapters::{self, VpnMode, VpnPick};
use crate::config::Config;
use crate::qbt::{iface_id_for, Iface, Qbt};
use serde::Serialize;
use std::path::Path;
use std::time::SystemTime;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct QbtView {
    pub listen_port: u16,
    pub bound_interface: String,
    pub upnp: bool,
    #[serde(skip)]
    pub interfaces: Vec<Iface>,
}

/// Everything the decision needs, gathered read-only.
#[derive(Debug, Clone)]
pub struct Observation {
    pub vpn: Option<VpnPick>,
    pub forwarded: Option<u16>,
    /// Why `forwarded` is missing, when it is.
    pub forwarded_note: Option<String>,
    pub reserved: bool,
    pub qbt: Result<QbtView, String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "decision", content = "detail", rename_all = "snake_case")]
pub enum Decision {
    InSync,
    /// The old script would write qBittorrent settings. The port is included
    /// even when only the interface or UPnP differ (it then re-binds).
    WouldSet(u16),
    Unknown(String),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Plan {
    pub decision: Decision,
    /// Other changes the old script would make in the same pass.
    pub rebind_interface: bool,
    pub disable_upnp: bool,
    pub warnings: Vec<String>,
}

impl Plan {
    fn unknown(reason: impl Into<String>) -> Plan {
        Plan {
            decision: Decision::Unknown(reason.into()),
            rebind_interface: false,
            disable_upnp: false,
            warnings: vec![],
        }
    }

    pub fn summary(&self) -> String {
        let mut s = match &self.decision {
            Decision::InSync => "in sync".to_string(),
            Decision::WouldSet(p) => format!("would set port {p}"),
            Decision::Unknown(r) => format!("unknown: {r}"),
        };
        if self.rebind_interface {
            s.push_str(", would rebind interface");
        }
        if self.disable_upnp {
            s.push_str(", would disable upnp");
        }
        for w in &self.warnings {
            s.push_str(&format!(" [warning: {w}]"));
        }
        s
    }
}

/// The decision table. Pure.
pub fn decide(obs: &Observation) -> Plan {
    let Some(vpn) = &obs.vpn else {
        return Plan::unknown("no vpn adapter connected");
    };
    let q = match &obs.qbt {
        Ok(q) => q,
        Err(e) => return Plan::unknown(format!("qbittorrent unavailable: {e}")),
    };
    let Some(port) = obs.forwarded else {
        let why = obs
            .forwarded_note
            .clone()
            .unwrap_or_else(|| "no forwarded port found".into());
        return Plan::unknown(why);
    };
    let Some(want_iface) = iface_id_for(&q.interfaces, &vpn.adapter) else {
        return Plan::unknown("qbittorrent cannot see the vpn adapter");
    };
    let rebind_interface = q.bound_interface != want_iface;
    let disable_upnp = q.upnp;
    let mut warnings = vec![];
    if obs.reserved {
        warnings.push("forwarded port is reserved by windows, reconnect proton".to_string());
    }
    let port_differs = q.listen_port != port;
    let decision = if port_differs || rebind_interface || disable_upnp {
        Decision::WouldSet(port)
    } else {
        Decision::InSync
    };
    Plan {
        decision,
        rebind_interface,
        disable_upnp,
        warnings,
    }
}

/// Last `Port pair <N>->` match in a log text.
pub fn parse_forwarded_port(text: &str) -> Option<u16> {
    const KEY: &str = "Port pair ";
    let mut found = None;
    let mut rest = text;
    while let Some(i) = rest.find(KEY) {
        rest = &rest[i + KEY.len()..];
        let digits: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
        if !digits.is_empty() && rest[digits.len()..].starts_with("->") {
            if let Ok(p) = digits.parse() {
                found = Some(p);
            }
        }
    }
    found
}

/// Newest two `client-logs*.txt` files; the first with a match wins.
pub fn latest_forwarded_port(dir: &Path) -> Option<u16> {
    let mut logs: Vec<(SystemTime, std::path::PathBuf)> = std::fs::read_dir(dir)
        .ok()?
        .filter_map(|e| e.ok())
        .filter(|e| {
            let n = e.file_name().to_string_lossy().to_string();
            n.starts_with("client-logs") && n.ends_with(".txt")
        })
        .filter_map(|e| Some((e.metadata().ok()?.modified().ok()?, e.path())))
        .collect();
    logs.sort_by_key(|l| std::cmp::Reverse(l.0));
    logs.iter().take(2).find_map(|(_, p)| {
        let bytes = std::fs::read(p).ok()?;
        parse_forwarded_port(&String::from_utf8_lossy(&bytes))
    })
}

/// Port from the old script's `state.json` (WG mode), read-only.
pub fn port_from_state_json(text: &str) -> Option<u16> {
    let v: serde_json::Value = serde_json::from_str(text).ok()?;
    u16::try_from(v.get("port")?.as_u64()?).ok()
}

/// Parse `netsh int ipv4 show excludedportrange` output into ranges.
pub fn parse_excluded_ranges(text: &str) -> Vec<(u16, u16)> {
    text.lines()
        .filter_map(|l| {
            let mut it = l.split_whitespace();
            let a: u16 = it.next()?.parse().ok()?;
            let b: u16 = it.next()?.parse().ok()?;
            Some((a, b))
        })
        .collect()
}

pub fn in_ranges(ranges: &[(u16, u16)], port: u16) -> bool {
    ranges.iter().any(|(a, b)| *a <= port && port <= *b)
}

#[cfg(windows)]
fn port_reserved(port: u16) -> bool {
    ["tcp", "udp"].iter().any(|proto| {
        std::process::Command::new("netsh")
            .args([
                "int",
                "ipv4",
                "show",
                "excludedportrange",
                &format!("protocol={proto}"),
            ])
            .output()
            .map(|o| {
                in_ranges(
                    &parse_excluded_ranges(&String::from_utf8_lossy(&o.stdout)),
                    port,
                )
            })
            .unwrap_or(false)
    })
}

#[cfg(not(windows))]
fn port_reserved(_port: u16) -> bool {
    false
}

fn read_qbt(cfg: &Config) -> Result<QbtView, String> {
    let q = Qbt::new(cfg).map_err(|e| e.to_string())?;
    let p = q.preferences().map_err(|e| e.to_string())?;
    let interfaces = q.interfaces().map_err(|e| e.to_string())?;
    Ok(QbtView {
        listen_port: p.listen_port,
        bound_interface: p.current_network_interface,
        upnp: p.upnp,
        interfaces,
    })
}

/// Gather an observation using read-only queries only.
pub fn observe(cfg: &Config) -> Observation {
    let vpn = adapters::list().ok().and_then(|a| adapters::pick_vpn(&a));
    let (forwarded, forwarded_note) = match &vpn {
        None => (None, None),
        Some(v) if v.mode == VpnMode::App => match cfg.proton_log_dir.as_deref() {
            Some(d) => match latest_forwarded_port(d) {
                Some(p) => (Some(p), None),
                None => (None, Some("no forwarded port in proton logs".to_string())),
            },
            None => (None, Some("proton log folder not configured".to_string())),
        },
        Some(_) => match cfg
            .portsync_state_file
            .as_deref()
            .and_then(|f| std::fs::read_to_string(f).ok())
            .and_then(|t| port_from_state_json(&t))
        {
            Some(p) => (Some(p), None),
            None => (
                None,
                Some("wireguard mode: nat-pmp is not queried in phase 1".to_string()),
            ),
        },
    };
    let reserved = forwarded.map(port_reserved).unwrap_or(false);
    Observation {
        vpn,
        forwarded,
        forwarded_note,
        reserved,
        qbt: read_qbt(cfg),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapters::{AdapterKind, ProtonProtocol};

    fn vpn() -> VpnPick {
        VpnPick {
            adapter: "ProtonVPN".into(),
            mode: VpnMode::App,
            kind: AdapterKind::ProtonApp(ProtonProtocol::WireGuard),
        }
    }

    fn ifaces() -> Vec<Iface> {
        vec![
            Iface {
                name: "ProtonVPN".into(),
                value: "if_a".into(),
            },
            Iface {
                name: "Tailscale".into(),
                value: "if_b".into(),
            },
        ]
    }

    fn view(port: u16, bound: &str, upnp: bool) -> QbtView {
        QbtView {
            listen_port: port,
            bound_interface: bound.into(),
            upnp,
            interfaces: ifaces(),
        }
    }

    fn obs(qbt: QbtView) -> Observation {
        Observation {
            vpn: Some(vpn()),
            forwarded: Some(40001),
            forwarded_note: None,
            reserved: false,
            qbt: Ok(qbt),
        }
    }

    #[test]
    fn in_sync() {
        let p = decide(&obs(view(40001, "if_a", false)));
        assert_eq!(p.decision, Decision::InSync);
        assert!(!p.rebind_interface && !p.disable_upnp);
    }

    #[test]
    fn port_differs() {
        let p = decide(&obs(view(40002, "if_a", false)));
        assert_eq!(p.decision, Decision::WouldSet(40001));
    }

    #[test]
    fn swapped_interface_ids_trigger_rebind() {
        let p = decide(&obs(view(40001, "if_b", false)));
        assert_eq!(p.decision, Decision::WouldSet(40001));
        assert!(p.rebind_interface);
    }

    #[test]
    fn upnp_on_triggers_disable() {
        let p = decide(&obs(view(40001, "if_a", true)));
        assert_eq!(p.decision, Decision::WouldSet(40001));
        assert!(p.disable_upnp);
    }

    #[test]
    fn no_vpn_is_unknown() {
        let mut o = obs(view(40001, "if_a", false));
        o.vpn = None;
        assert!(matches!(decide(&o).decision, Decision::Unknown(_)));
    }

    #[test]
    fn qbt_down_is_unknown() {
        let mut o = obs(view(1, "x", false));
        o.qbt = Err("unreachable".into());
        let p = decide(&o);
        assert!(matches!(p.decision, Decision::Unknown(ref r) if r.contains("unreachable")));
    }

    #[test]
    fn missing_port_is_unknown_with_note() {
        let mut o = obs(view(40001, "if_a", false));
        o.forwarded = None;
        o.forwarded_note = Some("no forwarded port in proton logs".into());
        let p = decide(&o);
        assert_eq!(
            p.decision,
            Decision::Unknown("no forwarded port in proton logs".into())
        );
    }

    #[test]
    fn adapter_missing_in_qbt_is_unknown() {
        let mut o = obs(view(40001, "if_a", false));
        o.vpn.as_mut().unwrap().adapter = "ProtonVPN TUN".into();
        assert!(matches!(decide(&o).decision, Decision::Unknown(_)));
    }

    #[test]
    fn reserved_port_warns_but_still_decides() {
        let mut o = obs(view(40001, "if_a", false));
        o.reserved = true;
        let p = decide(&o);
        assert_eq!(p.decision, Decision::InSync);
        assert_eq!(p.warnings.len(), 1);
        assert!(p.summary().contains("warning"));
    }

    #[test]
    fn parses_last_port_pair() {
        let text =
            "x Port pair 40001->40001 ok\nmore\nPort pair 40007->40007 renewed\nPort pair abc->1";
        assert_eq!(parse_forwarded_port(text), Some(40007));
        assert_eq!(parse_forwarded_port("nothing here"), None);
        assert_eq!(parse_forwarded_port("Port pair 99999999->1"), None);
    }

    #[test]
    fn parses_state_json_port() {
        assert_eq!(
            port_from_state_json(r#"{"mode":"wg","port":40009}"#),
            Some(40009)
        );
        assert_eq!(port_from_state_json(r#"{"port":null}"#), None);
        assert_eq!(port_from_state_json("garbage"), None);
    }

    #[test]
    fn excluded_ranges_parse_and_match() {
        let text = "\
Protocol tcp Port Exclusion Ranges

Start Port    End Port
----------    --------
     40000       40099
     49152       49251     *
";
        let r = parse_excluded_ranges(text);
        assert_eq!(r, vec![(40000, 40099), (49152, 49251)]);
        assert!(in_ranges(&r, 40001));
        assert!(!in_ranges(&r, 40100));
    }

    #[test]
    fn latest_log_wins_from_fixture_dir() {
        let d = std::env::temp_dir().join(format!("mediactl-logs-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(d.join("client-logs-1.txt"), "Port pair 40001->40001").unwrap();
        std::fs::write(d.join("other.txt"), "Port pair 40005->40005").unwrap();
        assert_eq!(latest_forwarded_port(&d), Some(40001));
        std::fs::remove_dir_all(d).ok();
    }
}

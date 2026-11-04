//! Network adapter discovery and VPN adapter selection.

use anyhow::Result;
use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ProtonProtocol {
    WireGuard,
    OpenVpn,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AdapterKind {
    /// Proton desktop app adapter (`ProtonVPN` on WireGuard, `ProtonVPN TUN` on OpenVPN).
    ProtonApp(ProtonProtocol),
    /// Standalone WireGuard tunnel service, adapter named exactly `proton`.
    RigTunnel,
    Tailscale,
    Other,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Adapter {
    pub name: String,
    pub connected: bool,
    pub kind: AdapterKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum VpnMode {
    App,
    Wg,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct VpnPick {
    pub adapter: String,
    pub mode: VpnMode,
    pub kind: AdapterKind,
}

/// Classify an adapter by name. Proton is matched by the `ProtonVPN` prefix,
/// never equality, because the OpenVPN adapter is `ProtonVPN TUN`.
pub fn classify(name: &str) -> AdapterKind {
    let lower = name.to_ascii_lowercase();
    if lower.starts_with("protonvpn tun") {
        AdapterKind::ProtonApp(ProtonProtocol::OpenVpn)
    } else if lower.starts_with("protonvpn") {
        AdapterKind::ProtonApp(ProtonProtocol::WireGuard)
    } else if lower == "proton" {
        AdapterKind::RigTunnel
    } else if lower.contains("tailscale") {
        AdapterKind::Tailscale
    } else {
        AdapterKind::Other
    }
}

/// Parse `netsh interface show interface` output.
///
/// Columns are Admin State, State, Type, then the interface name (which may
/// contain spaces).
pub fn parse_netsh(text: &str) -> Vec<Adapter> {
    text.lines()
        .filter_map(|line| {
            let line = line.trim();
            if line.is_empty() || line.starts_with('-') || line.starts_with("Admin State") {
                return None;
            }
            let (_admin, rest) = split_token(line)?;
            let (state, rest) = split_token(rest)?;
            let (_ty, name) = split_token(rest)?;
            let name = name.trim();
            if name.is_empty() {
                return None;
            }
            Some(Adapter {
                name: name.to_string(),
                connected: state.eq_ignore_ascii_case("connected"),
                kind: classify(name),
            })
        })
        .collect()
}

fn split_token(s: &str) -> Option<(&str, &str)> {
    let s = s.trim_start();
    let end = s.find(char::is_whitespace)?;
    Some((&s[..end], s[end..].trim_start()))
}

/// Pick the VPN adapter the way Media ON does: a connected Proton app adapter
/// first, then the standalone tunnel. Tailscale and others never qualify.
pub fn pick_vpn(adapters: &[Adapter]) -> Option<VpnPick> {
    let app = adapters
        .iter()
        .find(|a| a.connected && matches!(a.kind, AdapterKind::ProtonApp(_)));
    if let Some(a) = app {
        return Some(VpnPick {
            adapter: a.name.clone(),
            mode: VpnMode::App,
            kind: a.kind,
        });
    }
    adapters
        .iter()
        .find(|a| a.connected && a.kind == AdapterKind::RigTunnel)
        .map(|a| VpnPick {
            adapter: a.name.clone(),
            mode: VpnMode::Wg,
            kind: a.kind,
        })
}

/// List adapters via `netsh` (read-only). Windows only.
#[cfg(windows)]
pub fn list() -> Result<Vec<Adapter>> {
    let out = std::process::Command::new("netsh")
        .args(["interface", "show", "interface"])
        .output()?;
    Ok(parse_netsh(&String::from_utf8_lossy(&out.stdout)))
}

#[cfg(not(windows))]
pub fn list() -> Result<Vec<Adapter>> {
    anyhow::bail!("adapter listing is only implemented on Windows")
}

#[cfg(test)]
mod tests {
    use super::*;

    const FIXTURE: &str = "\
Admin State    State          Type             Interface Name
-------------------------------------------------------------------------
Enabled        Connected      Dedicated        Ethernet
Enabled        Connected      Dedicated        Tailscale
Enabled        Disconnected   Dedicated        Wi-Fi
Enabled        Connected      Dedicated        ProtonVPN
Enabled        Connected      Dedicated        ProtonVPN TUN
Enabled        Connected      Dedicated        proton
";

    fn named(list: &[Adapter], n: &str) -> Adapter {
        list.iter().find(|a| a.name == n).unwrap().clone()
    }

    #[test]
    fn parses_names_with_spaces() {
        let l = parse_netsh(FIXTURE);
        assert_eq!(l.len(), 6);
        assert!(l.iter().any(|a| a.name == "ProtonVPN TUN"));
        assert!(!named(&l, "Wi-Fi").connected);
    }

    #[test]
    fn classifies_by_prefix() {
        assert_eq!(
            classify("ProtonVPN"),
            AdapterKind::ProtonApp(ProtonProtocol::WireGuard)
        );
        assert_eq!(
            classify("ProtonVPN TUN"),
            AdapterKind::ProtonApp(ProtonProtocol::OpenVpn)
        );
        assert_eq!(
            classify("ProtonVPN TUN 2"),
            AdapterKind::ProtonApp(ProtonProtocol::OpenVpn)
        );
        assert_eq!(classify("proton"), AdapterKind::RigTunnel);
        assert_eq!(classify("Tailscale"), AdapterKind::Tailscale);
        assert_eq!(classify("Ethernet"), AdapterKind::Other);
    }

    #[test]
    fn app_adapter_beats_rig_tunnel() {
        let p = pick_vpn(&parse_netsh(FIXTURE)).unwrap();
        assert_eq!(p.mode, VpnMode::App);
        assert_eq!(p.adapter, "ProtonVPN");
    }

    #[test]
    fn openvpn_only_is_found() {
        let text = "\
Enabled        Connected      Dedicated        Tailscale
Enabled        Connected      Dedicated        ProtonVPN TUN
";
        let p = pick_vpn(&parse_netsh(text)).unwrap();
        assert_eq!(p.adapter, "ProtonVPN TUN");
        assert_eq!(p.mode, VpnMode::App);
    }

    #[test]
    fn tailscale_alone_is_not_a_vpn() {
        let text = "Enabled        Connected      Dedicated        Tailscale\n";
        assert!(pick_vpn(&parse_netsh(text)).is_none());
    }

    #[test]
    fn disconnected_proton_is_ignored() {
        let text = "\
Enabled        Disconnected   Dedicated        ProtonVPN
Enabled        Connected      Dedicated        proton
";
        let p = pick_vpn(&parse_netsh(text)).unwrap();
        assert_eq!(p.mode, VpnMode::Wg);
        assert_eq!(p.adapter, "proton");
    }

    #[test]
    fn nothing_connected_picks_nothing() {
        assert!(pick_vpn(&[]).is_none());
    }
}

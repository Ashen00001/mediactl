//! Read-only qBittorrent WebUI client.
//!
//! Only GET requests are made. If credentials are configured, one login POST
//! is sent to obtain a session cookie (this changes nothing in qBittorrent's
//! settings); with localhost auth bypass no login is needed at all.

use crate::config::Config;
use anyhow::{anyhow, bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::time::Duration;

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Iface {
    pub name: String,
    pub value: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Prefs {
    pub listen_port: u16,
    #[serde(default)]
    pub current_network_interface: String,
    #[serde(default)]
    pub upnp: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
pub struct Transfer {
    #[serde(default)]
    pub dl_info_speed: u64,
    #[serde(default)]
    pub up_info_speed: u64,
}

/// Interface id for an adapter *name*. Ids for Proton and Tailscale swap
/// across reboots, so the name is the only stable key.
pub fn iface_id_for<'a>(list: &'a [Iface], name: &str) -> Option<&'a str> {
    list.iter()
        .find(|i| i.name == name)
        .map(|i| i.value.as_str())
}

pub struct Qbt {
    base: String,
    http: reqwest::blocking::Client,
}

impl Qbt {
    pub fn new(cfg: &Config) -> Result<Qbt> {
        let base = cfg
            .qbt_url
            .as_deref()
            .ok_or_else(|| anyhow!("qBittorrent URL not configured (MEDIACTL_QBT_URL)"))?
            .trim_end_matches('/')
            .to_string();
        let http = reqwest::blocking::Client::builder()
            .timeout(Duration::from_secs(5))
            .cookie_store(true)
            .build()?;
        let q = Qbt { base, http };
        if let (Some(u), Some(p)) = (&cfg.qbt_username, &cfg.qbt_password) {
            q.login(u, p)?;
        }
        Ok(q)
    }

    fn login(&self, user: &str, pass: &str) -> Result<()> {
        let r = self
            .http
            .post(format!("{}/api/v2/auth/login", self.base))
            .header("Referer", &self.base)
            .form(&[("username", user), ("password", pass)])
            .send()
            .context("qBittorrent unreachable")?;
        let body = r.text().unwrap_or_default();
        if body.trim() != "Ok." {
            bail!("qBittorrent login rejected");
        }
        Ok(())
    }

    fn get<T: serde::de::DeserializeOwned>(&self, path: &str) -> Result<T> {
        let r = self
            .http
            .get(format!("{}/api/v2{}", self.base, path))
            .send()
            .context("qBittorrent unreachable")?;
        if !r.status().is_success() {
            bail!("qBittorrent returned HTTP {}", r.status().as_u16());
        }
        r.json().context("unexpected qBittorrent response")
    }

    pub fn preferences(&self) -> Result<Prefs> {
        self.get("/app/preferences")
    }

    pub fn interfaces(&self) -> Result<Vec<Iface>> {
        self.get("/app/networkInterfaceList")
    }

    pub fn transfer(&self) -> Result<Transfer> {
        self.get("/transfer/info")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_preferences_with_extra_fields() {
        let j = r#"{"listen_port": 40001, "upnp": true, "current_network_interface": "iftype53_a", "other": 1}"#;
        let p: Prefs = serde_json::from_str(j).unwrap();
        assert_eq!(p.listen_port, 40001);
        assert!(p.upnp);
        assert_eq!(p.current_network_interface, "iftype53_a");
    }

    #[test]
    fn preferences_default_missing_fields() {
        let p: Prefs = serde_json::from_str(r#"{"listen_port": 5}"#).unwrap();
        assert!(!p.upnp);
        assert_eq!(p.current_network_interface, "");
    }

    #[test]
    fn iface_lookup_is_by_name_when_ids_swap() {
        let before = vec![
            Iface {
                name: "ProtonVPN".into(),
                value: "iftype53_1".into(),
            },
            Iface {
                name: "Tailscale".into(),
                value: "iftype53_2".into(),
            },
        ];
        let after = vec![
            Iface {
                name: "ProtonVPN".into(),
                value: "iftype53_2".into(),
            },
            Iface {
                name: "Tailscale".into(),
                value: "iftype53_1".into(),
            },
        ];
        assert_eq!(iface_id_for(&before, "ProtonVPN"), Some("iftype53_1"));
        assert_eq!(iface_id_for(&after, "ProtonVPN"), Some("iftype53_2"));
        assert_eq!(iface_id_for(&after, "ProtonVPN TUN"), None);
    }

    #[test]
    fn transfer_parses() {
        let t: Transfer =
            serde_json::from_str(r#"{"dl_info_speed": 1000, "up_info_speed": 20, "x": 1}"#)
                .unwrap();
        assert_eq!(t.dl_info_speed, 1000);
        assert_eq!(t.up_info_speed, 20);
    }

    #[test]
    fn missing_url_is_an_error() {
        let cfg = Config {
            state_dir: std::env::temp_dir(),
            qbt_url: None,
            qbt_username: None,
            qbt_password: None,
            jellyfin_url: None,
            proton_log_dir: None,
            portsync_state_file: None,
        };
        assert!(Qbt::new(&cfg).is_err());
    }
}

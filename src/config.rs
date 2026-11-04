//! Configuration: env vars override an untracked `config.json` in the state folder.

use anyhow::{bail, Context, Result};
use serde::Deserialize;
use std::path::{Path, PathBuf};

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct FileConfig {
    qbt_url: Option<String>,
    qbt_username: Option<String>,
    qbt_password: Option<String>,
    jellyfin_url: Option<String>,
    proton_log_dir: Option<String>,
    portsync_state_file: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Config {
    pub state_dir: PathBuf,
    pub qbt_url: Option<String>,
    pub qbt_username: Option<String>,
    pub qbt_password: Option<String>,
    pub jellyfin_url: Option<String>,
    pub proton_log_dir: Option<PathBuf>,
    /// The old portsync's `state.json`, read only to learn the port in WG mode.
    pub portsync_state_file: Option<PathBuf>,
}

pub type EnvFn<'a> = &'a dyn Fn(&str) -> Option<String>;

pub fn real_env(key: &str) -> Option<String> {
    std::env::var(key).ok().filter(|v| !v.is_empty())
}

/// `MEDIACTL_STATE_DIR`, else `<home>/MediaStack/mediactl`.
pub fn default_state_dir(env: EnvFn) -> Result<PathBuf> {
    if let Some(d) = env("MEDIACTL_STATE_DIR") {
        return Ok(PathBuf::from(d));
    }
    let home = env("USERPROFILE")
        .or_else(|| env("HOME"))
        .context("cannot find a home folder; set MEDIACTL_STATE_DIR")?;
    Ok(Path::new(&home).join("MediaStack").join("mediactl"))
}

/// The state folder must never live inside a git repository.
pub fn ensure_outside_repo(dir: &Path) -> Result<()> {
    let mut cur = Some(dir);
    while let Some(p) = cur {
        if p.join(".git").exists() {
            bail!("state folder {} is inside a git repository", p.display());
        }
        cur = p.parent();
    }
    Ok(())
}

impl Config {
    pub fn load(env: EnvFn) -> Result<Config> {
        let state_dir = default_state_dir(env)?;
        Self::load_from(state_dir, env)
    }

    pub fn load_from(state_dir: PathBuf, env: EnvFn) -> Result<Config> {
        ensure_outside_repo(&state_dir)?;
        let path = state_dir.join("config.json");
        let file: FileConfig = match std::fs::read_to_string(&path) {
            Ok(text) => serde_json::from_str(&text)
                .with_context(|| format!("invalid config file {}", path.display()))?,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => FileConfig::default(),
            Err(e) => return Err(e).context("reading config file"),
        };
        let pick = |key: &str, f: Option<String>| env(key).or(f);
        let proton_log_dir = pick("MEDIACTL_PROTON_LOG_DIR", file.proton_log_dir)
            .map(PathBuf::from)
            .or_else(|| {
                env("LOCALAPPDATA")
                    .map(|l| Path::new(&l).join("Proton").join("Proton VPN").join("Logs"))
            });
        Ok(Config {
            qbt_url: pick("MEDIACTL_QBT_URL", file.qbt_url),
            qbt_username: pick("MEDIACTL_QBT_USER", file.qbt_username),
            qbt_password: pick("MEDIACTL_QBT_PASS", file.qbt_password),
            jellyfin_url: pick("MEDIACTL_JELLYFIN_URL", file.jellyfin_url),
            portsync_state_file: pick("MEDIACTL_PORTSYNC_STATE", file.portsync_state_file)
                .map(PathBuf::from),
            proton_log_dir,
            state_dir,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn env_of(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
        let m: HashMap<String, String> = pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        move |k| m.get(k).cloned()
    }

    fn scratch(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("mediactl-test-{name}-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn state_dir_from_override_and_home() {
        let e = env_of(&[("MEDIACTL_STATE_DIR", "/x/state")]);
        assert_eq!(default_state_dir(&e).unwrap(), PathBuf::from("/x/state"));
        let e = env_of(&[("HOME", "/h")]);
        assert_eq!(
            default_state_dir(&e).unwrap(),
            Path::new("/h").join("MediaStack").join("mediactl")
        );
        assert!(default_state_dir(&env_of(&[])).is_err());
    }

    #[test]
    fn missing_file_uses_env_only() {
        let d = scratch("missing");
        let e = env_of(&[("MEDIACTL_QBT_URL", "http://192.0.2.10:17000")]);
        let c = Config::load_from(d.clone(), &e).unwrap();
        assert_eq!(c.qbt_url.as_deref(), Some("http://192.0.2.10:17000"));
        assert!(c.qbt_username.is_none());
        std::fs::remove_dir_all(d).ok();
    }

    #[test]
    fn env_overrides_file() {
        let d = scratch("override");
        std::fs::write(
            d.join("config.json"),
            r#"{"qbt_url":"http://192.0.2.1:1","jellyfin_url":"http://192.0.2.2:2"}"#,
        )
        .unwrap();
        let e = env_of(&[("MEDIACTL_QBT_URL", "http://192.0.2.9:9")]);
        let c = Config::load_from(d.clone(), &e).unwrap();
        assert_eq!(c.qbt_url.as_deref(), Some("http://192.0.2.9:9"));
        assert_eq!(c.jellyfin_url.as_deref(), Some("http://192.0.2.2:2"));
        std::fs::remove_dir_all(d).ok();
    }

    #[test]
    fn unknown_keys_and_bad_json_are_errors() {
        let d = scratch("bad");
        std::fs::write(d.join("config.json"), r#"{"qbt_ulr":"x"}"#).unwrap();
        assert!(Config::load_from(d.clone(), &env_of(&[])).is_err());
        std::fs::write(d.join("config.json"), "not json").unwrap();
        assert!(Config::load_from(d.clone(), &env_of(&[])).is_err());
        std::fs::remove_dir_all(d).ok();
    }

    #[test]
    fn refuses_state_dir_inside_a_repo() {
        let d = scratch("repo");
        std::fs::create_dir_all(d.join(".git")).unwrap();
        let inner = d.join("state");
        assert!(Config::load_from(inner, &env_of(&[])).is_err());
        std::fs::remove_dir_all(d).ok();
    }

    #[test]
    fn proton_log_dir_defaults_from_localappdata() {
        let d = scratch("logdir");
        let e = env_of(&[("LOCALAPPDATA", "/la")]);
        let c = Config::load_from(d.clone(), &e).unwrap();
        assert!(c.proton_log_dir.unwrap().ends_with("Logs"));
        std::fs::remove_dir_all(d).ok();
    }
}

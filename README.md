# mediactl

A small Rust tool that watches a self-hosted media stack (qBittorrent behind a
Proton VPN, Sonarr/Prowlarr, Jellyfin behind Tailscale Funnel) and works out
what a controller *would* do about it. Phase 1 is **read-only**: it reports and
logs decisions and never acts.

## Why shadow-first

The stack is run by a pile of PowerShell and Python scripts that people use
every day. Replacing them with one reliable program is worth it (one process,
tests, no long-running interpreters), but only if the new program is proven
right first. So mediactl runs next to the old scripts, computes the decision
the old script would make, and logs it. After weeks of agreement, pieces get
switched over one at a time. The old scripts stay as the fallback until then.
How the old scripts behave is written down in
[docs/current-behavior.md](docs/current-behavior.md).

## Usage

```
mediactl status                  # snapshot
mediactl status --json           # same, machine-readable
mediactl status --watch          # live terminal view (q to quit)
mediactl portsync --shadow       # one decision, logged
mediactl portsync --shadow --loop 30   # repeat every 30 seconds
```

`--shadow` is mandatory. There is no flag in phase 1 that makes it act.

Adapter discovery and the reserved-port check are Windows-only
(`cfg(windows)`); on other platforms they report "unknown", which keeps CI and
the unit tests portable.

### Configuration

Environment variables override an untracked `config.json` in the state folder.

| Setting | Env var | Config key |
| --- | --- | --- |
| qBittorrent WebUI URL | `MEDIACTL_QBT_URL` | `qbt_url` |
| qBittorrent user (optional) | `MEDIACTL_QBT_USER` | `qbt_username` |
| qBittorrent password (optional) | `MEDIACTL_QBT_PASS` | `qbt_password` |
| Jellyfin URL (optional health check) | `MEDIACTL_JELLYFIN_URL` | `jellyfin_url` |
| Proton client log folder | `MEDIACTL_PROTON_LOG_DIR` | `proton_log_dir` |
| Old portsync `state.json` (WireGuard mode) | `MEDIACTL_PORTSYNC_STATE` | `portsync_state_file` |
| State folder | `MEDIACTL_STATE_DIR` | n/a |

The state folder defaults to `<home>/MediaStack/mediactl`. Logs go to
`mediactl.log` there. mediactl refuses a state folder inside a git repository.
With qBittorrent's localhost auth bypass on, no credentials are needed.

### Install

```
cargo install --path .
```

## Safety model

- Never starts or stops a process, never changes a qBittorrent setting, never
  touches the VPN or Tailscale, never edits or runs the existing scripts.
- qBittorrent access is GET-only. The one exception is the session login POST
  when credentials are configured; it changes no settings.
- Only writes: its own log in the state folder.
- Secrets live in env vars or the untracked config file, never in the repo.
  Tests use documentation addresses and fake ports.

## Roadmap

1. **Phase 1 (this)**: status, portsync decision, watchdog decision (pure, not
   yet wired to a live probe), shadow logging.
2. **Phase 2**: watchdog and the importer as supervised tasks, still compared
   against the old scripts before taking over. Live funnel probe.
3. **Phase 3**: Media ON/OFF supervisor.

Old scripts remain the fallback until each piece has proven itself.

## Work in progress

- Standalone-tunnel (WireGuard) mode does not query NAT-PMP yet; it can read
  the old script's state file if configured, else reports unknown.
- The watchdog decision is implemented and tested but has no live probe yet.
- Only the Proton-app port-from-logs path has been checked against a live
  system.

## License

MIT

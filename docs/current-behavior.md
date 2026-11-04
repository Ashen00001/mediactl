# Current behavior of the scripts mediactl will eventually replace

This describes what the existing PowerShell/Python scripts decide and when. It
is the spec mediactl's shadow mode is checked against. No real values appear
here on purpose.

## portsync (Python, long-running, ticks every ~2 s)

**Goal:** keep qBittorrent bound to the Proton VPN adapter and listening on the
port Proton forwarded to us.

### Mode selection (each tick)

1. **App mode**: the Proton desktop app is connecting, meaning its service is
   running *or* an adapter whose name starts with `ProtonVPN` is `Connected`
   in `netsh interface show interface`.
   - If the standalone WireGuard tunnel service (the "rig tunnel", adapter
     named exactly `proton`) is also running, portsync stops it so the app
     takes over.
   - Otherwise, at most every ~45 s, it reads the forwarded port, then syncs
     qBittorrent.
2. **WG mode**: only the rig tunnel is running. At most every ~45 s it asks the
   VPN gateway for a port mapping via NAT-PMP (UDP and TCP), then syncs.
3. Neither: idle, mode reset.

### Where the forwarded port comes from

- App mode: the newest of the two most recent Proton client log files that
  contains a line `Port pair <N>-><M>`; the **last** such match in that file
  wins.
- WG mode: NAT-PMP reply (public port field).

### Reserved ports

Windows (Hyper-V/WinNAT) reserves random port ranges. If the forwarded port
falls in any TCP or UDP excluded range, qBittorrent cannot listen on it.
App mode: record a warning ("reconnect Proton"). WG mode: switch to the next
server.

### sync_qbt(port, adapter)

Reads qBittorrent preferences over the local WebUI API, then builds a set of
changes:

- Adapter name must exist in qBittorrent's network interface list, else give
  up (returns not-synced). The interface id is looked up **by name**, because
  ids for Proton and Tailscale (both WireGuard-style adapters) swap between
  reboots.
- If the bound interface differs: set interface, name, clear address.
- If UPnP is on: turn it off.
- If listen port differs: set it.
- If anything changed and the port itself did not: nudge the port (+1, wait,
  back) to force a re-bind. A restart races the old socket, so it never
  restarts qBittorrent.
- qBittorrent unreachable: not synced, no error.

### Adapter name gotchas

- Proton app on WireGuard: adapter `ProtonVPN`. On OpenVPN: `ProtonVPN TUN`.
  Always match by the `ProtonVPN` **prefix**, never equality.
- Rig tunnel adapter is `proton` (lowercase, no "VPN"). Different thing.
- Tailscale is a separate adapter and must never be mistaken for the VPN.
- A name match must also be Connected/Up; a disconnected leftover adapter
  does not count.

### Failure handling (WG mode only)

NAT-PMP timeouts: logged on the 1st and 8th consecutive failure; after 8 it
requests the next server and waits. Not relevant to app mode.

## sync-vpn.ps1 (one-shot, called by Media ON in app mode)

Waits (up to ~60 s) for the qBittorrent API, finds the Up adapter matching
`ProtonVPN*`, resolves qBittorrent's interface id by name, reads the forwarded
port from the same Proton client logs (it also looks in the machine-wide log
folder), applies interface/address/port if different, always nudges the port,
and prints a status line. Warns if the port is reserved. Errors if
qBittorrent cannot see the adapter.

## funnel-watchdog.ps1 (scheduled task, every minute or two)

**Goal:** keep the public Jellyfin link reachable through Tailscale Funnel.

Per run:

1. Jellyfin process not running: exit, touch nothing.
2. Probe the public path from outside: resolve the public name through a DoH
   resolver, then fetch the public info endpoint against **each ingress IP**
   with a pinned resolve (testing via the local name only hairpins over the
   tailnet and proves nothing).
   - DoH itself unreachable, or no A records: result `nonet`.
   - NXDOMAIN: `down` (Tailscale stopped publishing the record).
   - Any ingress IP answers HTTP 200: `ok`; else `down`.
3. `ok`: clear the failure counter (log recovery if it was non-zero), exit.
4. `nonet`: exit, counter unchanged.
5. `down`: increment the persisted counter. Under 2: exit (one blip is
   tolerated).
6. At 2 consecutive failures: `funnel reset`, re-enable the public and backup
   Funnel ports plus the tailnet-only serve, and poll up to ~60 s for `ok`.
7. Still down: if the last Tailscale down/up cycle was 30+ minutes ago, cycle
   Tailscale and poll up to ~90 s; otherwise log and retry next run.
8. Counter is left at 1 after a failed escalation so the next run escalates
   again immediately.

Known edge: if Tailscale cannot reach its control plane (for example Proton's
split tunnel lacks the Tailscale daemon), resets cannot help and the watchdog
loops. A smarter decision would check Tailscale health first. That is a
candidate improvement, not current behavior.

## Media ON / OFF (context only)

- ON: refuses unless a VPN adapter is up (prefers `ProtonVPN*`, else starts
  the rig tunnel), starts qBittorrent, Prowlarr, Sonarr, portsync, the
  importer and the access-tagging loop; calls sync-vpn in app mode.
- OFF: stops those helpers and apps, shuts the arr apps down via their APIs,
  closes qBittorrent, drops the rig tunnel. Jellyfin stays up.

## Logs

portsync only logs on changes and failures (start, qBittorrent updates, NAT-PMP
failures, server switches), so a quiet log means "in sync". The watchdog logs
only down/recovery transitions.

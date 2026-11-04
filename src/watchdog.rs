//! Funnel watchdog decision (shadow only). Pure: the caller supplies the
//! probe result and persisted state; nothing here touches Tailscale.

use serde::Serialize;

/// Minutes between Tailscale down/up cycles.
pub const CYCLE_COOLDOWN_MIN: u64 = 30;
/// Consecutive failures before the watchdog escalates.
pub const FAILS_BEFORE_RESET: u32 = 2;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Probe {
    Ok,
    Down,
    /// No internet at all (DNS-over-HTTPS unreachable or no records).
    NoNet,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WatchdogState {
    pub consecutive_fails: u32,
    /// Minutes since the last Tailscale down/up cycle, if ever.
    pub minutes_since_cycle: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum WatchdogAction {
    /// Jellyfin not running: nothing to expose, touch nothing.
    SkipJellyfinDown,
    Healthy {
        recovered_after: Option<u32>,
    },
    /// No internet: nothing Funnel can fix.
    IgnoreNoNet,
    /// A single blip is tolerated.
    WaitAndRecheck {
        fails: u32,
    },
    /// Reset and re-enable Funnel; escalate to a Tailscale cycle if still
    /// down and the cooldown allows it.
    ResetFunnel {
        then_cycle_tailscale: bool,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct WatchdogDecision {
    pub action: WatchdogAction,
    /// Counter value after this run, before any escalation result is known.
    pub next_fails: u32,
}

pub fn decide(jellyfin_running: bool, probe: Probe, st: WatchdogState) -> WatchdogDecision {
    if !jellyfin_running {
        return WatchdogDecision {
            action: WatchdogAction::SkipJellyfinDown,
            next_fails: st.consecutive_fails,
        };
    }
    match probe {
        Probe::Ok => WatchdogDecision {
            action: WatchdogAction::Healthy {
                recovered_after: (st.consecutive_fails > 0).then_some(st.consecutive_fails),
            },
            next_fails: 0,
        },
        Probe::NoNet => WatchdogDecision {
            action: WatchdogAction::IgnoreNoNet,
            next_fails: st.consecutive_fails,
        },
        Probe::Down => {
            let fails = st.consecutive_fails + 1;
            if fails < FAILS_BEFORE_RESET {
                return WatchdogDecision {
                    action: WatchdogAction::WaitAndRecheck { fails },
                    next_fails: fails,
                };
            }
            let then_cycle = st
                .minutes_since_cycle
                .map(|m| m >= CYCLE_COOLDOWN_MIN)
                .unwrap_or(true);
            WatchdogDecision {
                action: WatchdogAction::ResetFunnel {
                    then_cycle_tailscale: then_cycle,
                },
                // The old script leaves the counter at 1 after a failed
                // escalation so the next run escalates right away.
                next_fails: 1,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn st(f: u32, m: Option<u64>) -> WatchdogState {
        WatchdogState {
            consecutive_fails: f,
            minutes_since_cycle: m,
        }
    }

    #[test]
    fn jellyfin_down_touches_nothing() {
        let d = decide(false, Probe::Down, st(3, None));
        assert_eq!(d.action, WatchdogAction::SkipJellyfinDown);
        assert_eq!(d.next_fails, 3);
    }

    #[test]
    fn ok_clears_counter() {
        let d = decide(true, Probe::Ok, st(0, None));
        assert_eq!(
            d.action,
            WatchdogAction::Healthy {
                recovered_after: None
            }
        );
        let d = decide(true, Probe::Ok, st(1, None));
        assert_eq!(
            d.action,
            WatchdogAction::Healthy {
                recovered_after: Some(1)
            }
        );
        assert_eq!(d.next_fails, 0);
    }

    #[test]
    fn nonet_is_ignored() {
        let d = decide(true, Probe::NoNet, st(1, None));
        assert_eq!(d.action, WatchdogAction::IgnoreNoNet);
        assert_eq!(d.next_fails, 1);
    }

    #[test]
    fn first_failure_waits() {
        let d = decide(true, Probe::Down, st(0, None));
        assert_eq!(d.action, WatchdogAction::WaitAndRecheck { fails: 1 });
        assert_eq!(d.next_fails, 1);
    }

    #[test]
    fn second_failure_resets_and_may_cycle() {
        let d = decide(true, Probe::Down, st(1, None));
        assert_eq!(
            d.action,
            WatchdogAction::ResetFunnel {
                then_cycle_tailscale: true
            }
        );
        let d = decide(true, Probe::Down, st(1, Some(30)));
        assert_eq!(
            d.action,
            WatchdogAction::ResetFunnel {
                then_cycle_tailscale: true
            }
        );
    }

    #[test]
    fn cooldown_blocks_cycle() {
        let d = decide(true, Probe::Down, st(1, Some(29)));
        assert_eq!(
            d.action,
            WatchdogAction::ResetFunnel {
                then_cycle_tailscale: false
            }
        );
        assert_eq!(d.next_fails, 1);
    }
}

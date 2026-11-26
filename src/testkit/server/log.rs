//! The request log (TST-3, TST-6) and the rules that fail a test (POL-1, POL-2).

use std::collections::HashMap;
use std::fmt;
use std::time::Duration;

use crate::clock::{Mono, Time};
use crate::testkit::github::{AskedOf, Rejected};

use super::fault::Fault;

/// POL-2: the least time between the end of one request and the start of the next.
pub(super) const GAP: Duration = Duration::from_secs(1);

/// One request as the fake GitHub received it (TST-3: "it records every request").
#[derive(Clone, PartialEq)]
pub struct Request {
    /// Its place in the log, from 0.
    pub index: usize,
    /// The clock's monotonic time when it arrived.
    pub mono: Mono,
    /// The clock's wall time (the local one, not GitHub's) when it arrived.
    pub wall: Time,
    /// What it cost by GitHub's formula: 0 when nothing was executed (a fault, a refusal).
    pub cost: u32,
    /// `Some` when it held a finding; inside, that page's cursor.
    pub finding: Option<Option<String>>,
    /// Each node ID asked for, with what was asked of it.
    pub nodes: Vec<(String, AskedOf)>,
    /// A by-name lookup's owner, repository and number.
    pub lookup: Option<(String, String, u64)>,
    /// The `Authorization` header's value as received.
    pub authorization: Option<String>,
    /// The `x-circitor-test-pid` header's value (TST-1).
    pub pid: Option<String>,
    /// The transport's fault it was answered with.
    pub fault: Option<Fault>,
    /// Why it couldn't be answered: it breaks a rule (TST-3).
    pub rejected: Option<Rejected>,
}

impl Request {
    /// The node IDs asked for, in order.
    pub fn node_ids(&self) -> Vec<&str> {
        self.nodes.iter().map(|(id, _)| id.as_str()).collect()
    }
}

// OPS-5, TST-11: the `Authorization` value holds the token; a panic message or a failed assertion
// that prints a request never shows it.
impl fmt::Debug for Request {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Request")
            .field("index", &self.index)
            .field("mono", &self.mono.as_duration())
            .field("wall", &self.wall.rfc3339())
            .field("cost", &self.cost)
            .field("finding", &self.finding)
            .field("nodes", &self.nodes)
            .field("lookup", &self.lookup)
            .field(
                "authorization",
                &self
                    .authorization
                    .as_ref()
                    .map(|a| format!("<{} bytes>", a.len())),
            )
            .field("pid", &self.pid)
            .field("fault", &self.fault)
            .field("rejected", &self.rejected)
            .finish()
    }
}

/// One process's (one daemon's) part of the rules: what it has in flight and when its last request ended.
#[derive(Default)]
pub(super) struct Proc {
    pub in_flight: usize,
    pub last_end: Option<Mono>,
}

/// The per-daemon rules (POL-2), held per process under `start_real`.
#[derive(Default)]
pub(super) struct Rules {
    /// Bumped by `new_daemon`: a request begun before it ends without touching the new count.
    pub epoch: u64,
    pub procs: HashMap<Option<String>, Proc>,
}

impl Rules {
    /// A request arrives: the broken rules it shows, if any. It's in flight from now on.
    pub fn arrive(&mut self, key: &Option<String>, at: Mono) -> Vec<String> {
        let p = self.procs.entry(key.clone()).or_default();
        let mut broken = Vec::new();
        if p.in_flight > 0 {
            // POL-2: never two in flight.
            broken.push(format!(
                "POL-2: a request arrived while {} other was in flight",
                p.in_flight
            ));
        }
        if let Some(end) = p.last_end {
            let gap = at.since(end);
            if p.in_flight == 0 && gap < GAP {
                // POL-2: at least 1 s after the last one ended, on the clock's monotonic time.
                broken.push(format!(
                    "POL-2: a request {gap:?} after the last one ended, under 1 s"
                ));
            }
        }
        p.in_flight += 1;
        broken
    }

    /// A request begun in `epoch` ends at `at`.
    pub fn end(&mut self, key: &Option<String>, epoch: u64, at: Mono) {
        if epoch != self.epoch {
            return;
        }
        let p = self.procs.entry(key.clone()).or_default();
        p.in_flight = p.in_flight.saturating_sub(1);
        p.last_end = Some(at);
    }

    /// `new_daemon`: the checks start afresh.
    pub fn reset(&mut self) {
        self.epoch += 1;
        self.procs.clear();
    }
}

/// A short account of a request for a broken rule's message: never its `Authorization`.
pub(super) fn summary(r: &Request) -> String {
    let mut parts = vec![
        format!("request #{}", r.index),
        format!("at mono {:?}", r.mono.as_duration()),
    ];
    if let Some(pid) = &r.pid {
        parts.push(format!("pid {pid}"));
    }
    if let Some(cursor) = &r.finding {
        parts.push(format!("finding after {cursor:?}"));
    }
    if !r.nodes.is_empty() {
        parts.push(format!("nodes {:?}", r.node_ids()));
    }
    if let Some((o, n, num)) = &r.lookup {
        parts.push(format!("lookup {o}/{n}#{num}"));
    }
    parts.join(", ")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(ms: u64) -> Mono {
        Mono::ZERO.plus(Duration::from_millis(ms))
    }

    // POL-2: one in flight, 1 s between the end of one and the start of the next.
    #[test]
    fn pol2_rules() {
        let mut r = Rules::default();
        let k = None;
        assert!(r.arrive(&k, at(0)).is_empty());
        assert_eq!(r.arrive(&k, at(10)).len(), 1);
        r.end(&k, 0, at(20));
        r.end(&k, 0, at(20));
        assert!(r.arrive(&k, at(1020)).is_empty());
        r.end(&k, 0, at(1030));
        assert!(r.arrive(&k, at(1900))[0].contains("under 1 s"));
        r.end(&k, 0, at(1900));
        r.reset();
        assert!(r.arrive(&k, at(1901)).is_empty());
        // An end from before the reset doesn't count against the new daemon.
        r.end(&k, 0, at(1902));
        assert_eq!(r.procs[&k].in_flight, 1);
        let other = Some("7".to_owned());
        assert!(r.arrive(&other, at(1903)).is_empty());
    }

    // OPS-5: a request's `Debug` never prints the `Authorization` value.
    #[test]
    fn ops5_debug_hides_the_authorization() {
        let r = Request {
            index: 0,
            mono: Mono::ZERO,
            wall: Time::from_unix_millis(0),
            cost: 0,
            finding: None,
            nodes: Vec::new(),
            lookup: None,
            authorization: Some("bearer secret".into()),
            pid: None,
            fault: None,
            rejected: None,
        };
        assert!(!format!("{r:?}").contains("secret"));
        assert!(!summary(&r).contains("secret"));
    }
}

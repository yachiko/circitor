//! The transport's faults (TST-3, "it can be told to misbehave"), what each is aimed at, and for how
//! long. The faults that change an answer are the world's (`testkit::github`); `FakeGitHub` exposes
//! two of them under the same aim and count (`pr_error`, `field_error`).

use crate::testkit::github::Asked;

/// One way the fake GitHub misbehaves on the wire. The bodies that carry text hold
/// `MARKER_GITHUB_TEXT` (TST-11).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Fault {
    /// This HTTP status with a short text page: a 500, a 502 (docs/02 §5: a 5xx's shape isn't measured).
    Status(u16),
    /// No answer at all, until the client goes away or the fault is cleared (then the connection is closed).
    Hang,
    /// The connection is closed without an answer: the client sees no status.
    Drop,
    /// A 401 with the REST-style body docs/02 §5 measured (`"status": "401"`).
    Unauthorized,
    /// A 200 whose body has `errors` and no `data`, and names no rate limit.
    NoData,
    /// A 200 whose body isn't JSON.
    NotJson,
    /// A 200 whose body is shorter than its `Content-Length`: the connection closes mid-body.
    CutShort,
    /// A 302 to another path of this server, which counts its hits (`FakeGitHub::redirect_hits`, OPS-16).
    Redirect,
    /// The primary limit exhausted (POL-25, assumed): no `data`, `x-ratelimit-remaining: 0`, and
    /// `x-ratelimit-reset` only when `reset` is true.
    Exhausted { status: LimitStatus, reset: bool },
    /// A secondary limit (POL-25, assumed): its message names a secondary rate limit, with a
    /// `Retry-After` of that many seconds when there's one.
    Secondary {
        status: LimitStatus,
        retry_after: Option<u32>,
    },
}

/// How a rate-limit answer comes: its status isn't measured (docs/02 §6 item 6), so both are offered.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LimitStatus {
    /// A 403 with a REST-style body: `message` and `status`, no `data` or `errors`.
    Forbidden,
    /// A 200 with only `errors`.
    Ok,
}

/// Which requests a fault applies to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Aim {
    /// Every request.
    All,
    /// Requests that hold a finding (`viewer.pullRequests`, POL-4).
    WithFinding,
    /// Requests that hold none.
    WithoutFinding,
    /// Requests that ask for this node ID.
    Node(String),
    /// Requests that look this PR up by name (POL-16): owner and repository as GitHub matches
    /// them, ignoring case, and the number.
    Lookup {
        owner: String,
        name: String,
        number: u64,
    },
}

impl Aim {
    /// `Aim::Node` for this ID.
    pub fn node(id: &str) -> Aim {
        Aim::Node(id.to_owned())
    }

    /// `Aim::Lookup` for `owner/name#number`.
    pub fn lookup(owner: &str, name: &str, number: u64) -> Aim {
        Aim::Lookup {
            owner: owner.to_owned(),
            name: name.to_owned(),
            number,
        }
    }

    /// Whether a request that asked `asked` is aimed at. A request whose query couldn't be read
    /// (`None`) is aimed at only by `All`.
    pub(super) fn matches(&self, asked: Option<&Asked>) -> bool {
        match (self, asked) {
            (Aim::All, _) => true,
            (_, None) => false,
            (Aim::WithFinding, Some(a)) => a.finding.is_some(),
            (Aim::WithoutFinding, Some(a)) => a.finding.is_none(),
            (Aim::Node(id), Some(a)) => a.nodes.iter().any(|(n, _)| n == id),
            (
                Aim::Lookup {
                    owner,
                    name,
                    number,
                },
                Some(a),
            ) => a.lookup.as_ref().is_some_and(|(o, r, n)| {
                o.eq_ignore_ascii_case(owner) && r.eq_ignore_ascii_case(name) && n == number
            }),
        }
    }
}

/// For how long a fault holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Times {
    /// For this many requests it's aimed at, then it's gone.
    Requests(u32),
    /// Until `FakeGitHub::clear_faults`.
    UntilCleared,
}

/// A fault in force: the fault, its aim, and the requests it has left (`None`: until cleared).
pub(super) struct Entry<T> {
    pub what: T,
    pub aim: Aim,
    pub left: Option<u32>,
}

impl<T> Entry<T> {
    pub fn new(what: T, aim: Aim, times: Times) -> Entry<T> {
        let left = match times {
            Times::Requests(n) => Some(n),
            Times::UntilCleared => None,
        };
        Entry { what, aim, left }
    }

    /// Spends one request of it; true when it has none left.
    pub fn spend(&mut self) -> bool {
        match &mut self.left {
            Some(n) => {
                *n = n.saturating_sub(1);
                *n == 0
            }
            None => false,
        }
    }
}

/// Takes the first fault in force aimed at the request, spending one of its requests.
pub(super) fn take<T: Clone>(entries: &mut Vec<Entry<T>>, asked: Option<&Asked>) -> Option<T> {
    let i = entries
        .iter()
        .position(|e| e.left != Some(0) && e.aim.matches(asked))?;
    let what = entries[i].what.clone();
    if entries[i].spend() {
        entries.remove(i);
    }
    Some(what)
}

/// Takes every fault in force aimed at the request, spending one request of each.
pub(super) fn take_all<T: Clone>(entries: &mut Vec<Entry<T>>, asked: Option<&Asked>) -> Vec<T> {
    let mut taken = Vec::new();
    entries.retain_mut(|e| {
        if e.left == Some(0) || !e.aim.matches(asked) {
            return true;
        }
        taken.push(e.what.clone());
        !e.spend()
    });
    taken
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testkit::github::AskedOf;

    fn asked(finding: bool, node: Option<&str>, lookup: Option<(&str, &str, u64)>) -> Asked {
        Asked {
            finding: finding.then_some(None),
            nodes: node
                .map(|n| vec![(n.to_owned(), AskedOf::Record)])
                .unwrap_or_default(),
            lookup: lookup.map(|(o, r, n)| (o.to_owned(), r.to_owned(), n)),
        }
    }

    // TST-3: a fault can be aimed.
    #[test]
    fn tst3_aims() {
        let finding = asked(true, None, None);
        let node = asked(false, Some("PR_a"), None);
        let lookup = asked(false, None, Some(("O", "R", 3)));
        assert!(Aim::All.matches(None));
        assert!(!Aim::WithFinding.matches(None));
        assert!(Aim::WithFinding.matches(Some(&finding)));
        assert!(!Aim::WithFinding.matches(Some(&node)));
        assert!(Aim::WithoutFinding.matches(Some(&node)));
        assert!(Aim::node("PR_a").matches(Some(&node)));
        assert!(!Aim::node("PR_b").matches(Some(&node)));
        assert!(Aim::lookup("o", "r", 3).matches(Some(&lookup)));
        assert!(!Aim::lookup("o", "r", 4).matches(Some(&lookup)));
    }

    // TST-3: each fault holds for a number of requests or until cleared.
    #[test]
    fn tst3_counts() {
        let mut entries = vec![
            Entry::new(1, Aim::WithFinding, Times::Requests(2)),
            Entry::new(2, Aim::All, Times::UntilCleared),
        ];
        let f = asked(true, None, None);
        assert_eq!(take(&mut entries, Some(&f)), Some(1));
        assert_eq!(take(&mut entries, Some(&f)), Some(1));
        assert_eq!(take(&mut entries, Some(&f)), Some(2));
        assert_eq!(take(&mut entries, None), Some(2));
        let mut zero = vec![Entry::new(3, Aim::All, Times::Requests(0))];
        assert_eq!(take(&mut zero, None), None);
        let mut both = vec![
            Entry::new(4, Aim::All, Times::Requests(1)),
            Entry::new(5, Aim::All, Times::UntilCleared),
        ];
        assert_eq!(take_all(&mut both, None), vec![4, 5]);
        assert_eq!(take_all(&mut both, None), vec![5]);
    }
}

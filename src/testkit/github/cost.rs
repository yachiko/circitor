//! GitHub's cost formula (docs/02 §2), and `describe`.

use apollo_compiler::executable::SelectionSet;

use super::query::{Prepared, prepare, sub_sets};
use super::{Asked, AskedOf, Rejected, valid_schema};

/// What a query asks, by GitHub's formula: the requests its connections need and the nodes they
/// can return. Both follow the page size asked for, never the rows returned (docs/02 §2).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct Measure {
    pub requests: u64,
    pub nodes: u64,
}

impl Measure {
    /// TST-3: divide by 100, round (half up: whether exactly half a point rounds up isn't measured,
    /// and this is the one place that decides it), at least 1.
    pub fn points(self) -> u32 {
        let points = self.requests.saturating_add(50) / 100;
        u32::try_from(points).unwrap_or(u32::MAX).max(1)
    }
}

/// TST-3: the cost of a request (the POST body) by GitHub's formula, without answering it: for the
/// queries the fake doesn't answer (`search`, `timelineItems`) as much as for those it does.
pub fn cost_of(request: &[u8]) -> Result<u32, Rejected> {
    let prep = prepare(request)?;
    Ok(measure(&prep)?.points())
}

/// TST-3: one request for each top-level connection, plus, for each parent node asked for (`first`
/// or `last`, or the number of IDs given), one for each connection nested in it. A connection with
/// neither `first` nor `last` isn't counted.
pub(super) fn measure(prep: &Prepared) -> Result<Measure, Rejected> {
    let mut m = Measure::default();
    walk(prep, &[&prep.op.selection_set], 1, &mut m)?;
    Ok(m)
}

fn walk(
    prep: &Prepared,
    sets: &[&SelectionSet],
    mult: u64,
    m: &mut Measure,
) -> Result<(), Rejected> {
    // Every fragment counts, whatever its type: GitHub costs the query as if every page were full.
    for (_, fields) in prep.collect(sets, &|_| true) {
        let f = fields[0];
        if f.name == "__typename" {
            continue;
        }
        let def = &f.definition;
        let subs = sub_sets(&fields);
        let is_connection = def.ty.inner_named_type().as_str().ends_with("Connection")
            && def.argument_by_name("first").is_some();
        let child = if is_connection {
            let first = prep.arg_i64(f, "first");
            let last = prep.arg_i64(f, "last");
            match first.or(last) {
                Some(n) => {
                    // docs/02 §2: `first`/`last` are 1–100.
                    if !(1..=100).contains(&n) {
                        return Err(Rejected::NotInSchema(format!(
                            "`{}` asks for {n} records; `first` and `last` are 1 to 100",
                            f.response_key()
                        )));
                    }
                    let n = n.unsigned_abs();
                    m.requests = m.requests.saturating_add(mult);
                    m.nodes = m.nodes.saturating_add(mult.saturating_mul(n));
                    mult.saturating_mul(n)
                }
                None => mult,
            }
        } else if f.name == "nodes" && def.argument_by_name("ids").is_some() {
            // `nodes(ids:)` isn't a connection, but each ID is a parent (docs/02 §2).
            let n = prep
                .arg(f, "ids")
                .and_then(|v| v.as_array().map(Vec::len))
                .unwrap_or(0) as u64;
            m.nodes = m.nodes.saturating_add(mult.saturating_mul(n));
            mult.saturating_mul(n)
        } else if f.name == "node" && def.argument_by_name("id").is_some() {
            m.nodes = m.nodes.saturating_add(mult);
            mult
        } else {
            mult
        };
        walk(prep, &subs, child, m)?;
    }
    Ok(())
}

/// The fields of the PR record: asking any of them is asking for the record.
const RECORD_FIELDS: &[&str] = &[
    "state",
    "headRefOid",
    "mergeable",
    "mergeStateStatus",
    "isDraft",
    "baseRefName",
    "headRefName",
    "title",
];

/// What a request asks, for T20's request log (`plan/contracts/testkit.md`).
pub fn describe(request: &[u8]) -> Result<Asked, Rejected> {
    let prep = prepare(request)?;
    let mut asked = Asked {
        finding: None,
        nodes: Vec::new(),
        lookup: None,
    };
    for (_, fields) in prep.collect(&[&prep.op.selection_set], &|_| true) {
        let f = fields[0];
        let subs = sub_sets(&fields);
        match f.name.as_str() {
            "viewer" => {
                for (_, inner) in prep.collect(&subs, &|_| true) {
                    if inner[0].name == "pullRequests" {
                        asked.finding = Some(prep.arg_str(inner[0], "after"));
                    }
                }
            }
            "nodes" => {
                let of = asked_of(&prep, &subs);
                let ids = prep.arg(f, "ids").and_then(|v| v.as_array().cloned());
                for id in ids.unwrap_or_default() {
                    if let Some(id) = id.as_str() {
                        asked.nodes.push((id.to_owned(), of));
                    }
                }
            }
            "node" => {
                if let Some(id) = prep.arg_str(f, "id") {
                    asked.nodes.push((id, asked_of(&prep, &subs)));
                }
            }
            "repository" => {
                let owner = prep.arg_str(f, "owner").unwrap_or_default();
                let name = prep.arg_str(f, "name").unwrap_or_default();
                for (_, inner) in prep.collect(&subs, &|_| true) {
                    if inner[0].name == "pullRequest" {
                        let number = prep
                            .arg_i64(inner[0], "number")
                            .map_or(0, i64::unsigned_abs);
                        asked.lookup = Some((owner.clone(), name.clone(), number));
                    }
                }
            }
            _ => {}
        }
    }
    Ok(asked)
}

fn asked_of(prep: &Prepared, sets: &[&SelectionSet]) -> AskedOf {
    let schema = valid_schema();
    let on_pr = |t: &str| t == "PullRequest" || schema.is_subtype(t, "PullRequest");
    let fields = prep.collect(sets, &on_pr);
    let record = fields
        .iter()
        .any(|(_, fs)| RECORD_FIELDS.contains(&fs[0].name.as_str()));
    let items = fields.iter().any(|(_, fs)| {
        let f = fs[0];
        matches!(f.name.as_str(), "comments" | "reviews")
            && (prep.arg(f, "first").is_some() || prep.arg(f, "last").is_some())
    });
    let checks_page = reaches_paged_contexts(prep, sets);
    match (record, items) {
        (true, true) => AskedOf::RecordAndItems,
        (true, false) => AskedOf::Record,
        (false, true) => AskedOf::Items,
        (false, false) if checks_page => AskedOf::ChecksPage,
        (false, false) => AskedOf::Record,
    }
}

/// Whether the selection reaches `contexts(after:)`: a further page of checks (POL-8).
fn reaches_paged_contexts(prep: &Prepared, sets: &[&SelectionSet]) -> bool {
    prep.collect(sets, &|_| true).iter().any(|(_, fs)| {
        let f = fs[0];
        (f.name == "contexts" && prep.arg(f, "after").is_some())
            || reaches_paged_contexts(prep, &sub_sets(fs))
    })
}

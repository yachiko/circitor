//! The builder: one `Query` becomes one GraphQL `query` document and its variables (POL-1).
//!
//! POL-1: this is the only code in circitor that writes a GraphQL document, and it only ever writes
//! one `query` operation. Hand-written strings with shared fragments; no code generation.

use serde_json::{Map, Value, json};

use crate::github::{Query, Want};

/// Where one `NodeQuery`'s answer sits in the response: under `alias`, at `index` in a `nodes(ids:)`
/// list, or the whole field for a `node(id:)` (a check page, which has its own cursor).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Slot {
    pub alias: String,
    pub index: Option<usize>,
}

/// A built request: the document, its variables, and where each node's answer will be.
pub(crate) struct Plan {
    pub document: String,
    pub variables: Map<String, Value>,
    /// One per `Query::nodes`, in order.
    pub slots: Vec<Slot>,
}

/// The aliases of the three `nodes(ids:)` groups.
pub(crate) const RECORD: &str = "record";
pub(crate) const RECORD_AND_ITEMS: &str = "recordAndItems";
pub(crate) const ITEMS: &str = "items";

// POL-6, MOD-2, MOD-3: every GitHub field of MOD-2's table, the repository's owner and name, the
// three counts, and the last commit's rollup with `contexts(first: 100)`. No comment or review IDs
// (POL-12). The counts are aliased: `comments(last: 20)` beside `comments { totalCount }` under one
// key doesn't validate. MOD-3 (D6): no query asks `isRequired`.
const RECORD_FRAGMENT: &str = "fragment Record on PullRequest {
  id number url title state isDraft createdAt updatedAt mergedAt closedAt
  repository { nameWithOwner }
  author { login }
  baseRefName headRefName headRefOid
  mergeable mergeStateStatus reviewDecision
  commentCount: comments { totalCount }
  reviewCount: reviews { totalCount }
  reviewThreads { totalCount }
  commits(last: 1) { nodes { commit { statusCheckRollup { state contexts(first: 100) { ...Contexts } } } } }
}
";

// MOD-3: the fields of a check run (its `databaseId` included) and of a commit status.
const CONTEXTS_FRAGMENT: &str = "fragment Contexts on StatusCheckRollupContextConnection {
  totalCount
  pageInfo { hasNextPage endCursor }
  nodes {
    __typename
    ... on CheckRun { databaseId name status conclusion detailsUrl }
    ... on StatusContext { context state targetUrl }
  }
}
";

// POL-12, MOD-12: the last 20 comments and the last 20 reviews, who, when and an ID; a review's
// state and its inline comment count. Aliased, as the counts are.
const ITEMS_FRAGMENT: &str = "fragment Items on PullRequest {
  lastComments: comments(last: 20) { nodes { databaseId author { login } createdAt } }
  lastReviews: reviews(last: 20) { nodes { databaseId author { login } state submittedAt comments { totalCount } } }
}
";

/// The POST body for `query`: `{"query": …, "variables": …}`.
pub(crate) fn body(plan: &Plan) -> Value {
    json!({ "query": plan.document, "variables": Value::Object(plan.variables.clone()) })
}

/// POL-1: builds the one `query` operation a `Query` describes.
pub(crate) fn plan(query: &Query) -> Plan {
    let mut declared: Vec<String> = Vec::new();
    let mut variables = Map::new();
    let mut fields: Vec<String> = Vec::new();
    let (mut uses_record, mut uses_items, mut uses_contexts) = (false, false, false);

    // Every query asks the rate-limit block.
    fields.push("rateLimit { cost remaining resetAt }".to_string());

    // POL-4's finding.
    if let Some(find) = &query.find {
        let after = match &find.after {
            Some(cursor) => {
                declared.push("$findAfter: String".to_string());
                variables.insert("findAfter".into(), Value::String(cursor.clone()));
                ", after: $findAfter"
            }
            None => "",
        };
        fields.push(format!(
            "viewer {{ login pullRequests(states: OPEN, first: 100{after}, orderBy: {{field: UPDATED_AT, direction: DESC}}) {{ pageInfo {{ hasNextPage endCursor }} nodes {{ id number updatedAt repository {{ nameWithOwner }} }} }} }}"
        ));
    }

    // POL-4, POL-7: by node ID, one shared fragment per want, each want under its own alias.
    let mut slots = Vec::with_capacity(query.nodes.len());
    let mut groups: [(&'static str, Vec<Value>); 3] = [
        (RECORD, vec![]),
        (RECORD_AND_ITEMS, vec![]),
        (ITEMS, vec![]),
    ];
    let mut pages = 0usize;
    for node in &query.nodes {
        let group = match &node.want {
            Want::Record => 0,
            Want::RecordAndItems => 1,
            Want::Items => 2,
            Want::ChecksPage { after } => {
                // POL-8: a check page has its own cursor, so it's its own aliased field.
                let alias = format!("checks{pages}");
                declared.push(format!("${alias}: ID!"));
                declared.push(format!("${alias}After: String"));
                variables.insert(alias.clone(), Value::String(node.id.0.clone()));
                variables.insert(format!("{alias}After"), Value::String(after.clone()));
                fields.push(format!(
                    "{alias}: node(id: ${alias}) {{ __typename ... on PullRequest {{ commits(last: 1) {{ nodes {{ commit {{ statusCheckRollup {{ state contexts(first: 100, after: ${alias}After) {{ ...Contexts }} }} }} }} }} }} }}"
                ));
                uses_contexts = true;
                slots.push(Slot { alias, index: None });
                pages += 1;
                continue;
            }
        };
        let (alias, ids) = &mut groups[group];
        slots.push(Slot {
            alias: alias.to_string(),
            index: Some(ids.len()),
        });
        ids.push(Value::String(node.id.0.clone()));
    }
    for (alias, ids) in groups {
        if ids.is_empty() {
            continue;
        }
        let spreads = match alias {
            RECORD => {
                uses_record = true;
                "...Record"
            }
            RECORD_AND_ITEMS => {
                uses_record = true;
                uses_items = true;
                "...Record ...Items"
            }
            _ => {
                uses_items = true;
                "...Items"
            }
        };
        declared.push(format!("${alias}: [ID!]!"));
        variables.insert(alias.to_string(), Value::Array(ids));
        fields.push(format!(
            "{alias}: nodes(ids: ${alias}) {{ __typename {spreads} }}"
        ));
    }

    // POL-16: reading through, as `RecordAndItems`. Not aliased: GitHub's error paths name
    // `repository` and `pullRequest` (docs/02 §5).
    if let Some(name) = &query.lookup {
        declared.push("$owner: String!".to_string());
        declared.push("$name: String!".to_string());
        declared.push("$number: Int!".to_string());
        variables.insert("owner".into(), Value::String(name.owner.clone()));
        variables.insert("name".into(), Value::String(name.repo.clone()));
        variables.insert("number".into(), Value::from(name.number));
        fields.push(
            "repository(owner: $owner, name: $name) { pullRequest(number: $number) { __typename ...Record ...Items } }"
                .to_string(),
        );
        uses_record = true;
        uses_items = true;
    }

    let mut document = String::from("query");
    if !declared.is_empty() {
        document.push('(');
        document.push_str(&declared.join(", "));
        document.push(')');
    }
    document.push_str(" {\n");
    for field in &fields {
        document.push_str("  ");
        document.push_str(field);
        document.push('\n');
    }
    document.push_str("}\n");
    if uses_record {
        document.push_str(RECORD_FRAGMENT);
        uses_contexts = true;
    }
    if uses_items {
        document.push_str(ITEMS_FRAGMENT);
    }
    if uses_contexts {
        document.push_str(CONTEXTS_FRAGMENT);
    }

    Plan {
        document,
        variables,
        slots,
    }
}

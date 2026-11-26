//! Parsing a POST body, checking it against the schema, and the helpers the executor, the cost and
//! `describe` share: argument values with variables, `@skip`/`@include`, and field collection.

use apollo_compiler::ast;
use apollo_compiler::executable::{DirectiveList, Field, Operation, Selection, SelectionSet};
use apollo_compiler::validation::Valid;
use apollo_compiler::{ExecutableDocument, Name, Node};
use serde_json::{Map, Value};

use super::{Rejected, valid_schema};

/// A request parsed and checked: the document, the operation to run and its variables.
pub(super) struct Prepared {
    pub doc: Valid<ExecutableDocument>,
    pub op: Node<Operation>,
    pub vars: Map<String, Value>,
}

/// The fields of a selection, grouped by response key in the order first seen (GraphQL's
/// CollectFields): every field under one key is merged into one answer.
pub(super) type Grouped<'d> = Vec<(&'d Name, Vec<&'d Node<Field>>)>;

/// Parses `request` (`{"query": …, "variables": …, "operationName": …}`) and checks it.
pub(super) fn prepare(request: &[u8]) -> Result<Prepared, Rejected> {
    // TST-3: it parses each query and checks it against GitHub's public schema.
    let body: Value = serde_json::from_slice(request)
        .map_err(|e| Rejected::Unparsable(format!("the body isn't JSON: {e}")))?;
    let text = body
        .get("query")
        .and_then(Value::as_str)
        .ok_or_else(|| Rejected::Unparsable("the body has no `query` string".into()))?;
    let vars = match body.get("variables") {
        None | Some(Value::Null) => Map::new(),
        Some(Value::Object(m)) => m.clone(),
        Some(_) => return Err(Rejected::Unparsable("`variables` isn't an object".into())),
    };
    let op_name = body.get("operationName").and_then(Value::as_str);

    // Syntax first, on the AST alone: a document that doesn't parse is `Unparsable`.
    let ast = ast::Document::parse(text, "request.graphql")
        .map_err(|e| Rejected::Unparsable(messages(&e.errors)))?;
    // POL-1: any mutation is refused as one, valid or not.
    let mutation = ast.definitions.iter().any(|d| {
        matches!(d, ast::Definition::OperationDefinition(op)
            if op.operation_type == ast::OperationType::Mutation)
    });
    if mutation {
        return Err(Rejected::Mutation);
    }
    let doc = ExecutableDocument::parse_and_validate(valid_schema(), text, "request.graphql")
        .map_err(|e| Rejected::NotInSchema(messages(&e.errors)))?;
    let op = doc
        .operations
        .get(op_name)
        .map_err(|e| Rejected::Unparsable(e.message().to_string()))?
        .clone();
    if op.operation_type != ast::OperationType::Query {
        return Err(Rejected::NotInSchema(format!(
            "a {} isn't a query",
            op.operation_type
        )));
    }
    // A required variable left out is refused, as GitHub refuses it.
    for def in &op.variables {
        let given = vars.get(def.name.as_str()).is_some_and(|v| !v.is_null());
        if def.ty.is_non_null() && def.default_value.is_none() && !given {
            return Err(Rejected::NotInSchema(format!(
                "variable `${}` of type `{}` was not provided",
                def.name, def.ty
            )));
        }
    }
    Ok(Prepared { doc, op, vars })
}

fn messages(list: &apollo_compiler::validation::DiagnosticList) -> String {
    let all: Vec<String> = list.iter().map(|d| d.error.to_string()).collect();
    all.join("; ")
}

impl Prepared {
    /// An argument's value as JSON, variables substituted; `None` when it isn't given (nor its
    /// variable, nor that variable's default).
    pub fn arg(&self, field: &Field, name: &str) -> Option<Value> {
        let v = field.specified_argument_by_name(name)?;
        let json = self.value(v);
        if json.is_null() { None } else { Some(json) }
    }

    pub fn arg_i64(&self, field: &Field, name: &str) -> Option<i64> {
        self.arg(field, name).and_then(|v| v.as_i64())
    }

    pub fn arg_str(&self, field: &Field, name: &str) -> Option<String> {
        self.arg(field, name)
            .and_then(|v| v.as_str().map(str::to_owned))
    }

    pub fn value(&self, v: &ast::Value) -> Value {
        match v {
            ast::Value::Null => Value::Null,
            ast::Value::Enum(n) => Value::String(n.to_string()),
            ast::Value::Variable(n) => self.var(n),
            ast::Value::String(s) => Value::String(s.clone()),
            ast::Value::Float(f) => f
                .try_to_f64()
                .ok()
                .and_then(serde_json::Number::from_f64)
                .map_or(Value::Null, Value::Number),
            ast::Value::Int(i) => i
                .as_str()
                .parse::<i64>()
                .map_or(Value::Null, |n| Value::Number(n.into())),
            ast::Value::Boolean(b) => Value::Bool(*b),
            ast::Value::List(items) => Value::Array(items.iter().map(|i| self.value(i)).collect()),
            ast::Value::Object(fields) => Value::Object(
                fields
                    .iter()
                    .map(|(k, v)| (k.to_string(), self.value(v)))
                    .collect(),
            ),
        }
    }

    fn var(&self, name: &Name) -> Value {
        if let Some(v) = self.vars.get(name.as_str()) {
            return v.clone();
        }
        self.op
            .variables
            .iter()
            .find(|d| d.name == *name)
            .and_then(|d| d.default_value.as_ref())
            .map_or(Value::Null, |d| self.value(d))
    }

    /// `@skip(if:)` and `@include(if:)`.
    pub fn included(&self, directives: &DirectiveList) -> bool {
        let flag = |name: &str| {
            directives
                .get(name)
                .and_then(|d| d.specified_argument_by_name("if"))
                .map(|v| self.value(v).as_bool().unwrap_or(false))
        };
        flag("skip") != Some(true) && flag("include") != Some(false)
    }

    /// GraphQL's CollectFields over several selection sets (the merged sub-selections of one
    /// response key). `applies(type_condition)` says whether a fragment applies to the object.
    pub fn collect<'d>(
        &'d self,
        sets: &[&'d SelectionSet],
        applies: &dyn Fn(&str) -> bool,
    ) -> Grouped<'d> {
        let mut out: Grouped<'d> = Vec::new();
        for set in sets {
            self.collect_into(set, applies, &mut out);
        }
        out
    }

    fn collect_into<'d>(
        &'d self,
        set: &'d SelectionSet,
        applies: &dyn Fn(&str) -> bool,
        out: &mut Grouped<'d>,
    ) {
        for sel in &set.selections {
            match sel {
                Selection::Field(f) => {
                    if !self.included(&f.directives) {
                        continue;
                    }
                    let key = f.response_key();
                    match out.iter_mut().find(|(k, _)| *k == key) {
                        Some((_, fields)) => fields.push(f),
                        None => out.push((key, vec![f])),
                    }
                }
                Selection::InlineFragment(inline) => {
                    if !self.included(&inline.directives) {
                        continue;
                    }
                    let ok = inline
                        .type_condition
                        .as_ref()
                        .is_none_or(|t| applies(t.as_str()));
                    if ok {
                        self.collect_into(&inline.selection_set, applies, out);
                    }
                }
                Selection::FragmentSpread(spread) => {
                    if !self.included(&spread.directives) {
                        continue;
                    }
                    if let Some(frag) = self.doc.fragments.get(&spread.fragment_name)
                        && applies(frag.type_condition().as_str())
                    {
                        self.collect_into(&frag.selection_set, applies, out);
                    }
                }
            }
        }
    }
}

/// The sub-selections of every field under one key.
pub(super) fn sub_sets<'d>(fields: &[&'d Node<Field>]) -> Vec<&'d SelectionSet> {
    fields.iter().map(|f| &f.selection_set).collect()
}

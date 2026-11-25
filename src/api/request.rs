#![allow(unused_variables, dead_code)] // stub: T43 removes this
//! `Request`'s spelling of the routes (T43).

use super::types::Request;

impl Request {
    pub fn method(&self) -> http::Method {
        unimplemented!("T43")
    }

    /// `/v1/prs?state=all&since=…`, `/v1/prs/{owner}/{repo}/{number}/wait?until=…&timeout=…` and so on,
    /// exactly as 04-api writes them; a flag is sent only when set, as `1`. A `wait`'s query holds `until`,
    /// then `sha`, `rerun_since`, `fail_fast`, `kind` and `since` as set, then `timeout` and `limit`.
    /// `NotifyCancel` is `DELETE /v1/prs/{owner}/{repo}/{number}/notify?socket=…`.
    pub fn path_and_query(&self) -> String {
        unimplemented!("T43")
    }

    /// `track`: `{"pr":"owner/repo#12"}`. `notify`: one JSON object with `until`, then `sha`, `rerun_since`,
    /// `fail_fast`, `kind`, `since`, `for` and `limit` only as set, then `socket` and `token`. In a body
    /// `fail_fast` is a JSON boolean and `kind` one comma-separated string (NTF-2).
    pub fn body(&self) -> Option<String> {
        unimplemented!("T43")
    }
}

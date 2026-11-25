#![allow(unused_variables, dead_code)] // stub: T43 removes this
//! `ScriptedGitHub` and `answers` (T43).

use std::sync::{Arc, Mutex};

use crate::clock::Activity;
use crate::token::Token;

use super::{Answer, Failure, GitHub, Query};

type Handler = Box<dyn FnMut(&Query) -> Result<Answer, Failure> + Send + 'static>;

/// A `GitHub` that answers by script. It doesn't check queries against a schema, compute costs,
/// or enforce POL-2: the fake GitHub server (T14, T20) does.
pub struct ScriptedGitHub {
    handler: Mutex<Handler>,
    calls: Mutex<Vec<Query>>,
}

impl ScriptedGitHub {
    pub fn new(
        handler: impl FnMut(&Query) -> Result<Answer, Failure> + Send + 'static,
    ) -> Arc<ScriptedGitHub> {
        unimplemented!("T43")
    }

    /// Every query sent, in order.
    pub fn calls(&self) -> Vec<Query> {
        unimplemented!("T43")
    }

    /// The next `send` waits until the returned `Release` is dropped, and is excused from the work in
    /// flight meanwhile (`clock.md`).
    pub fn hold(&self, activity: &Activity) -> Release {
        unimplemented!("T43")
    }
}

#[async_trait::async_trait]
impl GitHub for ScriptedGitHub {
    async fn send(&self, token: &Token, query: &Query) -> Result<Answer, Failure> {
        unimplemented!("T43")
    }
}

/// Dropping it lets the held `send` answer.
pub struct Release {
    inner: Option<tokio::sync::oneshot::Sender<()>>,
}

/// Helpers that build plain values for tests.
pub mod answers {
    use crate::github::{Answer, NodeAnswer, Rate};
    use crate::model::PrRead;

    /// A plain open PR read, named `name` (`"o/r#1"`).
    pub fn pr_read(name: &str) -> PrRead {
        unimplemented!("T43")
    }

    /// An answer holding `nodes`, with a plain rate block.
    pub fn answer_with(nodes: Vec<NodeAnswer>) -> Answer {
        unimplemented!("T43")
    }

    /// A rate block.
    pub fn rate(cost: u32, remaining: u32) -> Rate {
        unimplemented!("T43")
    }
}

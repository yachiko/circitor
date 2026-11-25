#![allow(unused_variables, dead_code)] // stub: T43 removes this
//! `FakeTokenSource` (T43).

use std::sync::Arc;
use std::time::Duration;

use crate::clock::SharedClock;

use super::{TokenRead, TokenSource, TokenSourceKind};

/// Its default read is `Token(MARKER_TOKEN)`; `kind()` is `Gh` and `gh_path()` is `gh` unless set.
pub struct FakeTokenSource {
    clock: SharedClock,
}

impl FakeTokenSource {
    pub fn new(clock: SharedClock) -> Arc<FakeTokenSource> {
        unimplemented!("T43")
    }

    /// What every read gives from now on.
    pub fn set(&self, read: TokenRead) {
        unimplemented!("T43")
    }

    /// Each read takes `delay` of the clock.
    pub fn set_delay(&self, delay: Duration) {
        unimplemented!("T43")
    }

    pub fn set_kind(&self, kind: TokenSourceKind) {
        unimplemented!("T43")
    }

    pub fn set_gh_path(&self, gh_path: &str) {
        unimplemented!("T43")
    }

    /// How many reads have begun.
    pub fn reads(&self) -> usize {
        unimplemented!("T43")
    }
}

#[async_trait::async_trait]
impl TokenSource for FakeTokenSource {
    async fn read(&self) -> TokenRead {
        unimplemented!("T43")
    }

    fn kind(&self) -> TokenSourceKind {
        unimplemented!("T43")
    }

    fn gh_path(&self) -> String {
        unimplemented!("T43")
    }
}

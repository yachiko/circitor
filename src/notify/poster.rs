#![allow(unused_variables, dead_code)] // stub: T46 removes this
//! The real poster: `tokio::fs::metadata`, `tokio::net::UnixStream` (NTF-6; T46).

use std::path::Path;
use std::sync::Arc;

use super::{Looked, Posted, Poster, SessionToken};

pub struct RealPoster;

impl RealPoster {
    pub fn new() -> Arc<RealPoster> {
        unimplemented!("T46")
    }
}

#[async_trait::async_trait]
impl Poster for RealPoster {
    async fn look(&self, socket: &Path) -> Looked {
        unimplemented!("T46")
    }

    async fn post(&self, socket: &Path, token: Option<&SessionToken>, text: &str) -> Posted {
        unimplemented!("T46")
    }
}

//! `WarpServerAdapter` — implements `AiProvider` by delegating to Warp's
//! hosted multi-agent endpoint via `ServerApi::generate_multi_agent_output_via_warp`.
//! See `crates/ai_provider` for the trait this adapter implements.

use std::sync::Arc;

use ai_provider::{AIApiError, AiProvider, ResponseEventStream};
use warp_multi_agent_api::Request;

use crate::server::server_api::ServerApi;

/// An `AiProvider` that calls Warp's hosted `/ai/multi-agent` endpoint.
///
/// Holds `Arc<ServerApi>` because the existing implementation depends on
/// the server API's HTTP client, auth manager, and other internal state.
/// This is a thin wrapper — the real work happens in
/// [`ServerApi::generate_multi_agent_output_via_warp`].
pub struct WarpServerAdapter {
    server_api: Arc<ServerApi>,
}

impl WarpServerAdapter {
    pub fn new(server_api: Arc<ServerApi>) -> Self {
        Self { server_api }
    }
}

#[async_trait::async_trait]
impl AiProvider for WarpServerAdapter {
    async fn chat_stream(
        &self,
        request: &Request,
    ) -> std::result::Result<ResponseEventStream, Arc<AIApiError>> {
        self.server_api
            .generate_multi_agent_output_via_warp(request)
            .await
    }
}

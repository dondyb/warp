//! `AiProvider` trait — the network-boundary abstraction for Warp's AI calls.

use std::sync::Arc;

use futures::stream::BoxStream;
use warp_multi_agent_api::{Request, ResponseEvent};

use crate::AIApiError;

/// A single streamed response from the AI backend, threaded through `Arc`
/// so downstream event handlers can clone errors. Mirrors the existing
/// `AIOutputStream` alias in `app/src/server/server_api.rs` (kept here in
/// trait-friendly form so the alias can be removed in a follow-up).
pub type ResponseEventStream =
    BoxStream<'static, std::result::Result<ResponseEvent, Arc<AIApiError>>>;

/// A client that turns a Warp `Request` into a stream of `ResponseEvent`s.
///
/// Implementations:
/// - `WarpServerAdapter` (M1b-prep): delegates to Warp's hosted backend via
///   the existing HTTP/SSE path — used when `WARP_AI_PROTOCOL` is unset.
/// - `OpenAiAdapter` (M1b-chat): translates to OpenAI Chat Completions.
/// - `AnthropicAdapter` (M2): translates to Anthropic Messages.
///
/// The outer `Result` represents setup/auth failures before any stream
/// begins. The inner per-event `Result<_, Arc<AIApiError>>` represents
/// per-event decode or transport failures during streaming.
#[async_trait::async_trait]
pub trait AiProvider: Send + Sync {
    async fn chat_stream(
        &self,
        request: &Request,
    ) -> std::result::Result<ResponseEventStream, Arc<AIApiError>>;
}

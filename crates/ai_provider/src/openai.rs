//! OpenAI Chat Completions adapter. Translates Warp's internal protobuf
//! request/response into OpenAI's HTTP+SSE protocol and back.
//!
//! Configured via env vars:
//! - `WARP_AI_OPENAI_ENDPOINT` — base URL (default `https://api.openai.com/v1`)
//! - `WARP_AI_OPENAI_API_KEY` — bearer token (required)
//! - `WARP_AI_OPENAI_MODEL` — model id (required; e.g. `gpt-4o-mini`)
//!
//! M1b-chat: text-only chat (no tool calls). Tools land in M1c.

use std::sync::Arc;

use crate::AIApiError;

/// OpenAI provider configuration loaded from environment variables.
#[derive(Debug, Clone)]
pub struct OpenAiConfig {
    /// Base URL — e.g. `https://api.openai.com/v1` or
    /// `http://localhost:11434/v1` for Ollama.
    pub endpoint: String,
    /// Bearer token for `Authorization` header.
    pub api_key: String,
    /// Model identifier (e.g. `gpt-4o-mini`, `claude-3-5-sonnet`,
    /// `llama3.2`) — sent as the `model` field in the chat completions request.
    pub model: String,
}

impl OpenAiConfig {
    /// Default endpoint when `WARP_AI_OPENAI_ENDPOINT` is unset.
    pub const DEFAULT_ENDPOINT: &'static str = "https://api.openai.com/v1";

    /// Build an `OpenAiConfig` from environment variables. Returns `Err` if
    /// the API key or model is missing — without those, the adapter cannot
    /// authenticate or know which model to call.
    pub fn from_env() -> std::result::Result<Self, Arc<AIApiError>> {
        let api_key = std::env::var("WARP_AI_OPENAI_API_KEY").map_err(|_| {
            Arc::new(AIApiError::Other(anyhow::anyhow!(
                "WARP_AI_PROTOCOL=openai requires WARP_AI_OPENAI_API_KEY"
            )))
        })?;
        let model = std::env::var("WARP_AI_OPENAI_MODEL").map_err(|_| {
            Arc::new(AIApiError::Other(anyhow::anyhow!(
                "WARP_AI_PROTOCOL=openai requires WARP_AI_OPENAI_MODEL"
            )))
        })?;
        let endpoint = std::env::var("WARP_AI_OPENAI_ENDPOINT")
            .unwrap_or_else(|_| Self::DEFAULT_ENDPOINT.to_string());
        Ok(Self {
            endpoint,
            api_key,
            model,
        })
    }
}

use serde_json::json;
use warp_multi_agent_api::{Request, request as req};

/// `AiProvider` impl that translates Warp's protobuf request to/from the
/// OpenAI Chat Completions HTTP API.
pub struct OpenAiAdapter {
    config: OpenAiConfig,
    /// Lazily-initialised HTTP client. `None` until first access so that
    /// unit tests (which only exercise `build_request_body`) do not trigger
    /// TLS provider initialisation.
    client: std::sync::OnceLock<reqwest::Client>,
}

impl OpenAiAdapter {
    /// Build an adapter from environment variables.
    pub fn from_env() -> std::result::Result<Self, Arc<AIApiError>> {
        Ok(Self::new(OpenAiConfig::from_env()?))
    }

    pub fn new(config: OpenAiConfig) -> Self {
        Self {
            config,
            client: std::sync::OnceLock::new(),
        }
    }

    /// Return a reference to the shared HTTP client, initialising it on first
    /// call.
    #[allow(dead_code)]
    fn client(&self) -> &reqwest::Client {
        self.client.get_or_init(reqwest::Client::new)
    }

    /// Build the OpenAI Chat Completions JSON body from a Warp `Request`.
    /// MVP: extracts the user query (UserQuery only — other input variants
    /// produce a "not supported" error), wraps in a system + user messages
    /// array, and sets the configured model with `stream: true`.
    pub(crate) fn build_request_body(
        &self,
        request: &Request,
    ) -> std::result::Result<serde_json::Value, Arc<AIApiError>> {
        let user_text = extract_user_query(request)?;
        Ok(json!({
            "model": self.config.model,
            "stream": true,
            "messages": [
                {
                    "role": "system",
                    "content": SYSTEM_PROMPT
                },
                {
                    "role": "user",
                    "content": user_text
                }
            ]
        }))
    }

    /// Extract a text delta from an OpenAI streaming response chunk.
    /// Returns `Ok(Some(text))` for content deltas, `Ok(None)` for chunks
    /// without content (role-only first chunk, finish_reason-only last
    /// chunk, etc.), or `Err` if the JSON cannot be parsed.
    pub(crate) fn extract_text_delta(
        &self,
        chunk_json: &str,
    ) -> std::result::Result<Option<String>, Arc<AIApiError>> {
        let v: serde_json::Value = serde_json::from_str(chunk_json).map_err(|e| {
            Arc::new(AIApiError::Other(anyhow::anyhow!(
                "OpenAI adapter: failed to parse SSE chunk JSON: {e:#}"
            )))
        })?;
        // Standard OpenAI shape: { "choices": [{ "delta": { "content": "..." } }] }
        let content = v
            .pointer("/choices/0/delta/content")
            .and_then(|c| c.as_str())
            .map(|s| s.to_string());
        Ok(content)
    }
}

/// Minimal system prompt for M1b-chat. Keeps the model in "helpful coding
/// assistant" mode without claiming Warp-specific tool capabilities (since
/// tools land in M1c). Future plans may expand this from `Request.task_context`
/// + `Request.settings` to better mirror Warp's hosted prompt.
const SYSTEM_PROMPT: &str = "You are a helpful AI assistant integrated into a \
    terminal application. Respond clearly and concisely. Do not invoke tools \
    or external commands — just provide a textual answer.";

/// Extract the user's most recent query from a Warp `Request`.
/// Looks at `Input::user_inputs.user_query` (the current path) and the
/// deprecated `Input::user_query`. Other input variants produce an error
/// because M1b-chat does not yet support them.
pub(crate) fn extract_user_query(
    request: &Request,
) -> std::result::Result<String, Arc<AIApiError>> {
    let Some(input) = request.input.as_ref() else {
        return Err(Arc::new(AIApiError::Other(anyhow::anyhow!(
            "OpenAI adapter: Request.input is missing"
        ))));
    };
    match input.r#type.as_ref() {
        Some(req::input::Type::UserInputs(user_inputs)) => {
            // Find the most recent UserQuery in the inputs list.
            let query = user_inputs.inputs.iter().rev().find_map(|ui| match ui.input.as_ref() {
                Some(req::input::user_inputs::user_input::Input::UserQuery(uq)) => {
                    Some(uq.query.clone())
                }
                _ => None,
            });
            query.ok_or_else(|| {
                Arc::new(AIApiError::Other(anyhow::anyhow!(
                    "OpenAI adapter: UserInputs contained no UserQuery (other input variants \
                     such as ToolCallResult are not yet supported in M1b-chat)"
                )))
            })
        }
        Some(req::input::Type::UserQuery(uq)) => Ok(uq.query.clone()),
        Some(other) => Err(Arc::new(AIApiError::Other(anyhow::anyhow!(
            "OpenAI adapter: input variant {other:?} is not yet supported \
             (planned for M1c+)"
        )))),
        None => Err(Arc::new(AIApiError::Other(anyhow::anyhow!(
            "OpenAI adapter: Request.input.type is missing"
        )))),
    }
}

use warp_multi_agent_api::{ResponseEvent, Task, Message};
use warp_multi_agent_api::response_event;
use warp_multi_agent_api::client_action;
use warp_multi_agent_api::message;

/// A fresh set of synthesized IDs for an OpenAI streaming response.
/// Same shape as the IDs Warp's hosted backend would issue, but
/// generated locally because OpenAI doesn't return them.
pub(crate) struct StreamIds {
    pub conversation_id: String,
    pub request_id: String,
    pub run_id: String,
    pub task_id: String,
    pub message_id: String,
}

impl StreamIds {
    pub(crate) fn new() -> Self {
        Self {
            conversation_id: uuid::Uuid::new_v4().to_string(),
            request_id: uuid::Uuid::new_v4().to_string(),
            run_id: uuid::Uuid::new_v4().to_string(),
            task_id: uuid::Uuid::new_v4().to_string(),
            message_id: uuid::Uuid::new_v4().to_string(),
        }
    }
}

/// Build the `StreamInit` ResponseEvent.
pub(crate) fn build_stream_init(ids: &StreamIds) -> ResponseEvent {
    ResponseEvent {
        r#type: Some(response_event::Type::Init(
            response_event::StreamInit {
                conversation_id: ids.conversation_id.clone(),
                request_id: ids.request_id.clone(),
                run_id: ids.run_id.clone(),
            },
        )),
    }
}

/// Build a `ClientActions` ResponseEvent containing the given actions.
pub(crate) fn build_client_actions(
    actions: Vec<warp_multi_agent_api::ClientAction>,
) -> ResponseEvent {
    ResponseEvent {
        r#type: Some(response_event::Type::ClientActions(
            response_event::ClientActions { actions },
        )),
    }
}

/// Build a `BeginTransaction` action.
pub(crate) fn action_begin_transaction() -> warp_multi_agent_api::ClientAction {
    warp_multi_agent_api::ClientAction {
        action: Some(client_action::Action::BeginTransaction(
            client_action::BeginTransaction {},
        )),
    }
}

/// Build a `CommitTransaction` action.
pub(crate) fn action_commit_transaction() -> warp_multi_agent_api::ClientAction {
    warp_multi_agent_api::ClientAction {
        action: Some(client_action::Action::CommitTransaction(
            client_action::CommitTransaction {},
        )),
    }
}

/// Build a `CreateTask` action with a minimal Task carrying just the id.
pub(crate) fn action_create_task(task_id: &str) -> warp_multi_agent_api::ClientAction {
    warp_multi_agent_api::ClientAction {
        action: Some(client_action::Action::CreateTask(
            client_action::CreateTask {
                task: Some(Task {
                    id: task_id.to_string(),
                    ..Default::default()
                }),
            },
        )),
    }
}

/// Build an `AddMessagesToTask` action that seeds the task with one empty
/// `AgentOutput` message — subsequent `AppendToMessageContent` actions
/// stream text into its `text` field.
pub(crate) fn action_add_empty_agent_output_message(
    task_id: &str,
    message_id: &str,
) -> warp_multi_agent_api::ClientAction {
    let msg = Message {
        id: message_id.to_string(),
        task_id: task_id.to_string(),
        message: Some(message::Message::AgentOutput(
            message::AgentOutput {
                text: String::new(),
                ..Default::default()
            },
        )),
        ..Default::default()
    };
    warp_multi_agent_api::ClientAction {
        action: Some(client_action::Action::AddMessagesToTask(
            client_action::AddMessagesToTask {
                task_id: task_id.to_string(),
                messages: vec![msg],
            },
        )),
    }
}

/// Build an `AppendToMessageContent` action that appends `delta` to the
/// agent output's text. The mask `"agent_output.text"` tells the client
/// to append the message's `agent_output.text` to the existing message
/// in place, rather than replace it.
pub(crate) fn action_append_text(
    task_id: &str,
    message_id: &str,
    delta: &str,
) -> warp_multi_agent_api::ClientAction {
    let msg = Message {
        id: message_id.to_string(),
        task_id: task_id.to_string(),
        message: Some(message::Message::AgentOutput(
            message::AgentOutput {
                text: delta.to_string(),
                ..Default::default()
            },
        )),
        ..Default::default()
    };
    warp_multi_agent_api::ClientAction {
        action: Some(client_action::Action::AppendToMessageContent(
            client_action::AppendToMessageContent {
                task_id: task_id.to_string(),
                message: Some(msg),
                mask: Some(::prost_types::FieldMask {
                    paths: vec!["agent_output.text".to_string()],
                }),
            },
        )),
    }
}

/// Build the final `StreamFinished{Done}` event.
pub(crate) fn build_stream_finished_done() -> ResponseEvent {
    ResponseEvent {
        r#type: Some(response_event::Type::Finished(
            response_event::StreamFinished {
                reason: Some(
                    response_event::stream_finished::Reason::Done(
                        response_event::stream_finished::Done {},
                    ),
                ),
                ..Default::default()
            },
        )),
    }
}

use crate::{AiProvider, ResponseEventStream};
use futures::StreamExt;

#[async_trait::async_trait]
impl AiProvider for OpenAiAdapter {
    async fn chat_stream(
        &self,
        request: &Request,
    ) -> std::result::Result<ResponseEventStream, Arc<AIApiError>> {
        // Translate the Warp request to OpenAI JSON.
        let body = self.build_request_body(request)?;
        let url = format!("{}/chat/completions", self.config.endpoint.trim_end_matches('/'));

        // Lazily get the reqwest client (OnceLock from Task 3 TLS workaround).
        let client = self.client.get_or_init(reqwest::Client::new);

        // Build the POST request using .json() so the body stays as Bytes
        // (required for EventSource::new which calls try_clone internally).
        let request_builder = client
            .post(&url)
            .header("Authorization", format!("Bearer {}", self.config.api_key))
            .json(&body);

        // Open the SSE stream.
        let event_source = reqwest_eventsource::EventSource::new(request_builder)
            .map_err(|e| {
                Arc::new(AIApiError::Other(anyhow::anyhow!(
                    "OpenAI adapter: failed to open SSE stream: {e:#}"
                )))
            })?;

        // Synthesize a fresh set of Warp IDs for this stream.
        let ids = StreamIds::new();

        // Phase 1: opening events (Init, BeginTransaction, CreateTask, AddMessagesToTask).
        let opening: Vec<std::result::Result<ResponseEvent, Arc<AIApiError>>> = vec![
            Ok(build_stream_init(&ids)),
            Ok(build_client_actions(vec![
                action_begin_transaction(),
                action_create_task(&ids.task_id),
                action_add_empty_agent_output_message(&ids.task_id, &ids.message_id),
            ])),
        ];
        let opening_stream = futures::stream::iter(opening);

        // Phase 2: streaming deltas. Capture owned IDs into the closure.
        let task_id = ids.task_id.clone();
        let message_id = ids.message_id.clone();
        let body_stream = event_source.filter_map(move |event| {
            let task_id = task_id.clone();
            let message_id = message_id.clone();
            async move {
                match event {
                    Ok(reqwest_eventsource::Event::Open) => None,
                    Ok(reqwest_eventsource::Event::Message(msg)) => {
                        // OpenAI emits a final `data: [DONE]` line that should be ignored.
                        if msg.data == "[DONE]" {
                            return None;
                        }
                        match parse_delta(&msg.data) {
                            Ok(Some(delta)) => Some(Ok(build_client_actions(vec![
                                action_append_text(&task_id, &message_id, &delta),
                            ]))),
                            Ok(None) => None,
                            Err(e) => Some(Err(e)),
                        }
                    }
                    Err(reqwest_eventsource::Error::StreamEnded) => None,
                    Err(e) => Some(Err(Arc::new(
                        AIApiError::from_stream_error("OpenAiAdapter", e).await,
                    ))),
                }
            }
        });

        // Phase 3: closing events.
        let closing: Vec<std::result::Result<ResponseEvent, Arc<AIApiError>>> = vec![
            Ok(build_client_actions(vec![action_commit_transaction()])),
            Ok(build_stream_finished_done()),
        ];
        let closing_stream = futures::stream::iter(closing);

        // Concatenate.
        let combined = opening_stream.chain(body_stream).chain(closing_stream);
        Ok(Box::pin(combined))
    }
}

/// Free-function form of `extract_text_delta` so it can be called from inside
/// the streaming closure (which can't capture `&self`). Same logic as the
/// method on `OpenAiAdapter`; mirroring it as a free fn keeps the closure
/// `'static` without requiring an `Arc<OpenAiAdapter>`.
fn parse_delta(chunk_json: &str) -> std::result::Result<Option<String>, Arc<AIApiError>> {
    let v: serde_json::Value = serde_json::from_str(chunk_json).map_err(|e| {
        Arc::new(AIApiError::Other(anyhow::anyhow!(
            "OpenAI adapter: failed to parse SSE chunk JSON: {e:#}"
        )))
    })?;
    let content = v
        .pointer("/choices/0/delta/content")
        .and_then(|c| c.as_str())
        .map(|s| s.to_string());
    Ok(content)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn with_env<F: FnOnce()>(vars: &[(&str, Option<&str>)], f: F) {
        let prev: Vec<(String, Option<String>)> = vars
            .iter()
            .map(|(k, _)| (k.to_string(), std::env::var(k).ok()))
            .collect();
        for (k, v) in vars {
            match v {
                Some(val) => std::env::set_var(k, val),
                None => std::env::remove_var(k),
            }
        }
        f();
        for (k, v) in prev {
            match v {
                Some(val) => std::env::set_var(&k, val),
                None => std::env::remove_var(&k),
            }
        }
    }

    #[test]
    fn loads_full_config() {
        with_env(
            &[
                ("WARP_AI_OPENAI_ENDPOINT", Some("https://example.test/v1")),
                ("WARP_AI_OPENAI_API_KEY", Some("sk-test-123")),
                ("WARP_AI_OPENAI_MODEL", Some("gpt-4o-mini")),
            ],
            || {
                let cfg = OpenAiConfig::from_env().expect("config");
                assert_eq!(cfg.endpoint, "https://example.test/v1");
                assert_eq!(cfg.api_key, "sk-test-123");
                assert_eq!(cfg.model, "gpt-4o-mini");
            },
        );
    }

    #[test]
    fn defaults_endpoint_when_unset() {
        with_env(
            &[
                ("WARP_AI_OPENAI_ENDPOINT", None),
                ("WARP_AI_OPENAI_API_KEY", Some("sk-default")),
                ("WARP_AI_OPENAI_MODEL", Some("gpt-4o-mini")),
            ],
            || {
                let cfg = OpenAiConfig::from_env().expect("config");
                assert_eq!(cfg.endpoint, OpenAiConfig::DEFAULT_ENDPOINT);
            },
        );
    }

    #[test]
    fn errors_when_api_key_missing() {
        with_env(
            &[
                ("WARP_AI_OPENAI_API_KEY", None),
                ("WARP_AI_OPENAI_MODEL", Some("gpt-4o-mini")),
            ],
            || {
                let err = OpenAiConfig::from_env().expect_err("expected err");
                assert!(format!("{err:#}").contains("WARP_AI_OPENAI_API_KEY"));
            },
        );
    }

    #[test]
    fn errors_when_model_missing() {
        with_env(
            &[
                ("WARP_AI_OPENAI_API_KEY", Some("sk-x")),
                ("WARP_AI_OPENAI_MODEL", None),
            ],
            || {
                let err = OpenAiConfig::from_env().expect_err("expected err");
                assert!(format!("{err:#}").contains("WARP_AI_OPENAI_MODEL"));
            },
        );
    }

    use warp_multi_agent_api::Request;
    use warp_multi_agent_api::request as req;

    fn build_request_with_query(text: &str) -> Request {
        Request {
            input: Some(req::Input {
                r#type: Some(req::input::Type::UserInputs(req::input::UserInputs {
                    inputs: vec![req::input::user_inputs::UserInput {
                        input: Some(
                            req::input::user_inputs::user_input::Input::UserQuery(
                                req::input::UserQuery {
                                    query: text.to_string(),
                                    ..Default::default()
                                },
                            ),
                        ),
                    }],
                    ..Default::default()
                })),
                ..Default::default()
            }),
            ..Default::default()
        }
    }

    #[test]
    fn extracts_user_query_from_user_inputs() {
        let req = build_request_with_query("hello world");
        let q = extract_user_query(&req).expect("query");
        assert_eq!(q, "hello world");
    }

    #[test]
    fn extract_errors_when_input_missing() {
        let req = Request::default();
        let err = extract_user_query(&req).expect_err("err");
        assert!(format!("{err:#}").contains("Request.input is missing"));
    }

    #[test]
    fn build_request_body_has_messages_array() {
        let cfg = OpenAiConfig {
            endpoint: "https://example.test/v1".into(),
            api_key: "sk-x".into(),
            model: "gpt-4o-mini".into(),
        };
        let adapter = OpenAiAdapter::new(cfg);
        let req = build_request_with_query("how are you");
        let body = adapter.build_request_body(&req).expect("body");
        assert_eq!(body["model"], "gpt-4o-mini");
        assert_eq!(body["stream"], true);
        let messages = body["messages"].as_array().expect("messages array");
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0]["role"], "system");
        assert_eq!(messages[1]["role"], "user");
        assert_eq!(messages[1]["content"], "how are you");
    }

    fn make_adapter() -> OpenAiAdapter {
        OpenAiAdapter::new(OpenAiConfig {
            endpoint: "https://example.test/v1".into(),
            api_key: "sk-x".into(),
            model: "gpt-4o-mini".into(),
        })
    }

    #[test]
    fn extracts_text_delta_from_content_chunk() {
        let adapter = make_adapter();
        let chunk = r#"{"id":"chatcmpl-1","object":"chat.completion.chunk","created":0,"model":"gpt-4o-mini","choices":[{"index":0,"delta":{"content":"Hello"},"finish_reason":null}]}"#;
        let delta = adapter.extract_text_delta(chunk).expect("delta");
        assert_eq!(delta, Some("Hello".to_string()));
    }

    #[test]
    fn returns_none_for_role_only_first_chunk() {
        let adapter = make_adapter();
        let chunk = r#"{"id":"chatcmpl-1","object":"chat.completion.chunk","created":0,"model":"gpt-4o-mini","choices":[{"index":0,"delta":{"role":"assistant"},"finish_reason":null}]}"#;
        let delta = adapter.extract_text_delta(chunk).expect("ok");
        assert_eq!(delta, None);
    }

    #[test]
    fn returns_none_for_finish_chunk() {
        let adapter = make_adapter();
        let chunk = r#"{"id":"chatcmpl-1","object":"chat.completion.chunk","created":0,"model":"gpt-4o-mini","choices":[{"index":0,"delta":{},"finish_reason":"stop"}]}"#;
        let delta = adapter.extract_text_delta(chunk).expect("ok");
        assert_eq!(delta, None);
    }

    #[test]
    fn errors_on_malformed_json() {
        let adapter = make_adapter();
        let err = adapter.extract_text_delta("not json").expect_err("err");
        assert!(format!("{err:#}").contains("failed to parse"));
    }

    #[test]
    fn stream_ids_are_unique_uuids() {
        let ids = StreamIds::new();
        assert_ne!(ids.conversation_id, ids.request_id);
        assert!(uuid::Uuid::parse_str(&ids.conversation_id).is_ok());
        assert!(uuid::Uuid::parse_str(&ids.task_id).is_ok());
    }

    #[test]
    fn append_text_action_has_correct_mask() {
        let action = action_append_text("task-1", "msg-1", "hello");
        match action.action.as_ref().unwrap() {
            client_action::Action::AppendToMessageContent(a) => {
                assert_eq!(a.task_id, "task-1");
                let mask = a.mask.as_ref().expect("mask");
                assert_eq!(mask.paths, vec!["agent_output.text"]);
                let msg = a.message.as_ref().expect("message");
                match msg.message.as_ref().unwrap() {
                    message::Message::AgentOutput(out) => {
                        assert_eq!(out.text, "hello");
                    }
                    _ => panic!("expected AgentOutput"),
                }
            }
            _ => panic!("expected AppendToMessageContent"),
        }
    }
}

//! OpenAI Chat Completions adapter. Translates Warp's internal protobuf
//! request/response into OpenAI's HTTP+SSE protocol and back.
//!
//! Configured via env vars:
//! - `WARP_AI_OPENAI_ENDPOINT` — base URL (default `https://api.openai.com/v1`)
//! - `WARP_AI_OPENAI_API_KEY` — bearer token (required)
//! - `WARP_AI_OPENAI_MODEL` — model id (required; e.g. `gpt-4o-mini`)
//!
//! M1b-chat: text-only chat (no tool calls). Tools land in M1c.

use std::sync::{Arc, RwLock};

use crate::AIApiError;

/// Process-wide override for the active OpenAI config. Set by the
/// Settings UI when the user saves their config; read by the dispatcher
/// in `generate_multi_agent_output`. Falls back to `OpenAiConfig::from_env()`
/// if `None`.
static RUNTIME_CONFIG: RwLock<Option<OpenAiConfig>> = RwLock::new(None);

/// Set the process-wide runtime config. Called from the Settings UI
/// whenever the user updates their configuration.
pub fn set_runtime_config(config: Option<OpenAiConfig>) {
    *RUNTIME_CONFIG.write().expect("RUNTIME_CONFIG poisoned") = config;
}

/// Read the process-wide runtime config, if set.
pub fn runtime_config() -> Option<OpenAiConfig> {
    RUNTIME_CONFIG.read().expect("RUNTIME_CONFIG poisoned").clone()
}

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

    /// Construct an `OpenAiConfig` from explicit values. Used when the
    /// values come from settings storage instead of env vars. Returns
    /// `Err` if `api_key` or `model` is empty (analogous to `from_env`'s
    /// missing-required-vars handling). An empty `endpoint` falls back
    /// to `DEFAULT_ENDPOINT`.
    pub fn from_parts(
        endpoint: String,
        api_key: String,
        model: String,
    ) -> std::result::Result<Self, Arc<AIApiError>> {
        if api_key.trim().is_empty() {
            return Err(Arc::new(AIApiError::Other(anyhow::anyhow!(
                "AI Provider API key is required"
            ))));
        }
        if model.trim().is_empty() {
            return Err(Arc::new(AIApiError::Other(anyhow::anyhow!(
                "AI Provider model is required"
            ))));
        }
        let endpoint = if endpoint.trim().is_empty() {
            Self::DEFAULT_ENDPOINT.to_string()
        } else {
            endpoint
        };
        Ok(Self { endpoint, api_key, model })
    }

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

/// Fetch the list of available model IDs from an OpenAI-compatible
/// endpoint's `/v1/models` route. Bypasses `OpenAiConfig` validation
/// because callers may invoke this before a model has been chosen.
///
/// Standard OpenAI returns `{"data": [{"id": "..."}, ...]}`; the same
/// shape is used by LiteLLM, OpenRouter, Ollama (with /v1 prefix), etc.
pub async fn fetch_available_models(
    endpoint: &str,
    api_key: &str,
) -> std::result::Result<Vec<String>, Arc<AIApiError>> {
    if api_key.trim().is_empty() {
        return Err(Arc::new(AIApiError::Other(anyhow::anyhow!(
            "AI Provider API key is required"
        ))));
    }
    let endpoint = if endpoint.trim().is_empty() {
        OpenAiConfig::DEFAULT_ENDPOINT
    } else {
        endpoint.trim_end_matches('/')
    };
    let url = format!("{endpoint}/models");
    let client = reqwest::Client::new();
    let resp = client
        .get(&url)
        .header("Authorization", format!("Bearer {api_key}"))
        .send()
        .await
        .map_err(|e| {
            Arc::new(AIApiError::Other(anyhow::anyhow!(
                "fetch_models: HTTP error: {e:#}"
            )))
        })?;
    if !resp.status().is_success() {
        return Err(Arc::new(AIApiError::Other(anyhow::anyhow!(
            "fetch_models: HTTP {}",
            resp.status()
        ))));
    }
    let body: serde_json::Value = resp.json().await.map_err(|e| {
        Arc::new(AIApiError::Other(anyhow::anyhow!(
            "fetch_models: parse error: {e:#}"
        )))
    })?;
    let models = body
        .get("data")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|m| m.get("id").and_then(|i| i.as_str()).map(String::from))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    if models.is_empty() {
        return Err(Arc::new(AIApiError::Other(anyhow::anyhow!(
            "fetch_models: endpoint returned no models"
        ))));
    }
    Ok(models)
}

use std::collections::HashMap;
use serde_json::json;
use warp_multi_agent_api::Request;

/// Accumulator for streaming OpenAI tool calls. OpenAI sends each
/// `tool_calls[i]` field across multiple SSE chunks (e.g., `name` in
/// the first chunk, then `arguments` partial JSON in subsequent
/// chunks). We assemble per-`index` until we receive a chunk with
/// `finish_reason == "tool_calls"`.
#[derive(Default, Debug)]
struct ToolCallAccumulator {
    /// index → (id, name, accumulated_args_json_string)
    inflight: HashMap<u32, AccumulatedToolCall>,
}

#[derive(Default, Debug, Clone)]
struct AccumulatedToolCall {
    id: String,
    name: String,
    arguments: String,
}

impl ToolCallAccumulator {
    fn ingest_chunk(&mut self, chunk: &serde_json::Value) {
        // Extract choices[0].delta.tool_calls[*]
        let Some(tcs) = chunk
            .pointer("/choices/0/delta/tool_calls")
            .and_then(|v| v.as_array())
        else {
            return;
        };
        for tc in tcs {
            let Some(index) = tc.get("index").and_then(|v| v.as_u64()) else {
                continue;
            };
            let entry = self.inflight.entry(index as u32).or_default();
            if let Some(id) = tc.get("id").and_then(|v| v.as_str()) {
                if !id.is_empty() {
                    entry.id = id.to_string();
                }
            }
            if let Some(func) = tc.get("function") {
                if let Some(name) = func.get("name").and_then(|v| v.as_str()) {
                    if !name.is_empty() {
                        entry.name = name.to_string();
                    }
                }
                if let Some(args_part) = func.get("arguments").and_then(|v| v.as_str()) {
                    entry.arguments.push_str(args_part);
                }
            }
        }
    }

    fn drain_completed(&mut self) -> Vec<AccumulatedToolCall> {
        // Collect entries with their keys so we can sort by index.
        let mut indexed: Vec<(u32, AccumulatedToolCall)> = self
            .inflight
            .drain()
            .collect();
        indexed.sort_by_key(|(k, _)| *k);
        indexed.into_iter().map(|(_, v)| v).collect()
    }
}

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
    ///
    /// Walks `request.task_context.tasks[*].messages` to reconstruct the full
    /// conversation history (UserQuery → role:user, AgentOutput → role:assistant,
    /// ToolCall → role:assistant with tool_calls), then appends the current
    /// input — either a UserQuery (role:user) or ToolCallResult entries
    /// (role:tool). When `request.settings.supported_tools` is non-empty, a
    /// `tools[]` array is added so the model knows which tools it can call.
    pub(crate) fn build_request_body(
        &self,
        request: &Request,
    ) -> std::result::Result<serde_json::Value, Arc<AIApiError>> {
        let mut messages: Vec<serde_json::Value> = vec![
            json!({ "role": "system", "content": SYSTEM_PROMPT }),
        ];

        let registry = crate::ToolRegistry::default();
        let messages_from_request = build_messages_from_request(request, &registry)?;
        messages.extend(messages_from_request);

        let mut body = json!({
            "model": self.config.model,
            "stream": true,
            "messages": messages,
        });

        // Add tools[] if the client declared supported tools.
        if let Some(settings) = request.settings.as_ref() {
            if !settings.supported_tools.is_empty() {
                let supported_registry = registry.filter_to_supported(&settings.supported_tools);
                let tools_json = supported_registry.openai_tools_json();
                if let serde_json::Value::Array(arr) = &tools_json {
                    if !arr.is_empty() {
                        body["tools"] = tools_json;
                        // The model decides whether to call a tool; default is "auto".
                    }
                }
            }
        }

        Ok(body)
    }

}

/// Minimal system prompt for M1b-chat. Keeps the model in "helpful coding
/// assistant" mode without claiming Warp-specific tool capabilities (since
/// tools land in M1c). Future plans may expand this from `Request.task_context`
/// + `Request.settings` to better mirror Warp's hosted prompt.
const SYSTEM_PROMPT: &str = "You are a helpful AI assistant integrated into a \
    terminal application. Respond clearly and concisely. Do not invoke tools \
    or external commands — just provide a textual answer.";


/// Walk `request.task_context.tasks[*].messages[*]` to reconstruct the
/// conversation history as OpenAI-shaped messages (role: user, role: assistant,
/// role: tool), then append the current `request.input` entries.
///
/// - `Message::UserQuery`  → `{ "role": "user", "content": ... }`
/// - `Message::AgentOutput` → `{ "role": "assistant", "content": ... }`
/// - `Message::ToolCall`   → `{ "role": "assistant", "tool_calls": [...] }`
///   (only emitted when the tool_def for the variant is in the registry)
/// - `UserInput::ToolCallResult` → `{ "role": "tool", "tool_call_id": ..., "content": ... }`
/// - `UserInput::UserQuery`      → `{ "role": "user", "content": ... }`
///
/// When the registry is empty (Phase A/early Phase B), the ToolCall history
/// re-emission and ToolCallResult → role:tool translation gracefully fall
/// through (no message emitted for those variants).
fn build_messages_from_request(
    request: &Request,
    registry: &crate::ToolRegistry,
) -> std::result::Result<Vec<serde_json::Value>, Arc<AIApiError>> {
    let mut out: Vec<serde_json::Value> = Vec::new();

    // Walk past tasks for multi-turn history.
    if let Some(tc) = request.task_context.as_ref() {
        for task in &tc.tasks {
            for msg in &task.messages {
                use warp_multi_agent_api::message;
                match msg.message.as_ref() {
                    Some(message::Message::UserQuery(uq)) => {
                        out.push(json!({ "role": "user", "content": uq.query }));
                    }
                    Some(message::Message::AgentOutput(ao)) => {
                        out.push(json!({ "role": "assistant", "content": ao.text }));
                    }
                    Some(message::Message::ToolCall(tc_msg)) => {
                        // Reconstruct the assistant's tool_call message so OpenAI
                        // sees the full conversation context.
                        if let Some(tool_variant) = tc_msg.tool.as_ref() {
                            if let Some(tool_def) = registry.tool_for_proto(tool_variant) {
                                let args = tool_def.encode_call_args(tool_variant);
                                out.push(json!({
                                    "role": "assistant",
                                    "tool_calls": [{
                                        "id": tc_msg.tool_call_id,
                                        "type": "function",
                                        "function": {
                                            "name": tool_def.name(),
                                            "arguments": serde_json::to_string(&args)
                                                .unwrap_or_default(),
                                        }
                                    }]
                                }));
                            }
                        }
                    }
                    _ => {
                        // ToolCallResult in history is not re-emitted here because the
                        // corresponding role:tool message was already produced in a
                        // prior request's input processing. Other variants (ServerEvent,
                        // Summarization, …) have no direct OpenAI representation.
                    }
                }
            }
        }
    }

    // Append the current input. A missing input is an error — we have no
    // user message to send to OpenAI.
    let Some(input) = request.input.as_ref() else {
        return Err(Arc::new(AIApiError::Other(anyhow::anyhow!(
            "OpenAI adapter: Request.input is missing"
        ))));
    };

    use warp_multi_agent_api::request::input as req_input;
    match input.r#type.as_ref() {
        Some(req_input::Type::UserInputs(user_inputs)) => {
            for ui in &user_inputs.inputs {
                match ui.input.as_ref() {
                    Some(req_input::user_inputs::user_input::Input::UserQuery(uq)) => {
                        out.push(json!({ "role": "user", "content": uq.query }));
                    }
                    Some(req_input::user_inputs::user_input::Input::ToolCallResult(tcr)) => {
                        if let Some(result_variant) = tcr.result.as_ref() {
                            if let Some(tool_def) =
                                registry.tool_for_proto_result(result_variant)
                            {
                                let content_text =
                                    tool_def.encode_result_text(result_variant)?;
                                out.push(json!({
                                    "role": "tool",
                                    "tool_call_id": tcr.tool_call_id,
                                    "content": content_text,
                                }));
                            } else {
                                // Unknown tool result variant — represent as a generic
                                // placeholder so the model knows the call completed.
                                out.push(json!({
                                    "role": "tool",
                                    "tool_call_id": tcr.tool_call_id,
                                    "content": "(unsupported tool result variant)",
                                }));
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
        #[allow(deprecated)]
        Some(req_input::Type::UserQuery(uq)) => {
            out.push(json!({ "role": "user", "content": uq.query }));
        }
        _ => {}
    }

    // Part 2: Defensive synthetic-user-query. If the messages array produced
    // above has no role:user entry (e.g., a tool-result-only turn whose
    // corresponding UserQuery was never stored in task history), prepend a
    // synthetic placeholder so the request never hits "No user query found".
    let has_user_message = out.iter().any(|m| {
        m.get("role").and_then(|r| r.as_str()) == Some("user")
    });
    if !has_user_message {
        out.insert(
            0,
            json!({
                "role": "user",
                "content": "(continue)",
            }),
        );
    }

    Ok(out)
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

    /// Build IDs for a request: reuse `conversation_id` and `task_id` from
    /// the incoming request if present (multi-turn continuation), otherwise
    /// synthesize fresh ones (fresh conversation).
    pub(crate) fn for_request(request: &Request) -> Self {
        let mut ids = Self::new();
        if let Some(metadata) = request.metadata.as_ref() {
            if !metadata.conversation_id.is_empty() {
                ids.conversation_id = metadata.conversation_id.clone();
            }
        }
        // Reuse the LAST task in TaskContext if any.
        if let Some(task_context) = request.task_context.as_ref() {
            if let Some(last_task) = task_context.tasks.last() {
                if !last_task.id.is_empty() {
                    ids.task_id = last_task.id.clone();
                }
            }
        }
        ids
    }

    /// Returns true if the request is a continuation (the task already
    /// existed before this request) — controls whether we emit a CreateTask
    /// action in the transaction sequence.
    pub(crate) fn is_continuation(request: &Request) -> bool {
        request
            .metadata
            .as_ref()
            .is_some_and(|m| !m.conversation_id.is_empty())
            || request
                .task_context
                .as_ref()
                .is_some_and(|tc| !tc.tasks.is_empty())
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

/// Build an `AddMessagesToTask` action from an arbitrary list of messages.
/// Used by `chat_stream` to batch the optional UserQuery placeholder and
/// the required empty AgentOutput message into a single opening action.
pub(crate) fn action_add_messages(
    task_id: &str,
    messages: Vec<warp_multi_agent_api::Message>,
) -> warp_multi_agent_api::ClientAction {
    warp_multi_agent_api::ClientAction {
        action: Some(client_action::Action::AddMessagesToTask(
            client_action::AddMessagesToTask {
                task_id: task_id.to_string(),
                messages,
            },
        )),
    }
}

/// Extract the user query text from the request's current input, if the
/// input contains a `UserQuery` variant. Returns `None` for tool-result-only
/// continuations (so we skip adding a redundant UserQuery to task history).
fn current_user_query_text(request: &Request) -> Option<String> {
    use warp_multi_agent_api::request::input;
    let input = request.input.as_ref()?;
    let input_type = input.r#type.as_ref()?;
    match input_type {
        input::Type::UserInputs(user_inputs) => {
            user_inputs.inputs.iter().find_map(|ui| match ui.input.as_ref() {
                Some(input::user_inputs::user_input::Input::UserQuery(uq)) => {
                    Some(uq.query.clone())
                }
                _ => None,
            })
        }
        _ => None,
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

/// Build a `ClientAction` that adds a `Message::ToolCall` to the conversation
/// task. The actual proto `ToolCall` variant is decoded by the registered
/// `ToolDefinition` for this tool name. If the tool name is unknown,
/// emit the message with `tool: None` (the client may render it as "unknown tool").
fn build_tool_call_action(
    task_id: &str,
    accumulated: &AccumulatedToolCall,
) -> warp_multi_agent_api::ClientAction {
    let registry = crate::ToolRegistry::default();
    let tool_def = registry.by_name(&accumulated.name);

    let tool_variant = if let Some(tool_def) = tool_def {
        let args_value = serde_json::from_str::<serde_json::Value>(&accumulated.arguments)
            .unwrap_or_else(|_| serde_json::Value::Object(serde_json::Map::new()));
        tool_def.decode_call_args(args_value).ok()
    } else {
        None
    };

    let tool_call_msg = warp_multi_agent_api::Message {
        id: uuid::Uuid::new_v4().to_string(),
        task_id: task_id.to_string(),
        message: Some(message::Message::ToolCall(
            message::ToolCall {
                tool_call_id: accumulated.id.clone(),
                tool: tool_variant,
            },
        )),
        ..Default::default()
    };

    warp_multi_agent_api::ClientAction {
        action: Some(client_action::Action::AddMessagesToTask(
            client_action::AddMessagesToTask {
                task_id: task_id.to_string(),
                messages: vec![tool_call_msg],
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

        // Synthesize or reuse Warp IDs for this stream.
        let ids = StreamIds::for_request(request);
        let is_continuation = StreamIds::is_continuation(request);
        log::info!(
            "[ai_provider::openai] chat_stream: conversation_id={} (continuation={}), task_id={}",
            ids.conversation_id,
            is_continuation,
            ids.task_id
        );

        // Phase 1: opening events (Init, BeginTransaction, [CreateTask,] AddMessagesToTask).
        //
        // Part 1 fix: if the request carries a UserQuery (i.e., it's a fresh user
        // message, not a tool-result-only continuation), emit a Message::UserQuery
        // into the task history BEFORE the empty AgentOutput placeholder. This
        // ensures that on the next turn, `build_messages_from_request` can walk
        // the task history and find a role:user message, avoiding the
        // "No user query found in messages" error from the endpoint.
        let mut messages_to_add: Vec<warp_multi_agent_api::Message> = Vec::new();

        if let Some(user_query_text) = current_user_query_text(request) {
            let user_query_msg = warp_multi_agent_api::Message {
                id: uuid::Uuid::new_v4().to_string(),
                task_id: ids.task_id.clone(),
                message: Some(message::Message::UserQuery(
                    message::UserQuery {
                        query: user_query_text,
                        ..Default::default()
                    },
                )),
                ..Default::default()
            };
            messages_to_add.push(user_query_msg);
        }

        // Always add the empty AgentOutput placeholder for streaming text deltas.
        let agent_output_msg = warp_multi_agent_api::Message {
            id: ids.message_id.clone(),
            task_id: ids.task_id.clone(),
            message: Some(message::Message::AgentOutput(
                message::AgentOutput {
                    text: String::new(),
                },
            )),
            ..Default::default()
        };
        messages_to_add.push(agent_output_msg);

        let mut opening_actions: Vec<warp_multi_agent_api::ClientAction> = Vec::new();
        opening_actions.push(action_begin_transaction());
        if !is_continuation {
            opening_actions.push(action_create_task(&ids.task_id));
        }
        opening_actions.push(action_add_messages(&ids.task_id, messages_to_add));

        let opening: Vec<std::result::Result<ResponseEvent, Arc<AIApiError>>> = vec![
            Ok(build_stream_init(&ids)),
            Ok(build_client_actions(opening_actions)),
        ];
        let opening_stream = futures::stream::iter(opening);

        // Phase 2: streaming deltas. Capture owned IDs into the closure.
        let task_id = ids.task_id.clone();
        let message_id = ids.message_id.clone();

        // Accumulator for tool_calls deltas across SSE chunks.
        // Wrapped in Arc<Mutex<...>> because the filter_map closure captures by
        // move and the future it returns is polled across async boundaries.
        let tool_accumulator = std::sync::Arc::new(
            std::sync::Mutex::new(ToolCallAccumulator::default()),
        );

        // Stop polling EventSource as soon as the HTTP response body ends.
        // Without this `take_while`, the default ExponentialBackoff retry policy
        // would reconnect indefinitely, preventing the closing events from ever
        // being emitted by the chained `closing_stream`.
        let body_stream = event_source
            .take_while(|e| {
                futures::future::ready(!matches!(
                    e,
                    Err(reqwest_eventsource::Error::StreamEnded)
                ))
            })
            .filter_map(move |event| {
            let task_id = task_id.clone();
            let message_id = message_id.clone();
            let tool_accumulator = tool_accumulator.clone();
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
                            Ok(None) => {
                                // Check for tool_calls delta or finish_reason.
                                if let Ok(chunk_json) =
                                    serde_json::from_str::<serde_json::Value>(&msg.data)
                                {
                                    tool_accumulator
                                        .lock()
                                        .unwrap()
                                        .ingest_chunk(&chunk_json);

                                    // If finish_reason == "tool_calls", emit the assembled
                                    // tool calls as a single ClientActions event.
                                    let finish = chunk_json
                                        .pointer("/choices/0/finish_reason")
                                        .and_then(|v| v.as_str());
                                    if finish == Some("tool_calls") {
                                        let calls = tool_accumulator
                                            .lock()
                                            .unwrap()
                                            .drain_completed();
                                        if !calls.is_empty() {
                                            let actions: Vec<_> = calls
                                                .iter()
                                                .map(|tc| build_tool_call_action(&task_id, tc))
                                                .collect();
                                            return Some(Ok(build_client_actions(actions)));
                                        }
                                    }
                                }
                                None
                            }
                            Err(e) => Some(Err(e)),
                        }
                    }
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

    #[test]
    fn from_parts_with_full_values() {
        let cfg = OpenAiConfig::from_parts(
            "https://example.test/v1".into(),
            "sk-test".into(),
            "gpt-4o-mini".into(),
        )
        .expect("config");
        assert_eq!(cfg.endpoint, "https://example.test/v1");
        assert_eq!(cfg.api_key, "sk-test");
        assert_eq!(cfg.model, "gpt-4o-mini");
    }

    #[test]
    fn from_parts_defaults_empty_endpoint() {
        let cfg = OpenAiConfig::from_parts(
            "".into(),
            "sk-x".into(),
            "gpt-4o-mini".into(),
        )
        .expect("config");
        assert_eq!(cfg.endpoint, OpenAiConfig::DEFAULT_ENDPOINT);
    }

    #[test]
    fn from_parts_errors_on_empty_api_key() {
        let err = OpenAiConfig::from_parts(
            "".into(),
            "".into(),
            "gpt-4o-mini".into(),
        )
        .expect_err("err");
        assert!(format!("{err:#}").contains("API key is required"));
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
                })),
                ..Default::default()
            }),
            ..Default::default()
        }
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

    #[test]
    fn build_request_body_omits_tools_when_supported_tools_empty() {
        let cfg = OpenAiConfig {
            endpoint: "https://example.test/v1".into(),
            api_key: "sk-x".into(),
            model: "gpt-4o-mini".into(),
        };
        let adapter = OpenAiAdapter::new(cfg);
        let req = build_request_with_query("hello");
        let body = adapter.build_request_body(&req).expect("body");
        assert!(body.get("tools").is_none(), "tools should be absent when supported_tools is empty");
    }

    #[allow(dead_code)]
    fn make_adapter() -> OpenAiAdapter {
        OpenAiAdapter::new(OpenAiConfig {
            endpoint: "https://example.test/v1".into(),
            api_key: "sk-x".into(),
            model: "gpt-4o-mini".into(),
        })
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

    #[test]
    fn for_request_reuses_conversation_id_when_set() {
        let request = warp_multi_agent_api::Request {
            metadata: Some(req::Metadata {
                conversation_id: "existing-conv".to_string(),
                ..Default::default()
            }),
            ..Default::default()
        };
        let ids = StreamIds::for_request(&request);
        assert_eq!(ids.conversation_id, "existing-conv");
        assert!(StreamIds::is_continuation(&request));
    }

    #[test]
    fn for_request_reuses_task_id_when_present() {
        let request = warp_multi_agent_api::Request {
            task_context: Some(warp_multi_agent_api::request::TaskContext {
                tasks: vec![warp_multi_agent_api::Task {
                    id: "existing-task".to_string(),
                    ..Default::default()
                }],
            }),
            ..Default::default()
        };
        let ids = StreamIds::for_request(&request);
        assert_eq!(ids.task_id, "existing-task");
        assert!(StreamIds::is_continuation(&request));
    }

    #[test]
    fn for_request_synthesizes_fresh_when_default() {
        let request = warp_multi_agent_api::Request::default();
        let ids = StreamIds::for_request(&request);
        assert!(uuid::Uuid::parse_str(&ids.conversation_id).is_ok());
        assert!(uuid::Uuid::parse_str(&ids.task_id).is_ok());
        assert!(!StreamIds::is_continuation(&request));
    }

    // --- Task 10: parallel tool calls ---

    #[test]
    fn accumulator_handles_two_parallel_tool_calls() {
        let mut acc = ToolCallAccumulator::default();
        acc.ingest_chunk(&serde_json::json!({
            "choices": [{
                "delta": {
                    "tool_calls": [
                        { "index": 0, "id": "call_a", "function": { "name": "run_shell_command", "arguments": "{\"command\":\"ls\"}" } },
                        { "index": 1, "id": "call_b", "function": { "name": "grep", "arguments": "{\"queries\":[\"main\"],\"path\":\".\"}" } }
                    ]
                }
            }]
        }));
        let drained = acc.drain_completed();
        assert_eq!(drained.len(), 2);
    }

    // --- Task 11: tool call error handling ---

    #[test]
    fn unknown_tool_name_emits_message_without_tool_variant() {
        let acc = AccumulatedToolCall {
            id: "call_x".into(),
            name: "made_up_tool".into(),
            arguments: "{}".into(),
        };
        let action = build_tool_call_action("task-1", &acc);
        match action.action.as_ref().unwrap() {
            client_action::Action::AddMessagesToTask(a) => {
                assert_eq!(a.messages.len(), 1);
                let msg = &a.messages[0];
                if let Some(message::Message::ToolCall(tc)) = msg.message.as_ref() {
                    assert!(tc.tool.is_none(), "unknown tool should produce tool=None");
                } else {
                    panic!("expected ToolCall message variant");
                }
            }
            _ => panic!("expected AddMessagesToTask"),
        }
    }

    #[test]
    fn malformed_args_falls_back_to_empty_object() {
        let acc = AccumulatedToolCall {
            id: "call_x".into(),
            name: "run_shell_command".into(),
            arguments: "{not valid json".into(),
        };
        let action = build_tool_call_action("task-1", &acc);
        match action.action.as_ref().unwrap() {
            client_action::Action::AddMessagesToTask(a) => {
                let msg = &a.messages[0];
                if let Some(message::Message::ToolCall(tc)) = msg.message.as_ref() {
                    assert!(tc.tool.is_none(), "malformed args should produce tool=None");
                }
            }
            _ => panic!("expected AddMessagesToTask"),
        }
    }
}

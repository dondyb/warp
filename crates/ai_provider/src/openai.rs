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
}

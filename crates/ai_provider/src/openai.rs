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
}

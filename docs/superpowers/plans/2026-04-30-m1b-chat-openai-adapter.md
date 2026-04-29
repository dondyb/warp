# M1b-chat — OpenAI Chat Completions Adapter (Text-Only)

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Implement `OpenAiAdapter` that translates Warp's `warp_multi_agent_api::Request` into OpenAI Chat Completions calls, streams the SSE response back, and emits the canonical Warp `ResponseEvent` transaction sequence so Agent Mode renders the AI's reply. **MVP scope: text-only, no tools.** A user with `WARP_AI_PROTOCOL=openai`, `WARP_AI_OPENAI_ENDPOINT=...`, `WARP_AI_OPENAI_API_KEY=...`, `WARP_AI_OPENAI_MODEL=...` set in the environment can type a prompt in Agent Mode and get a streaming text response from their endpoint.

**Architecture:** New module `crates/ai_provider/src/openai.rs` containing the `OpenAiConfig` struct (env-var-loaded), `OpenAiAdapter` struct (impls `AiProvider`), and the translation logic. Adapter:

1. Extracts the user query from `Request.input` (`UserInputs::user_query` and the deprecated `user_query` variants).
2. Builds an OpenAI Chat Completions JSON body with a fixed minimal system prompt + the user query.
3. POSTs to `{endpoint}/v1/chat/completions` with `Authorization: Bearer {key}`, `stream: true`.
4. Parses the SSE response into OpenAI delta chunks.
5. Synthesizes the canonical Warp transaction sequence: `StreamInit` → `BeginTransaction` → `CreateTask` → `AddMessagesToTask{messages: [Message{agent_output: AgentOutput{text: ""}}]}` → repeated `AppendToMessageContent{mask: "agent_output.text"}` → `CommitTransaction` → `StreamFinished{Done}`.
6. Plugs into the dispatcher's `Protocol::OpenAi` arm in `app/src/server/server_api.rs`, replacing the inline "not yet implemented" error.

**Tech Stack:** Rust 2021. `reqwest` (already a dep) for HTTP + `reqwest_eventsource` for SSE, `serde_json` for the OpenAI JSON, `uuid` for synthesizing IDs. `mockito` (already a workspace dep) for tests. No new external crate dependencies beyond `uuid`.

---

## Context

Confirmed proto details from a survey:

- **`AgentOutput`** has a single field: `string text = 1;`. The AI's text reply lives there.
- **`AppendToMessageContent { task_id, message, mask: FieldMask }`** — mask is a string field path like `"agent_output.text"`. The `message` field carries the FULL message with the **new content delta as the value of the masked field**; the client's `FieldMaskOperation::append` then appends that string to the existing message field. So for a text delta `" world"`, set `message.agent_output.text = " world"` and `mask = "agent_output.text"`.
- **`Task`** requires only `id` (a `string`). All other fields are optional.
- **IDs are opaque strings.** UUIDs work; the client doesn't validate format.
- **No existing OpenAI Rust client** in the workspace. Hand-rolled via reqwest + serde_json.
- **`AiProvider` trait** is defined in `crates/ai_provider/src/client.rs:20–32` with `async fn chat_stream(&self, &Request) -> Result<ResponseEventStream, Arc<AIApiError>>`.
- **Dispatcher** at `app/src/server/server_api.rs:885–908` (line numbers may shift) currently returns `Err(...)` for `Protocol::OpenAi`. This task replaces that with adapter construction.

After M1b-chat:

- `crates/ai_provider/src/openai.rs` defines `OpenAiConfig`, `OpenAiAdapter`, and translation helpers.
- The dispatcher constructs `OpenAiAdapter::from_env()` for the OpenAi branch.
- A `WARP_AI_PROTOCOL=openai` build sends a chat completion to the configured endpoint and renders the streamed text in Agent Mode.

**Out of scope** (deferred): tool calling (M1c), multi-turn conversation continuation (relies on the user re-typing context for now), input variants other than `UserQuery`, error retry policy, capability detection, settings GUI (M3).

## File Structure

**Create:**

| Path | Responsibility |
|---|---|
| `crates/ai_provider/src/openai.rs` | `OpenAiConfig` (env-var loader), `OpenAiAdapter` (impls `AiProvider`), translation helpers + a `#[cfg(test)] mod tests` block. |
| `crates/ai_provider/tests/openai_integration.rs` | Mockito-based tests covering happy path + key error paths. |

**Modify:**

| Path | Change |
|---|---|
| `crates/ai_provider/Cargo.toml` | Add `uuid = { workspace = true, features = ["v4"] }`. Add `mockito` to `[dev-dependencies]` if not present. |
| `Cargo.toml` (root) | Verify `uuid` is in `[workspace.dependencies]`. If not, add `uuid = "1.10"`. |
| `crates/ai_provider/src/lib.rs` | Add `pub mod openai;` and `pub use openai::{OpenAiAdapter, OpenAiConfig};`. |
| `app/src/server/server_api.rs` | Replace the inline `Err(...)` for `Protocol::OpenAi` with `OpenAiAdapter::from_env()` + delegation through `AiProvider::chat_stream`. Update the M1a integration test's error-substring assertion if it references the old "not yet implemented" wording. |

---

## Tasks

### Task 1: Add `uuid` dep + module skeleton

**Files:**
- Modify: `Cargo.toml` (workspace root, only if `uuid` missing)
- Modify: `crates/ai_provider/Cargo.toml`
- Modify: `crates/ai_provider/src/lib.rs`
- Create: `crates/ai_provider/src/openai.rs` (stub)

- [ ] **Step 1: Verify `uuid` is in workspace deps**

Run: `rg -n "^uuid ?=" /Users/dondy/Codes/warp/Cargo.toml`. If a hit, note the version. If not, add to `[workspace.dependencies]` (alphabetical):

```toml
uuid = { version = "1.10", default-features = false, features = ["v4"] }
```

(If a different version is already pinned, use that — don't downgrade. The features `v4` is what we need for `Uuid::new_v4()`.)

- [ ] **Step 2: Add deps to `crates/ai_provider/Cargo.toml`**

In the `[dependencies]` section, add:

```toml
uuid = { workspace = true, features = ["v4"] }
```

In `[dev-dependencies]`, ensure these are present (add if missing):

```toml
mockito.workspace = true
tokio = { workspace = true, features = ["macros", "rt-multi-thread"] }
```

- [ ] **Step 3: Create `crates/ai_provider/src/openai.rs` stub**

Create the file with:

```rust
//! OpenAI Chat Completions adapter. Translates Warp's internal protobuf
//! request/response into OpenAI's HTTP+SSE protocol and back.
//!
//! Configured via env vars:
//! - `WARP_AI_OPENAI_ENDPOINT` — base URL (default `https://api.openai.com/v1`)
//! - `WARP_AI_OPENAI_API_KEY` — bearer token (required)
//! - `WARP_AI_OPENAI_MODEL` — model id (required; e.g. `gpt-4o-mini`)
//!
//! M1b-chat: text-only chat (no tool calls). Tools land in M1c.

// Populated in subsequent tasks.
```

- [ ] **Step 4: Update `crates/ai_provider/src/lib.rs`**

Add a new line near the existing `pub mod client; pub mod error;` declarations:

```rust
pub mod openai;
```

Don't add a re-export yet — Tasks 3 and 5 add the types that get re-exported.

- [ ] **Step 5: Verify compile**

Run: `cargo check -p ai_provider 2>&1 | tail -5`
Expected: PASS, no errors.

- [ ] **Step 6: Commit**

```bash
cd /Users/dondy/Codes/warp
git add Cargo.toml crates/ai_provider
git commit -m "build(ai_provider): add uuid dep and openai module stub"
```

After committing: `git log --oneline -2`.

---

### Task 2: Define `OpenAiConfig` + env-var loader

**Files:**
- Modify: `crates/ai_provider/src/openai.rs` (add `OpenAiConfig`)
- Modify: `crates/ai_provider/src/lib.rs` (re-export)

- [ ] **Step 1: Append to `crates/ai_provider/src/openai.rs`**

Replace the file's contents with:

```rust
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
```

- [ ] **Step 2: Re-export from `lib.rs`**

In `crates/ai_provider/src/lib.rs`, find the existing `pub use error::...` line and add a new line near it:

```rust
pub use openai::OpenAiConfig;
```

(Don't re-export `OpenAiAdapter` yet — Task 3 creates it.)

- [ ] **Step 3: Run the tests**

Run: `cargo nextest run -p ai_provider --lib openai::tests`
Expected: 4 tests pass.

If tests are flaky (env-var contamination across tests in the same process), nextest's process-per-test default isolates them — the helper restores state per-test as a belt-and-suspenders. If flake nonetheless, add `serial_test` as a workspace dev-dep.

- [ ] **Step 4: Commit**

```bash
cd /Users/dondy/Codes/warp
git add crates/ai_provider
git commit -m "feat(ai_provider): add OpenAiConfig + env-var loader"
```

---

### Task 3: `OpenAiAdapter` skeleton + JSON request body

**Files:**
- Modify: `crates/ai_provider/src/openai.rs` (add adapter struct and request-builder helper)
- Modify: `crates/ai_provider/src/lib.rs` (re-export)

- [ ] **Step 1: Append the adapter struct to `crates/ai_provider/src/openai.rs`**

After the `OpenAiConfig` block, before the `#[cfg(test)] mod tests` block, append:

```rust
use serde_json::json;
use warp_multi_agent_api::{Request, request as req};

/// `AiProvider` impl that translates Warp's protobuf request to/from the
/// OpenAI Chat Completions HTTP API.
pub struct OpenAiAdapter {
    config: OpenAiConfig,
    client: reqwest::Client,
}

impl OpenAiAdapter {
    /// Build an adapter from environment variables.
    pub fn from_env() -> std::result::Result<Self, Arc<AIApiError>> {
        Ok(Self::new(OpenAiConfig::from_env()?))
    }

    pub fn new(config: OpenAiConfig) -> Self {
        Self {
            config,
            client: reqwest::Client::new(),
        }
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
```

> **Note about the proto module path** (`use warp_multi_agent_api::{Request, request as req};`): the protobuf `Request` message has nested message types accessed via `request::input::*`. If `cargo check` complains about the path, run `rg -nl "use warp_multi_agent_api" /Users/dondy/Codes/warp/app/src` to find existing call sites and copy their import style. Common alternatives: `warp_multi_agent_api::request::input::Type`, `warp_multi_agent_api::Request_::Input_::Type_` (depends on prost-rust naming).

- [ ] **Step 2: Add corresponding test cases to the existing `mod tests`**

Inside the `#[cfg(test)] mod tests { ... }` block, add:

```rust
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
```

- [ ] **Step 3: Re-export `OpenAiAdapter` from `lib.rs`**

Update the existing `pub use openai::OpenAiConfig;` line to:

```rust
pub use openai::{OpenAiAdapter, OpenAiConfig};
```

- [ ] **Step 4: Verify compile and run the new tests**

Run:
```bash
cargo check -p ai_provider 2>&1 | tail -5
cargo nextest run -p ai_provider --lib openai::tests
```
Expected: 7 tests pass (the 4 from Task 2 + 3 new ones).

If a compile error fires about `request as req` not resolving, run `rg -nl "use warp_multi_agent_api::" /Users/dondy/Codes/warp/app/src` to find a successful import site (e.g., in `app/src/server/server_api.rs` or a callsite) and copy its style.

- [ ] **Step 5: Commit**

```bash
cd /Users/dondy/Codes/warp
git add crates/ai_provider
git commit -m "feat(ai_provider): OpenAiAdapter skeleton + chat-completions request body"
```

---

### Task 4: SSE response → text deltas

**Files:**
- Modify: `crates/ai_provider/src/openai.rs` (add SSE parsing helper)

- [ ] **Step 1: Append a delta-extraction helper inside `OpenAiAdapter`'s impl block**

Add to `OpenAiAdapter`'s `impl` block (between `build_request_body` and the closing brace):

```rust
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
```

- [ ] **Step 2: Add tests for the helper inside `mod tests`**

Append to the existing `#[cfg(test)] mod tests` block:

```rust
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
```

- [ ] **Step 3: Verify**

Run: `cargo nextest run -p ai_provider --lib openai::tests`
Expected: 11 tests pass (7 prior + 4 new).

- [ ] **Step 4: Commit**

```bash
cd /Users/dondy/Codes/warp
git add crates/ai_provider/src/openai.rs
git commit -m "feat(ai_provider): SSE chunk → text-delta extraction"
```

---

### Task 5: Build the Warp `ResponseEvent` transaction sequence

**Files:**
- Modify: `crates/ai_provider/src/openai.rs` (add transaction-sequence builder helpers)

- [ ] **Step 1: Append builder helpers**

Inside `crates/ai_provider/src/openai.rs`, near the end of the file (before `#[cfg(test)] mod tests`), append:

```rust
use warp_multi_agent_api::{response, ResponseEvent, Task, Message};
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
        r#type: Some(response::response_event::Type::Init(
            response::response_event::StreamInit {
                conversation_id: ids.conversation_id.clone(),
                request_id: ids.request_id.clone(),
                run_id: ids.run_id.clone(),
            },
        )),
    }
}

/// Build a `ClientActions` ResponseEvent containing the given actions.
pub(crate) fn build_client_actions(
    actions: Vec<response::ClientAction>,
) -> ResponseEvent {
    ResponseEvent {
        r#type: Some(response::response_event::Type::ClientActions(
            response::response_event::ClientActions { actions },
        )),
    }
}

/// Build a `BeginTransaction` action.
pub(crate) fn action_begin_transaction() -> response::ClientAction {
    response::ClientAction {
        action: Some(response::client_action::Action::BeginTransaction(
            response::client_action::BeginTransaction {},
        )),
    }
}

/// Build a `CommitTransaction` action.
pub(crate) fn action_commit_transaction() -> response::ClientAction {
    response::ClientAction {
        action: Some(response::client_action::Action::CommitTransaction(
            response::client_action::CommitTransaction {},
        )),
    }
}

/// Build a `CreateTask` action with a minimal Task carrying just the id.
pub(crate) fn action_create_task(task_id: &str) -> response::ClientAction {
    response::ClientAction {
        action: Some(response::client_action::Action::CreateTask(
            response::client_action::CreateTask {
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
) -> response::ClientAction {
    let message = Message {
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
    response::ClientAction {
        action: Some(response::client_action::Action::AddMessagesToTask(
            response::client_action::AddMessagesToTask {
                task_id: task_id.to_string(),
                messages: vec![message],
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
) -> response::ClientAction {
    let message = Message {
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
    response::ClientAction {
        action: Some(response::client_action::Action::AppendToMessageContent(
            response::client_action::AppendToMessageContent {
                task_id: task_id.to_string(),
                message: Some(message),
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
        r#type: Some(response::response_event::Type::Finished(
            response::response_event::StreamFinished {
                reason: Some(
                    response::response_event::stream_finished::Reason::Done(
                        response::response_event::stream_finished::Done {},
                    ),
                ),
                ..Default::default()
            },
        )),
    }
}
```

> **Path-resolution caveat:** The proto-generated module paths above (`response::response_event::*`, `response::client_action::*`, `message::*`) match prost's standard naming. If `cargo check` reports unresolved paths, the proto module structure may differ. Investigate with:
> ```bash
> rg "pub mod response_event|pub mod client_action|pub mod message_v2" $(find /Users/dondy/.cargo/git/checkouts/warp-proto-apis-* -name "*.rs" 2>/dev/null) | head -20
> ```
> and adjust paths to match.

- [ ] **Step 2: Add tests for the builders**

In the `#[cfg(test)] mod tests`:

```rust
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
            response::client_action::Action::AppendToMessageContent(a) => {
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
```

- [ ] **Step 3: Verify**

Run: `cargo nextest run -p ai_provider --lib openai::tests`
Expected: all openai unit tests pass.

- [ ] **Step 4: Commit**

```bash
cd /Users/dondy/Codes/warp
git add crates/ai_provider/src/openai.rs
git commit -m "feat(ai_provider): build canonical Warp transaction events"
```

---

### Task 6: Implement `AiProvider::chat_stream` for `OpenAiAdapter`

**Files:**
- Modify: `crates/ai_provider/src/openai.rs` (add the trait impl)

- [ ] **Step 1: Append the trait impl**

At the bottom of `crates/ai_provider/src/openai.rs` (before `#[cfg(test)] mod tests`):

```rust
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

        // Open the SSE stream.
        let event_source = reqwest_eventsource::EventSource::new(
            self.client
                .post(&url)
                .header("Authorization", format!("Bearer {}", self.config.api_key))
                .header("Content-Type", "application/json")
                .body(serde_json::to_string(&body).map_err(|e| {
                    Arc::new(AIApiError::Other(anyhow::anyhow!(
                        "OpenAI adapter: failed to serialize request: {e:#}"
                    )))
                })?),
        )
        .map_err(|e| {
            Arc::new(AIApiError::Other(anyhow::anyhow!(
                "OpenAI adapter: failed to open SSE stream: {e:#}"
            )))
        })?;

        // Synthesize a fresh set of Warp IDs for this stream.
        let ids = StreamIds::new();

        // Adapter copies for the stream closure.
        let adapter = OpenAiAdapter {
            config: self.config.clone(),
            client: self.client.clone(),
        };

        // Build a stream that emits the canonical event sequence.
        // Phase 1: opening events (Init, BeginTransaction, CreateTask, AddMessagesToTask).
        // Phase 2: AppendToMessageContent for each text delta.
        // Phase 3: Closing events (CommitTransaction, StreamFinished{Done}).
        let opening = vec![
            Ok(build_stream_init(&ids)),
            Ok(build_client_actions(vec![
                action_begin_transaction(),
                action_create_task(&ids.task_id),
                action_add_empty_agent_output_message(&ids.task_id, &ids.message_id),
            ])),
        ];
        let opening_stream = futures::stream::iter(opening);

        // Phase 2: streaming deltas.
        let task_id = ids.task_id.clone();
        let message_id = ids.message_id.clone();
        let body_stream = event_source.filter_map(move |event| {
            let task_id = task_id.clone();
            let message_id = message_id.clone();
            let adapter = OpenAiAdapter {
                config: adapter.config.clone(),
                client: adapter.client.clone(),
            };
            async move {
                match event {
                    Ok(reqwest_eventsource::Event::Open) => None,
                    Ok(reqwest_eventsource::Event::Message(msg)) => {
                        // OpenAI emits a final `data: [DONE]` line that should be ignored.
                        if msg.data == "[DONE]" {
                            return None;
                        }
                        match adapter.extract_text_delta(&msg.data) {
                            Ok(Some(delta)) => {
                                Some(Ok(build_client_actions(vec![action_append_text(
                                    &task_id,
                                    &message_id,
                                    &delta,
                                )])))
                            }
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
        let closing = vec![
            Ok(build_client_actions(vec![action_commit_transaction()])),
            Ok(build_stream_finished_done()),
        ];
        let closing_stream = futures::stream::iter(closing);

        // Concatenate.
        let combined = opening_stream.chain(body_stream).chain(closing_stream);
        Ok(Box::pin(combined))
    }
}
```

> **The closure-clone gymnastics around `OpenAiAdapter`** are because `EventSource::filter_map` requires `'static` ownership. If the borrow checker complains, the simplest fix is to factor `extract_text_delta` to a free function (no `&self`) — adjust the signature in Task 4 if needed.

> **Note:** if `reqwest_eventsource::Error::StreamEnded` doesn't exist (older reqwest_eventsource versions named it `Stream` or had a different end-of-stream signal), check the actual variant via `rg -n "enum Error" $(find /Users/dondy/.cargo/registry/src -path "*/reqwest-eventsource-*/src/lib.rs" 2>/dev/null | head -1) | head -5`. Adjust the match arm.

- [ ] **Step 2: Verify compile**

Run: `cargo check -p ai_provider 2>&1 | tail -10`
Expected: PASS.

If borrow-checker / lifetime errors fire, the most common fix is to capture only owned data (clone the IDs and the adapter into the closure as shown). If unable to resolve in 30 minutes, report `BLOCKED` with the exact error.

- [ ] **Step 3: Commit**

```bash
cd /Users/dondy/Codes/warp
git add crates/ai_provider/src/openai.rs
git commit -m "feat(ai_provider): impl AiProvider::chat_stream for OpenAiAdapter"
```

---

### Task 7: Wire `OpenAiAdapter` into the dispatcher

**Files:**
- Modify: `app/src/server/server_api.rs` (replace the inline `Err(...)` for `Protocol::OpenAi`)

- [ ] **Step 1: Find the dispatcher**

Run: `rg -n "Protocol::OpenAi" /Users/dondy/Codes/warp/app/src/server/server_api.rs`. The dispatcher is in `pub async fn generate_multi_agent_output`.

- [ ] **Step 2: Replace the `Protocol::OpenAi` arm**

Edit the arm. It currently reads:

```rust
Protocol::OpenAi => Err(Arc::new(AIApiError::Other(anyhow!(
    "WARP_AI_PROTOCOL=openai requested, but the OpenAI adapter \
     is not yet implemented (planned for M1b-chat)"
)))),
```

Replace with:

```rust
Protocol::OpenAi => {
    let adapter = ai_provider::OpenAiAdapter::from_env()?;
    ai_provider::AiProvider::chat_stream(&adapter, request).await
}
```

> The leftover `Anthropic` arm stays unchanged for now (M2 plugs it in).

- [ ] **Step 3: Update the M1a integration test's expected error substring**

The test `openai_protocol_returns_not_implemented_error` in `app/src/server/server_api.rs` (in the M1a `#[cfg(test)] mod m1a_dispatch_tests` at the bottom of the file) asserts `format!("{err:#}").contains("OpenAI adapter is not yet implemented")`. With the OpenAI adapter now wired up, that error no longer fires for the OpenAi protocol — instead, with no env vars set, `OpenAiAdapter::from_env()` errors with "WARP_AI_PROTOCOL=openai requires WARP_AI_OPENAI_API_KEY".

Update that test's assertion to:

```rust
assert!(
    format!("{err:#}").contains("requires WARP_AI_OPENAI_API_KEY"),
    "unexpected error: {err:#}"
);
```

The Anthropic test stays unchanged.

- [ ] **Step 4: Verify the workspace builds**

Run: `cargo check --workspace 2>&1 | tail -10`
Expected: PASS.

- [ ] **Step 5: Run the M1a/dispatch tests**

```bash
cargo nextest run -p warp -E 'test(openai_protocol_returns_not_implemented_error) | test(anthropic_protocol_returns_not_implemented_error)'
```
Expected: 2 tests pass.

- [ ] **Step 6: Commit**

```bash
cd /Users/dondy/Codes/warp
git add app/src/server/server_api.rs
git commit -m "feat(server): wire OpenAiAdapter into Protocol dispatcher"
```

---

### Task 8: Mockito integration test — happy path

**Files:**
- Create: `crates/ai_provider/tests/openai_integration.rs`

- [ ] **Step 1: Create the integration test file**

Create `crates/ai_provider/tests/openai_integration.rs`:

```rust
//! Integration tests for `OpenAiAdapter` using `mockito` to fake the
//! OpenAI Chat Completions endpoint.

use ai_provider::{AIApiError, AiProvider, OpenAiAdapter, OpenAiConfig};
use futures::StreamExt;
use std::sync::Arc;
use warp_multi_agent_api::{request as req, Request, ResponseEvent, response};

fn make_request(query: &str) -> Request {
    Request {
        input: Some(req::Input {
            r#type: Some(req::input::Type::UserInputs(req::input::UserInputs {
                inputs: vec![req::input::user_inputs::UserInput {
                    input: Some(
                        req::input::user_inputs::user_input::Input::UserQuery(
                            req::input::UserQuery {
                                query: query.to_string(),
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

fn sse_chunk(json: &str) -> String {
    format!("data: {}\n\n", json)
}

#[tokio::test]
async fn happy_path_emits_canonical_transaction_sequence() {
    let mut server = mockito::Server::new_async().await;
    let body = format!(
        "{}{}{}",
        sse_chunk(r#"{"choices":[{"delta":{"role":"assistant"},"finish_reason":null}]}"#),
        sse_chunk(r#"{"choices":[{"delta":{"content":"Hello"},"finish_reason":null}]}"#),
        sse_chunk(r#"{"choices":[{"delta":{"content":" world"},"finish_reason":"stop"}]}"#)
            + "data: [DONE]\n\n",
    );
    let _m = server
        .mock("POST", "/chat/completions")
        .match_header("authorization", "Bearer sk-test")
        .with_status(200)
        .with_header("content-type", "text/event-stream")
        .with_body(body)
        .create_async()
        .await;

    let adapter = OpenAiAdapter::new(OpenAiConfig {
        endpoint: server.url(),
        api_key: "sk-test".into(),
        model: "gpt-4o-mini".into(),
    });
    let mut stream = adapter
        .chat_stream(&make_request("hi"))
        .await
        .expect("stream");

    // Collect all events.
    let mut events: Vec<ResponseEvent> = Vec::new();
    while let Some(item) = stream.next().await {
        events.push(item.expect("event"));
    }

    // Expected sequence:
    // 0: StreamInit
    // 1: ClientActions [BeginTransaction, CreateTask, AddMessagesToTask]
    // 2: ClientActions [AppendToMessageContent("Hello")]
    // 3: ClientActions [AppendToMessageContent(" world")]
    // 4: ClientActions [CommitTransaction]
    // 5: StreamFinished{Done}
    assert_eq!(events.len(), 6, "events: {events:#?}");

    // Spot-check the structure.
    matches!(
        events[0].r#type.as_ref().unwrap(),
        response::response_event::Type::Init(_)
    );
    matches!(
        events[5].r#type.as_ref().unwrap(),
        response::response_event::Type::Finished(_)
    );

    // The two delta events carry the right text.
    let extract_append_text = |e: &ResponseEvent| -> Option<String> {
        if let response::response_event::Type::ClientActions(a) = e.r#type.as_ref().unwrap() {
            if let Some(action) = a.actions.first() {
                if let response::client_action::Action::AppendToMessageContent(ap) =
                    action.action.as_ref().unwrap()
                {
                    if let Some(msg) = ap.message.as_ref() {
                        if let warp_multi_agent_api::message::Message::AgentOutput(out) =
                            msg.message.as_ref().unwrap()
                        {
                            return Some(out.text.clone());
                        }
                    }
                }
            }
        }
        None
    };
    assert_eq!(extract_append_text(&events[2]), Some("Hello".into()));
    assert_eq!(extract_append_text(&events[3]), Some(" world".into()));
}
```

- [ ] **Step 2: Run the test**

Run: `cargo nextest run -p ai_provider --test openai_integration 2>&1 | tail -10`
Expected: 1 test passes.

If the test fails because of a path mismatch (e.g., the proto module structure differs from what's in the helpers), align the helper imports with what actually compiles in `openai.rs`.

- [ ] **Step 3: Commit**

```bash
cd /Users/dondy/Codes/warp
git add crates/ai_provider/tests/openai_integration.rs
git commit -m "test(ai_provider): mockito happy-path for OpenAI streaming"
```

---

### Task 9: Mockito integration test — error paths

**Files:**
- Modify: `crates/ai_provider/tests/openai_integration.rs`

- [ ] **Step 1: Append error-path tests**

```rust
#[tokio::test]
async fn returns_error_for_401_unauthorized() {
    let mut server = mockito::Server::new_async().await;
    let _m = server
        .mock("POST", "/chat/completions")
        .with_status(401)
        .with_body(r#"{"error":{"message":"Invalid API key","type":"invalid_request_error"}}"#)
        .create_async()
        .await;

    let adapter = OpenAiAdapter::new(OpenAiConfig {
        endpoint: server.url(),
        api_key: "sk-bad".into(),
        model: "gpt-4o-mini".into(),
    });
    // The error may surface either at chat_stream() or as the first stream item.
    match adapter.chat_stream(&make_request("hi")).await {
        Ok(mut stream) => {
            let first = stream.next().await.expect("at least one event");
            assert!(first.is_err(), "expected Err event, got {first:?}");
        }
        Err(_) => { /* error surfaced synchronously — also acceptable */ }
    }
}

#[tokio::test]
async fn errors_when_request_has_no_user_query() {
    let adapter = OpenAiAdapter::new(OpenAiConfig {
        endpoint: "http://localhost:1".into(),
        api_key: "sk-x".into(),
        model: "gpt-4o-mini".into(),
    });
    let req = Request::default(); // no input
    let err = adapter.chat_stream(&req).await.expect_err("err");
    assert!(format!("{err:#}").contains("Request.input is missing"));
}

#[tokio::test]
async fn errors_on_malformed_sse_chunk() {
    let mut server = mockito::Server::new_async().await;
    let body = format!("{}", sse_chunk("not valid json"));
    let _m = server
        .mock("POST", "/chat/completions")
        .with_status(200)
        .with_header("content-type", "text/event-stream")
        .with_body(body)
        .create_async()
        .await;

    let adapter = OpenAiAdapter::new(OpenAiConfig {
        endpoint: server.url(),
        api_key: "sk-test".into(),
        model: "gpt-4o-mini".into(),
    });
    let mut stream = adapter.chat_stream(&make_request("hi")).await.expect("stream");

    // First two events are Init + ClientActions opening — collect until error.
    let mut saw_error = false;
    while let Some(item) = stream.next().await {
        if item.is_err() {
            saw_error = true;
            break;
        }
    }
    assert!(saw_error, "expected error from malformed JSON");
}
```

- [ ] **Step 2: Run all integration tests**

Run: `cargo nextest run -p ai_provider --test openai_integration 2>&1 | tail -15`
Expected: 4 tests pass.

- [ ] **Step 3: Commit**

```bash
cd /Users/dondy/Codes/warp
git add crates/ai_provider/tests/openai_integration.rs
git commit -m "test(ai_provider): cover OpenAI adapter error paths"
```

---

### Task 10: Clippy + nextest + manual smoke

**Files:** none

- [ ] **Step 1: Clippy**

```bash
cargo clippy -p ai_provider -p warp --tests --all-targets -- -D warnings 2>&1 | tail -10
```
Expected: 0 errors. Fix in place.

- [ ] **Step 2: Nextest on touched crates**

```bash
cargo nextest run -p ai_provider -p warp --no-fail-fast 2>&1 | tail -15
```
Expected: no NEW failures vs. M1b-prep's baseline.

- [ ] **Step 3: Manual GUI smoke against a real OpenAI-compatible endpoint**

This step needs the human user. The user sets:

```sh
export WARP_AI_PROTOCOL=openai
export WARP_AI_OPENAI_API_KEY="sk-..."          # their key
export WARP_AI_OPENAI_MODEL="gpt-4o-mini"        # or any model their key has access to
export WARP_AI_OPENAI_ENDPOINT="https://api.openai.com/v1"  # or Ollama/OpenRouter/etc.
./script/run
```

In the GUI, open Agent Mode and send a simple prompt: *"what is 2+2?"*. Confirm:

1. A streaming text response appears.
2. The response is the model's actual answer (not a Warp-server response).
3. No "OpenAI adapter is not yet implemented" error.
4. App quits cleanly.

If the response is empty or garbled, the most common causes are:
- `WARP_AI_OPENAI_ENDPOINT` URL has a trailing slash or wrong path. The adapter expects `/chat/completions` to be appended; verify with `curl -X POST $WARP_AI_OPENAI_ENDPOINT/chat/completions -H "Authorization: Bearer $WARP_AI_OPENAI_API_KEY" -d '{"model":"...","messages":[{"role":"user","content":"hi"}]}'`.
- The transaction sequence is reaching the UI but the Block view isn't subscribed to AgentOutput updates. (Less likely — AgentOutput is the standard reply path.)

---

## Self-Review Checklist (run before declaring M1b-chat done)

- [ ] `cargo check --workspace` passes.
- [ ] `cargo clippy -p ai_provider -p warp --tests --all-targets -- -D warnings` clean.
- [ ] `cargo nextest run -p ai_provider -p warp` shows no new failures.
- [ ] `crates/ai_provider/src/openai.rs` exists and contains `OpenAiAdapter` impl of `AiProvider`.
- [ ] Unit tests cover: env-var loading (4), query extraction (3), request body shape (1), delta extraction (4), event-builder helpers (2). 14 unit tests minimum.
- [ ] Integration tests cover: happy path, 401 unauthorized, no user query, malformed SSE chunk. 4 integration tests minimum.
- [ ] Manual GUI smoke (Task 10 Step 3) confirms streaming text from a real OpenAI-compatible endpoint.

## Out of scope for M1b-chat (deferred)

- **Tool calling.** M1c — translate `Settings.supported_tools` → OpenAI `tools[]`, parse `tool_calls` deltas, handle `ToolCallResult` round-trip.
- **Other input variants.** `CodeReview`, `FetchReviewComments`, `GeneratePassiveSuggestions`, `InvokeSkill`, etc. — return a clear "not yet supported" error in M1b-chat. M1c+ may add those incrementally.
- **Multi-turn conversation continuity.** Each turn synthesizes fresh IDs; the conversation is effectively single-turn from the client's perspective. M1c will plumb conversation_id round-trip.
- **Retry policy.** The adapter does not retry on 5xx or rate limits today. M5 (capability/polish).
- **Capability detection runtime check.** No `supports_tools` config yet — Agent Mode tool buttons may appear but produce no-ops. M5.
- **Settings GUI.** M3 — env vars only for now.
- **System prompt construction from `TaskContext` / `Settings` / `MCPContext`.** A fixed minimal prompt is shipped in M1b-chat. Future plans may build a richer prompt from the request fields.
- **Anthropic adapter.** M2 — will follow this plan's pattern with Anthropic-specific JSON shape and SSE event names.

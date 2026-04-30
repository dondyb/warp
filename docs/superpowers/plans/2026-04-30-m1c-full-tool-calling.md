# M1c — Full Tool Calling for OpenAI Adapter

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add complete tool calling to the OpenAI adapter at `crates/ai_provider/src/openai.rs`. After this plan, the AI can invoke any of Warp's 18 client-executed tools (run shell commands, read/edit files, search code, use the computer, MCP, etc.) through the user's custom OpenAI-compatible endpoint, with proper round-trip handling of tool results.

**Architecture:** New module `crates/ai_provider/src/tools.rs` with a `ToolDefinition` trait that each tool implements. The trait has three responsibilities:

1. **`json_schema()`** — produce the OpenAI function schema for the tool (name, description, parameters JSON schema). Used to build the `tools[]` array in OpenAI requests.
2. **`decode_call_args(args: serde_json::Value) -> proto::ToolCall`** — translate a JSON tool-call from OpenAI into Warp's protobuf `ToolCall` variant. Used when streaming tool calls from the model.
3. **`encode_result_text(result: &proto::ToolCallResult) -> String`** — translate a Warp `ToolCallResult` (returned by the client after executing the tool) into the textual `content` of an OpenAI `role: tool` message. Used in the next request's messages array.

A central `ToolRegistry` maps each `ToolType` enum variant to its `ToolDefinition`. `OpenAiAdapter::build_request_body` consults the registry for every tool in `Request.settings.supported_tools` to build the `tools[]` array. The streaming SSE parser accumulates `tool_calls[*]` deltas across chunks and emits a `Message::ToolCall` event when a tool call is complete. On the next user turn, the dispatcher reads `Input::UserInputs[ToolCallResult{...}]` from `Request.input`, looks up the tool by id, and inserts an OpenAI `role: tool` message before the new user message.

**Tech Stack:** Rust 2021. New deps: none (existing reqwest + serde_json are sufficient). Tests: `mockito` (already in dev-deps).

---

## Context (from earlier survey + decisions)

- **18 client-executed tools** to implement (full list in Phase B + Phase C tasks below).
- **14 server-only tools** (SuggestPlan, SuggestCreatePlan, OpenCodeReview, InitProject, FetchConversation, StartAgent, SendMessageToAgent, AskUserQuestion, etc.) are NOT translated — they're orchestration concerns that the OSS fork doesn't run.
- **Proto location:** `~/.cargo/git/checkouts/warp-proto-apis-2098b3a955068931/78a78f2/apis/multi_agent/v1/task.proto` defines `enum ToolType` (lines 1547–1580), `message ToolCall` (lines 353–393), and `message ToolCallResult` (lines 884–933).
- **Existing adapter:** `crates/ai_provider/src/openai.rs` after M1b-chat handles text-only chat with single-turn (now multi-turn after M3+M4b's `StreamIds::for_request` fix). This plan extends it.
- **Existing tool conversion infra:** `crates/ai/src/agent/action/convert.rs` translates proto `ToolCall` → internal `AIAgentActionType`. **We do not depend on this** — we build a parallel translation in `ai_provider` for OpenAI-specific concerns. The two translations meet at the proto boundary.
- **OpenAI tool-calling protocol:**
  - Request `tools: [{ type: "function", function: { name, description, parameters: <json-schema> } }]`
  - Response delta: `choices[0].delta.tool_calls[i] = { index, id, type: "function", function: { name?, arguments? (partial JSON) } }` — accumulated across chunks.
  - Subsequent request includes the assistant's tool_call message + a `role: tool` message per result.

## File Structure

**Create:**

| Path | Responsibility |
|---|---|
| `crates/ai_provider/src/tools.rs` | `ToolDefinition` trait + `ToolRegistry` + helper types. |
| `crates/ai_provider/src/tools/run_shell_command.rs` | RunShellCommand tool impl. |
| `crates/ai_provider/src/tools/read_search.rs` | ReadFiles, ReadDocuments, Grep, FileGlobV2, ReadShellCommandOutput, ReadSkill, ReadMCPResource. |
| `crates/ai_provider/src/tools/edit_write.rs` | ApplyFileDiffs, EditDocuments, CreateDocuments, WriteToLongRunningShellCommand. |
| `crates/ai_provider/src/tools/misc.rs` | SearchCodebase, CallMCPTool, UseComputer, RequestComputerUse, InsertReviewComments, UploadFileArtifact. |
| `crates/ai_provider/tests/openai_tools.rs` | Mockito integration tests for tool calling. |

**Modify:**

| Path | Change |
|---|---|
| `crates/ai_provider/src/lib.rs` | Add `pub mod tools;` and re-exports. |
| `crates/ai_provider/src/openai.rs` | (1) `build_request_body` now adds `tools: [...]` array based on `Request.settings.supported_tools`. (2) `chat_stream`'s SSE parser accumulates `tool_calls` deltas → emits `Message::ToolCall` events. (3) Translate incoming `ToolCallResult` from `Request.input` into OpenAI `role: tool` messages. |

---

## Phase A: Foundation (5 tasks)

### Task 1: Create `tools` module skeleton + `ToolDefinition` trait

**Files:**
- Create: `crates/ai_provider/src/tools.rs`
- Modify: `crates/ai_provider/src/lib.rs`

- [ ] **Step 1: Create `tools.rs` with the trait + registry**

```rust
//! Tool calling support for the OpenAI adapter.
//!
//! Each Warp tool that the AI can invoke is implemented as a `ToolDefinition`.
//! The `ToolRegistry` maps `ToolType` enum variants to their definitions and
//! is consulted by `OpenAiAdapter::build_request_body` when constructing the
//! OpenAI `tools[]` array, by the SSE parser when assembling tool-call events,
//! and by the next-turn handler when translating `ToolCallResult` back to
//! OpenAI `role: tool` messages.
//!
//! Per-tool implementations live in submodules (`tools/run_shell_command.rs`,
//! etc.) and are registered via `ToolRegistry::default()`.

use std::sync::Arc;

use serde_json::Value;
use warp_multi_agent_api::message;
use warp_multi_agent_api::request::input;

use crate::AIApiError;

/// One tool the AI can invoke. Every client-executed Warp tool implements this.
pub trait ToolDefinition: Send + Sync {
    /// Stable identifier used in OpenAI's `tool_calls[].function.name` and
    /// the corresponding `role: tool` continuation. Snake_case; should not
    /// collide with other tool names.
    fn name(&self) -> &'static str;

    /// One-line natural-language description of what the tool does.
    /// Sent as `function.description` to OpenAI so the model knows when
    /// to use this tool.
    fn description(&self) -> &'static str;

    /// JSON Schema for the tool's arguments. Becomes
    /// `function.parameters` in the OpenAI `tools[]` entry.
    fn parameters_schema(&self) -> Value;

    /// Decode an OpenAI `tool_calls[].function.arguments` JSON string
    /// into a Warp protobuf `ToolCall` variant. Errors are mapped to
    /// `AIApiError::Other`.
    fn decode_call_args(
        &self,
        args: Value,
    ) -> std::result::Result<message::tool_call::Tool, Arc<AIApiError>>;

    /// Encode a Warp `ToolCallResult` (variant matching this tool) into
    /// the textual `content` of an OpenAI `role: tool` message. Most
    /// implementations serialize structured fields to JSON-shaped text;
    /// others use plain summaries.
    ///
    /// If the result variant doesn't match this tool, return an error.
    fn encode_result_text(
        &self,
        result: &input::tool_call_result::Result,
    ) -> std::result::Result<String, Arc<AIApiError>>;
}

/// Registry of all known tool definitions. Cheap to construct (uses static
/// references). The order of insertion does not matter — lookup is by name.
pub struct ToolRegistry {
    tools: Vec<&'static dyn ToolDefinition>,
}

impl Default for ToolRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl ToolRegistry {
    pub fn new() -> Self {
        let mut reg = Self { tools: Vec::new() };
        // Per-tool registrations get added by Phase B + Phase C tasks:
        //   reg.tools.push(&run_shell_command::TOOL);
        //   reg.tools.push(&read_search::READ_FILES);
        //   ... etc.
        reg
    }

    /// Look up a tool by its OpenAI function name.
    pub fn by_name(&self, name: &str) -> Option<&'static dyn ToolDefinition> {
        self.tools.iter().find(|t| t.name() == name).copied()
    }

    /// Return all registered tools as `tools[]` JSON entries for OpenAI.
    pub fn openai_tools_json(&self) -> Value {
        let entries: Vec<Value> = self
            .tools
            .iter()
            .map(|tool| {
                serde_json::json!({
                    "type": "function",
                    "function": {
                        "name": tool.name(),
                        "description": tool.description(),
                        "parameters": tool.parameters_schema(),
                    }
                })
            })
            .collect();
        Value::Array(entries)
    }

    /// Filter the registry to only the tools the client says it supports
    /// (per `Request.settings.supported_tools`). Returns a new registry
    /// containing only the tools whose `name()` corresponds to a `ToolType`
    /// in `supported`.
    ///
    /// This is the entry point used by `OpenAiAdapter::build_request_body`.
    pub fn filter_to_supported(
        &self,
        supported: &[i32],
    ) -> ToolRegistry {
        // Per-tool tasks add a `fn matches_tool_type(t: ToolType) -> bool`
        // to each tool. The registry checks each tool against the supported set.
        // Stub for Phase A — populated in Phase B onward.
        let _ = supported;
        ToolRegistry {
            tools: self.tools.clone(),
        }
    }
}

// Submodules — populated in Phase B + Phase C:
// pub mod run_shell_command;
// pub mod read_search;
// pub mod edit_write;
// pub mod misc;
```

> The "matches_tool_type" / "filter_to_supported" mechanism is intentionally stubbed in Phase A. The first concrete tool (Phase B's RunShellCommand) wires it up; subsequent tools follow that pattern.

- [ ] **Step 2: Re-export from `lib.rs`**

In `crates/ai_provider/src/lib.rs`, add near the existing `pub use openai::*;` line:

```rust
pub mod tools;
pub use tools::{ToolDefinition, ToolRegistry};
```

- [ ] **Step 3: Verify compile**

Run: `cargo check -p ai_provider 2>&1 | tail -5`
Expected: PASS, possibly with `unused` warnings on `tools` field. Acceptable.

- [ ] **Step 4: Add unit test for empty registry behavior**

In `crates/ai_provider/src/tools.rs` at the bottom:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_registry_emits_empty_tools_array() {
        let reg = ToolRegistry::new();
        assert_eq!(reg.openai_tools_json(), serde_json::json!([]));
    }

    #[test]
    fn empty_registry_lookup_misses() {
        let reg = ToolRegistry::new();
        assert!(reg.by_name("anything").is_none());
    }
}
```

Run: `cargo nextest run -p ai_provider --lib tools::tests`
Expected: 2 tests pass.

- [ ] **Step 5: Commit**

```bash
cd /Users/dondy/Codes/warp
git add crates/ai_provider/src/tools.rs crates/ai_provider/src/lib.rs
git commit -m "feat(ai_provider): scaffold ToolDefinition trait + ToolRegistry"
```

---

### Task 2: Wire `tools[]` into OpenAI request body

**Files:**
- Modify: `crates/ai_provider/src/openai.rs` (`build_request_body`)

- [ ] **Step 1: Read the current `build_request_body`**

Run: `grep -n "build_request_body" /Users/dondy/Codes/warp/crates/ai_provider/src/openai.rs | head -3` and read the function.

Currently it produces JSON like:
```json
{
  "model": "...",
  "stream": true,
  "messages": [{"role": "system", ...}, {"role": "user", ...}]
}
```

- [ ] **Step 2: Add tools array conditionally**

Modify `build_request_body` to include a `tools` array when `Request.settings.supported_tools` is non-empty:

```rust
pub(crate) fn build_request_body(
    &self,
    request: &Request,
) -> std::result::Result<serde_json::Value, Arc<AIApiError>> {
    let user_text = extract_user_query(request)?;
    let mut body = json!({
        "model": self.config.model,
        "stream": true,
        "messages": [
            { "role": "system", "content": SYSTEM_PROMPT },
            { "role": "user", "content": user_text }
        ]
    });

    // Add tools[] if the client declared supported tools.
    if let Some(settings) = request.settings.as_ref() {
        if !settings.supported_tools.is_empty() {
            let registry = crate::ToolRegistry::default()
                .filter_to_supported(&settings.supported_tools);
            let tools_json = registry.openai_tools_json();
            if let Value::Array(arr) = &tools_json {
                if !arr.is_empty() {
                    body["tools"] = tools_json;
                    // Allow the model to decide; alternative: "tool_choice": "auto" (default).
                }
            }
        }
    }

    Ok(body)
}
```

- [ ] **Step 3: Add a unit test**

Inside the existing `mod tests` block in `openai.rs`:

```rust
#[test]
fn build_request_body_omits_tools_when_supported_tools_empty() {
    let cfg = OpenAiConfig { /* same as other tests */ };
    let adapter = OpenAiAdapter::new(cfg);
    let req = build_request_with_query("hello");
    let body = adapter.build_request_body(&req).expect("body");
    assert!(body.get("tools").is_none(), "tools should be absent");
}
```

(Don't add a `tools_present` test yet — that requires registered tools, which Phase B adds.)

- [ ] **Step 4: Verify**

```bash
cargo check -p ai_provider 2>&1 | tail -5
cargo nextest run -p ai_provider --lib openai::tests
```

- [ ] **Step 5: Commit**

```bash
git add crates/ai_provider/src/openai.rs
git commit -m "feat(ai_provider): include tools[] in OpenAI request when supported"
```

---

### Task 3: Accumulate tool-call deltas in SSE parser

**Files:**
- Modify: `crates/ai_provider/src/openai.rs` (the `chat_stream` filter_map)

- [ ] **Step 1: Add a tool-call accumulator type**

Inside `crates/ai_provider/src/openai.rs`, near the top (with other helpers), add:

```rust
use std::collections::HashMap;

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
        let mut out: Vec<_> = self.inflight.values().cloned().collect();
        out.sort_by_key(|tc| {
            self.inflight
                .iter()
                .find(|(_, v)| v.id == tc.id)
                .map(|(k, _)| *k)
                .unwrap_or(0)
        });
        self.inflight.clear();
        out
    }
}
```

- [ ] **Step 2: Modify `chat_stream`'s SSE parser to ingest tool-call chunks**

In the `body_stream` filter_map closure, add a closure-captured accumulator (wrapped in `Arc<Mutex<...>>` since the closure captures by move and runs across futures):

```rust
let tool_accumulator = std::sync::Arc::new(std::sync::Mutex::new(ToolCallAccumulator::default()));
```

Then in the `Ok(reqwest_eventsource::Event::Message(msg))` arm, after parsing the JSON:

```rust
match parse_delta(&msg.data) {
    Ok(Some(delta_text)) => {
        // Existing text-delta handling
    }
    Ok(None) => {
        // Check for tool_calls delta or finish_reason
        if let Ok(chunk_json) = serde_json::from_str::<serde_json::Value>(&msg.data) {
            tool_accumulator.lock().unwrap().ingest_chunk(&chunk_json);

            // If finish_reason == "tool_calls", emit the assembled tool calls.
            let finish = chunk_json
                .pointer("/choices/0/finish_reason")
                .and_then(|v| v.as_str());
            if finish == Some("tool_calls") {
                let calls = tool_accumulator.lock().unwrap().drain_completed();
                if !calls.is_empty() {
                    let actions = calls
                        .into_iter()
                        .map(|tc| build_tool_call_action(&task_id, &tc))
                        .collect::<Vec<_>>();
                    return Some(Ok(build_client_actions(actions)));
                }
            }
        }
        None
    }
    Err(e) => Some(Err(e)),
}
```

- [ ] **Step 3: Add `build_tool_call_action` helper**

In the same file, near the other `action_*` builders:

```rust
/// Build a `ClientAction` that adds a `Message::ToolCall` to the conversation
/// task. The actual proto `ToolCall` variant is decoded by the registered
/// `ToolDefinition` for this tool name. If the tool name is unknown,
/// emit a "tool not supported" error message inline.
fn build_tool_call_action(
    task_id: &str,
    accumulated: &AccumulatedToolCall,
) -> response::ClientAction {
    let registry = crate::ToolRegistry::default();
    let tool = registry.by_name(&accumulated.name);

    let tool_variant = if let Some(tool) = tool {
        let args_value = serde_json::from_str::<serde_json::Value>(&accumulated.arguments)
            .unwrap_or_else(|_| serde_json::Value::Object(serde_json::Map::new()));
        match tool.decode_call_args(args_value) {
            Ok(t) => Some(t),
            Err(_) => None,
        }
    } else {
        None
    };

    let tool_call_msg = warp_multi_agent_api::Message {
        id: uuid::Uuid::new_v4().to_string(),
        task_id: task_id.to_string(),
        message: Some(message::Message::ToolCall(
            warp_multi_agent_api::message::ToolCall {
                tool_call_id: accumulated.id.clone(),
                tool: tool_variant,
                ..Default::default()
            },
        )),
        ..Default::default()
    };

    response::ClientAction {
        action: Some(response::client_action::Action::AddMessagesToTask(
            response::client_action::AddMessagesToTask {
                task_id: task_id.to_string(),
                messages: vec![tool_call_msg],
            },
        )),
    }
}
```

> The proto module path for `message::ToolCall` and `message::tool_call::Tool` may differ slightly — search `rg "message::ToolCall|message::tool_call" /Users/dondy/Codes/warp/app/src` if `cargo check` complains.

- [ ] **Step 4: Verify**

Run: `cargo check -p ai_provider 2>&1 | tail -5`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/ai_provider/src/openai.rs
git commit -m "feat(ai_provider): accumulate OpenAI tool_call deltas into Warp ToolCall events"
```

---

### Task 4: Translate incoming `ToolCallResult` into OpenAI `role: tool` messages

**Files:**
- Modify: `crates/ai_provider/src/openai.rs` (`build_request_body`, `extract_user_query`)

- [ ] **Step 1: Refactor `build_request_body` to handle multi-turn tool flow**

The current `build_request_body` calls `extract_user_query(request)` which only handles `UserQuery`. We need to ALSO handle `ToolCallResult` inputs — which appear in `Request.input.user_inputs[*]` after the model has called tools and the client has executed them.

Replace the body-construction logic so that **all** entries in `UserInputs.inputs` contribute to the `messages[]` array:

```rust
pub(crate) fn build_request_body(
    &self,
    request: &Request,
) -> std::result::Result<serde_json::Value, Arc<AIApiError>> {
    let mut messages: Vec<Value> = vec![
        json!({ "role": "system", "content": SYSTEM_PROMPT })
    ];

    // Walk the conversation history from the request and synthesize
    // OpenAI messages. For now, the simplest fidelity-preserving approach:
    //   - The most-recent UserQuery → role: user
    //   - Each ToolCallResult preceding it → role: tool
    //
    // Future enhancement: walk Task.messages from task_context to
    // reconstruct full history. For M1c MVP, single-turn tool round-trip
    // works because OpenAI itself maintains state via the assistant's
    // tool_calls in the previous response — but since we don't echo the
    // assistant's tool_call message in subsequent turns (we'd need to
    // persist them), reconstruct from request.task_context for fidelity.
    let registry = crate::ToolRegistry::default();
    let messages_from_request =
        build_messages_from_request(request, &registry)?;
    messages.extend(messages_from_request);

    let mut body = json!({
        "model": self.config.model,
        "stream": true,
        "messages": messages,
    });

    if let Some(settings) = request.settings.as_ref() {
        if !settings.supported_tools.is_empty() {
            let supported_registry = registry.filter_to_supported(&settings.supported_tools);
            let tools_json = supported_registry.openai_tools_json();
            if let Value::Array(arr) = &tools_json {
                if !arr.is_empty() {
                    body["tools"] = tools_json;
                }
            }
        }
    }

    Ok(body)
}

/// Walk `request.task_context.tasks[*].messages[*]` to reconstruct the
/// conversation history as OpenAI messages (role: user, role: assistant,
/// role: tool). Falls back to extracting just the latest user query if
/// no task context is available.
fn build_messages_from_request(
    request: &Request,
    registry: &crate::ToolRegistry,
) -> std::result::Result<Vec<Value>, Arc<AIApiError>> {
    let mut out: Vec<Value> = Vec::new();

    // Walk past tasks for history.
    if let Some(tc) = request.task_context.as_ref() {
        for task in &tc.tasks {
            for msg in &task.messages {
                use warp_multi_agent_api::message;
                if let Some(variant) = msg.message.as_ref() {
                    match variant {
                        message::Message::UserQuery(uq) => {
                            out.push(json!({ "role": "user", "content": uq.query }));
                        }
                        message::Message::AgentOutput(ao) => {
                            out.push(json!({ "role": "assistant", "content": ao.text }));
                        }
                        message::Message::ToolCall(tc_msg) => {
                            // Reconstruct the assistant's tool_call message.
                            // The tool variant tells us name + args.
                            if let Some(tool_variant) = tc_msg.tool.as_ref() {
                                if let Some(tool_def) =
                                    registry.tool_for_proto(tool_variant)
                                {
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
                        message::Message::ToolCallResult(_)
                        | _ => {
                            // Other variants don't have direct OpenAI representations.
                        }
                    }
                }
            }
        }
    }

    // Then append the current input.
    if let Some(input) = request.input.as_ref() {
        if let Some(input_type) = input.r#type.as_ref() {
            use warp_multi_agent_api::request::input;
            match input_type {
                input::Type::UserInputs(user_inputs) => {
                    for ui in &user_inputs.inputs {
                        match ui.input.as_ref() {
                            Some(input::user_inputs::user_input::Input::UserQuery(uq)) => {
                                out.push(json!({ "role": "user", "content": uq.query }));
                            }
                            Some(input::user_inputs::user_input::Input::ToolCallResult(tcr)) => {
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
                                        // Unknown tool result variant — represent as a
                                        // generic error so the model knows the call failed.
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
                input::Type::UserQuery(uq) => {
                    out.push(json!({ "role": "user", "content": uq.query }));
                }
                _ => {}
            }
        }
    }

    Ok(out)
}
```

- [ ] **Step 2: Add `tool_for_proto`, `tool_for_proto_result`, and `encode_call_args` to the `ToolDefinition` trait + registry**

In `crates/ai_provider/src/tools.rs`, extend the trait:

```rust
pub trait ToolDefinition: Send + Sync {
    // ... existing methods ...

    /// True iff this tool corresponds to the given proto `ToolCall` variant.
    fn matches_proto_call(&self, tool: &message::tool_call::Tool) -> bool;

    /// True iff this tool corresponds to the given proto `ToolCallResult` variant.
    fn matches_proto_result(&self, result: &input::tool_call_result::Result) -> bool;

    /// Encode the proto `ToolCall::Tool` variant back into the JSON args form
    /// (used when reconstructing the assistant's previous tool_calls in
    /// multi-turn requests). Default: return empty object.
    fn encode_call_args(&self, _tool: &message::tool_call::Tool) -> Value {
        Value::Object(serde_json::Map::new())
    }
}
```

And on `ToolRegistry`:

```rust
impl ToolRegistry {
    pub fn tool_for_proto(&self, tool: &message::tool_call::Tool) -> Option<&'static dyn ToolDefinition> {
        self.tools.iter().find(|t| t.matches_proto_call(tool)).copied()
    }

    pub fn tool_for_proto_result(&self, r: &input::tool_call_result::Result) -> Option<&'static dyn ToolDefinition> {
        self.tools.iter().find(|t| t.matches_proto_result(r)).copied()
    }
}
```

- [ ] **Step 3: Verify**

Run: `cargo check -p ai_provider 2>&1 | tail -5`
Expected: PASS (with warnings about `_ => {}` arms — acceptable).

- [ ] **Step 4: Commit**

```bash
git add crates/ai_provider/src/openai.rs crates/ai_provider/src/tools.rs
git commit -m "feat(ai_provider): translate ToolCallResult to OpenAI role:tool messages"
```

---

### Task 5: Update `extract_user_query` to be optional / non-fatal for tool-only inputs

**Files:**
- Modify: `crates/ai_provider/src/openai.rs`

- [ ] **Step 1: Soften the error**

The original `extract_user_query` errors when a request has no UserQuery. Now that we walk all messages in `build_messages_from_request`, the absence of a UserQuery is not an error — it just means this request is a tool-result continuation.

Change `extract_user_query` to return `Option<String>` instead of `Result`, and only error if the request has no input AT ALL:

```rust
pub(crate) fn extract_user_query(request: &Request) -> Option<String> {
    let input = request.input.as_ref()?;
    use warp_multi_agent_api::request::input;
    let input_type = input.r#type.as_ref()?;
    match input_type {
        input::Type::UserInputs(user_inputs) => user_inputs.inputs.iter().rev().find_map(|ui| {
            match ui.input.as_ref() {
                Some(input::user_inputs::user_input::Input::UserQuery(uq)) => {
                    Some(uq.query.clone())
                }
                _ => None,
            }
        }),
        input::Type::UserQuery(uq) => Some(uq.query.clone()),
        _ => None,
    }
}
```

Update its callers (test code) to handle the new signature.

- [ ] **Step 2: Verify + commit**

```bash
cargo check -p ai_provider 2>&1 | tail -5
cargo nextest run -p ai_provider --lib openai::tests
```

```bash
git add crates/ai_provider/src/openai.rs
git commit -m "refactor(ai_provider): extract_user_query returns Option for tool-only flows"
```

---

## Phase B: RunShellCommand MVP (1 task)

### Task 6: Implement RunShellCommand tool

**Files:**
- Create: `crates/ai_provider/src/tools/run_shell_command.rs`
- Modify: `crates/ai_provider/src/tools.rs` (declare submodule + register)

- [ ] **Step 1: Create `tools/run_shell_command.rs`**

```rust
//! RunShellCommand tool — execute a shell command on the user's machine.

use serde_json::{json, Value};
use std::sync::Arc;
use warp_multi_agent_api::message;
use warp_multi_agent_api::message::tool_call::run_shell_command::RiskCategory;
use warp_multi_agent_api::request::input;

use crate::tools::ToolDefinition;
use crate::AIApiError;

pub static TOOL: RunShellCommandTool = RunShellCommandTool;

pub struct RunShellCommandTool;

impl ToolDefinition for RunShellCommandTool {
    fn name(&self) -> &'static str {
        "run_shell_command"
    }

    fn description(&self) -> &'static str {
        "Execute a shell command on the user's machine and return its output. \
         Use for any operation that needs the local shell — running scripts, \
         querying system state, building, testing. Set risk_category to \
         indicate the command's destructiveness (read_only, trivial_local_change, \
         nontrivial_local_change, external_change, risky)."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "command": {
                    "type": "string",
                    "description": "The exact shell command to execute."
                },
                "risk_category": {
                    "type": "string",
                    "enum": [
                        "read_only",
                        "trivial_local_change",
                        "nontrivial_local_change",
                        "external_change",
                        "risky"
                    ],
                    "description": "How destructive the command is. Use read_only \
                                    for queries (ls, cat, grep), trivial_local_change \
                                    for safe edits, risky for irreversible operations."
                },
                "uses_pager": {
                    "type": "boolean",
                    "description": "True if the command uses an interactive pager (less, more)."
                },
                "wait_until_complete": {
                    "type": "boolean",
                    "description": "True (default) for short-running commands. False for \
                                    long-running commands where the user wants live output."
                }
            },
            "required": ["command", "risk_category"]
        })
    }

    fn decode_call_args(
        &self,
        args: Value,
    ) -> std::result::Result<message::tool_call::Tool, Arc<AIApiError>> {
        let command = args
            .get("command")
            .and_then(|v| v.as_str())
            .ok_or_else(|| {
                Arc::new(AIApiError::Other(anyhow::anyhow!(
                    "run_shell_command: missing required `command` argument"
                )))
            })?
            .to_string();
        let risk_category = match args.get("risk_category").and_then(|v| v.as_str()) {
            Some("read_only") => RiskCategory::ReadOnly,
            Some("trivial_local_change") => RiskCategory::TrivialLocalChange,
            Some("nontrivial_local_change") => RiskCategory::NontrivialLocalChange,
            Some("external_change") => RiskCategory::ExternalChange,
            Some("risky") => RiskCategory::Risky,
            _ => RiskCategory::Unspecified,
        };
        let uses_pager = args
            .get("uses_pager")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let wait_until_complete = args
            .get("wait_until_complete")
            .and_then(|v| v.as_bool())
            .unwrap_or(true);

        Ok(message::tool_call::Tool::RunShellCommand(
            message::tool_call::RunShellCommand {
                command,
                risk_category: risk_category.into(),
                uses_pager,
                wait_until_complete_value: Some(
                    message::tool_call::run_shell_command::WaitUntilCompleteValue::WaitUntilComplete(
                        wait_until_complete,
                    ),
                ),
                ..Default::default()
            },
        ))
    }

    fn encode_call_args(&self, tool: &message::tool_call::Tool) -> Value {
        if let message::tool_call::Tool::RunShellCommand(rsc) = tool {
            json!({
                "command": rsc.command,
                "risk_category": match RiskCategory::try_from(rsc.risk_category)
                    .unwrap_or(RiskCategory::Unspecified)
                {
                    RiskCategory::ReadOnly => "read_only",
                    RiskCategory::TrivialLocalChange => "trivial_local_change",
                    RiskCategory::NontrivialLocalChange => "nontrivial_local_change",
                    RiskCategory::ExternalChange => "external_change",
                    RiskCategory::Risky => "risky",
                    _ => "read_only",
                },
                "uses_pager": rsc.uses_pager,
            })
        } else {
            Value::Object(Default::default())
        }
    }

    fn encode_result_text(
        &self,
        result: &input::tool_call_result::Result,
    ) -> std::result::Result<String, Arc<AIApiError>> {
        if let input::tool_call_result::Result::RunShellCommand(rsc_result) = result {
            // RunShellCommandResult has oneof { LongRunningShellCommandSnapshot, ShellCommandFinished, PermissionDenied }.
            // Render each as text.
            use input::tool_call_result::run_shell_command_result::Result as Inner;
            match rsc_result.result.as_ref() {
                Some(Inner::ShellCommandFinished(finished)) => Ok(format!(
                    "exit_code: {}\noutput:\n{}",
                    finished.exit_code, finished.output
                )),
                Some(Inner::LongRunningShellCommandSnapshot(snap)) => Ok(format!(
                    "(long-running snapshot, command_id: {})\n{}",
                    snap.command_id, snap.output_snapshot
                )),
                Some(Inner::PermissionDenied(_)) => {
                    Ok("permission_denied: user did not approve this command".to_string())
                }
                None => Ok("(no result)".to_string()),
            }
        } else {
            Err(Arc::new(AIApiError::Other(anyhow::anyhow!(
                "run_shell_command: result variant mismatch"
            ))))
        }
    }

    fn matches_proto_call(&self, tool: &message::tool_call::Tool) -> bool {
        matches!(tool, message::tool_call::Tool::RunShellCommand(_))
    }

    fn matches_proto_result(&self, result: &input::tool_call_result::Result) -> bool {
        matches!(result, input::tool_call_result::Result::RunShellCommand(_))
    }
}
```

> Proto module path verification: search for `RiskCategory` and adjust import path:
> ```bash
> rg "RiskCategory|enum.*Risk" /Users/dondy/.cargo/git/checkouts/warp-proto-apis-* | head -5
> ```
> Field names in `RunShellCommandResult` may differ — verify in `task.proto` and adjust.

- [ ] **Step 2: Register in `tools.rs`**

In `crates/ai_provider/src/tools.rs`:

```rust
pub mod run_shell_command;

impl ToolRegistry {
    pub fn new() -> Self {
        Self {
            tools: vec![&run_shell_command::TOOL],
        }
    }
}
```

- [ ] **Step 3: Add a unit test for the schema + decode round-trip**

In `crates/ai_provider/src/tools/run_shell_command.rs` at the bottom:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schema_has_required_fields() {
        let schema = TOOL.parameters_schema();
        let required = schema.get("required").and_then(|v| v.as_array()).unwrap();
        assert!(required.iter().any(|v| v.as_str() == Some("command")));
        assert!(required.iter().any(|v| v.as_str() == Some("risk_category")));
    }

    #[test]
    fn decode_minimal_call() {
        let args = json!({
            "command": "ls",
            "risk_category": "read_only"
        });
        let tool = TOOL.decode_call_args(args).expect("decode");
        match tool {
            message::tool_call::Tool::RunShellCommand(rsc) => {
                assert_eq!(rsc.command, "ls");
                assert_eq!(rsc.risk_category, RiskCategory::ReadOnly as i32);
            }
            _ => panic!("expected RunShellCommand variant"),
        }
    }

    #[test]
    fn decode_missing_command_errors() {
        let args = json!({"risk_category": "read_only"});
        let err = TOOL.decode_call_args(args).expect_err("err");
        assert!(format!("{err:#}").contains("missing required `command`"));
    }
}
```

- [ ] **Step 4: Verify**

```bash
cargo check -p ai_provider 2>&1 | tail -5
cargo nextest run -p ai_provider --lib tools::run_shell_command::tests
```

- [ ] **Step 5: Commit**

```bash
git add crates/ai_provider/src/tools.rs crates/ai_provider/src/tools/run_shell_command.rs
git commit -m "feat(ai_provider): implement RunShellCommand tool definition"
```

---

## Phase C: Tool Fanout (3 tasks)

### Task 7: Read/Search tools (7 tools)

**Files:**
- Create: `crates/ai_provider/src/tools/read_search.rs`
- Modify: `crates/ai_provider/src/tools.rs`

This task implements 7 tools that all share a "read or search" purpose. Follow the RunShellCommand pattern verbatim. Each tool below lists its parameter schema, proto path, and result schema.

- [ ] **Step 1: Create `tools/read_search.rs` with all 7 tool definitions**

The skeleton:

```rust
//! Read and search tools: ReadFiles, ReadDocuments, Grep, FileGlobV2,
//! ReadShellCommandOutput, ReadSkill, ReadMCPResource.

use serde_json::{json, Value};
use std::sync::Arc;
use warp_multi_agent_api::message;
use warp_multi_agent_api::request::input;

use crate::tools::ToolDefinition;
use crate::AIApiError;

pub static READ_FILES: ReadFilesTool = ReadFilesTool;
pub static READ_DOCUMENTS: ReadDocumentsTool = ReadDocumentsTool;
pub static GREP: GrepTool = GrepTool;
pub static FILE_GLOB_V2: FileGlobV2Tool = FileGlobV2Tool;
pub static READ_SHELL_COMMAND_OUTPUT: ReadShellCommandOutputTool = ReadShellCommandOutputTool;
pub static READ_SKILL: ReadSkillTool = ReadSkillTool;
pub static READ_MCP_RESOURCE: ReadMcpResourceTool = ReadMcpResourceTool;

// 7 struct definitions + impl blocks below.
```

For EACH tool, implement the `ToolDefinition` trait with:

#### 7a. ReadFiles
- **name:** `"read_files"`
- **description:** `"Read the contents of one or more files."`
- **parameters:** `{ files: [{ name: string, line_ranges?: [{ start: int, end: int }] }] }`, required: `["files"]`
- **decode → `message::tool_call::Tool::ReadFiles(ReadFiles { files: [File { name, line_ranges }] })`**
- **encode_result:** `input::tool_call_result::Result::ReadFiles` — render `TextFilesSuccess` as concatenated `name:\n<content>` per file; `Error` as the error message.

#### 7b. ReadDocuments
- **name:** `"read_documents"`
- **description:** `"Read Warp Drive documents by ID."`
- **parameters:** `{ documents: [{ document_id: string, line_ranges?: [...] }] }`, required: `["documents"]`
- **decode → `message::tool_call::Tool::ReadDocuments(...)`**
- **encode_result:** `Result::ReadDocuments` — render success as concatenated document contents.

#### 7c. Grep
- **name:** `"grep"`
- **description:** `"Search for patterns in files using regex."`
- **parameters:** `{ queries: [string], path: string }`, required: `["queries", "path"]`
- **decode → `message::tool_call::Tool::Grep(Grep { queries, path })`**
- **encode_result:** `Result::Grep` — render matches as `path:line: <line_text>`.

#### 7d. FileGlobV2
- **name:** `"file_glob_v2"`
- **description:** `"Find files matching glob patterns."`
- **parameters:** `{ patterns: [string], search_dir: string, max_matches?: int, max_depth?: int, min_depth?: int }`, required: `["patterns", "search_dir"]`
- **decode → `message::tool_call::Tool::FileGlobV2(FileGlobV2 { ... })`**
- **encode_result:** `Result::FileGlobV2` — render matches as a newline-separated path list.

#### 7e. ReadShellCommandOutput
- **name:** `"read_shell_command_output"`
- **description:** `"Read output from a long-running shell command."`
- **parameters:** `{ command_id: string, delay_ms?: int }`, required: `["command_id"]`
- **decode → `message::tool_call::Tool::ReadShellCommandOutput(...)`**
- **encode_result:** `Result::ReadShellCommandOutput` — render `(snapshot|finished|error)` analogously to RunShellCommand's result encoding.

#### 7f. ReadSkill
- **name:** `"read_skill"`
- **description:** `"Read a Warp Skill (instructions/prompt template)."`
- **parameters:** `{ name: string, skill_path?: string, bundled_skill_id?: string }`, required: `["name"]`
- **decode → `message::tool_call::Tool::ReadSkill(...)`**
- **encode_result:** `Result::ReadSkill` — render the skill content.

#### 7g. ReadMCPResource
- **name:** `"read_mcp_resource"`
- **description:** `"Read a resource from a connected MCP server."`
- **parameters:** `{ uri: string, server_id?: string }`, required: `["uri"]`
- **decode → `message::tool_call::Tool::ReadMcpResource(...)`** (note: prost-rust naming may be `ReadMcpResource` — verify)
- **encode_result:** `Result::ReadMcpResource` — render content as text.

For each tool, follow the **exact pattern from RunShellCommand**:
- A `fn decode_call_args` returning `Ok(message::tool_call::Tool::<Variant>(<message::tool_call::<Variant> { fields }>))`
- A `fn encode_result_text` matching on `input::tool_call_result::Result::<Variant>` and rendering its sub-fields
- A `fn matches_proto_call` and `fn matches_proto_result` using `matches!`
- A simple `fn encode_call_args` (default empty object is acceptable for tools where the model rarely revisits the call site)

- [ ] **Step 2: Register all 7 in `tools.rs`**

```rust
pub mod read_search;

impl ToolRegistry {
    pub fn new() -> Self {
        Self {
            tools: vec![
                &run_shell_command::TOOL,
                &read_search::READ_FILES,
                &read_search::READ_DOCUMENTS,
                &read_search::GREP,
                &read_search::FILE_GLOB_V2,
                &read_search::READ_SHELL_COMMAND_OUTPUT,
                &read_search::READ_SKILL,
                &read_search::READ_MCP_RESOURCE,
            ],
        }
    }
}
```

- [ ] **Step 3: Add minimal unit tests per tool**

For each tool, add ONE test that decodes a minimal valid args object and asserts the proto variant. ~7 tests total. Skip schema-shape tests (covered by the trait); skip exhaustive result-encoding tests (covered by the integration test in Phase D).

- [ ] **Step 4: Verify**

```bash
cargo check -p ai_provider 2>&1 | tail -5
cargo nextest run -p ai_provider --lib tools 2>&1 | tail -10
```

- [ ] **Step 5: Commit**

```bash
git add crates/ai_provider/src/tools.rs crates/ai_provider/src/tools/read_search.rs
git commit -m "feat(ai_provider): implement 7 read/search tools"
```

---

### Task 8: Edit/Write tools (4 tools)

**Files:**
- Create: `crates/ai_provider/src/tools/edit_write.rs`
- Modify: `crates/ai_provider/src/tools.rs`

Implement 4 tools following the same pattern.

#### 8a. ApplyFileDiffs
- **name:** `"apply_file_diffs"`
- **description:** `"Edit, create, or delete files via search/replace diffs."`
- **parameters:** `{ summary: string, diffs?: [{ file_path, search, replace }], new_files?: [{ file_path, content }], deleted_files?: [{ file_path }] }`, required: `["summary"]`
- **decode → `Tool::ApplyFileDiffs(ApplyFileDiffs { summary, diffs, new_files, deleted_files, ..Default::default() })`**
- **encode_result:** `Result::ApplyFileDiffs` — render `Success` as `applied: <updated_files>\ndeleted: <deleted_files>`, `Error` as the error.

#### 8b. EditDocuments
- **name:** `"edit_documents"`
- **description:** `"Edit Warp Drive documents via search/replace."`
- **parameters:** `{ diffs: [{ document_id, search, replace }] }`, required: `["diffs"]`
- **decode → `Tool::EditDocuments(...)`**
- **encode_result:** Success → list updated document IDs; Error → message.

#### 8c. CreateDocuments
- **name:** `"create_documents"`
- **description:** `"Create new Warp Drive documents."`
- **parameters:** `{ new_documents: [{ title, content }] }`, required: `["new_documents"]`
- **decode → `Tool::CreateDocuments(...)`**
- **encode_result:** Success → list new document IDs; Error → message.

#### 8d. WriteToLongRunningShellCommand
- **name:** `"write_to_long_running_shell_command"`
- **description:** `"Send input to a long-running shell command (REPL or interactive process)."`
- **parameters:** `{ command_id: string, input: string, mode?: "raw"|"line"|"block" }`, required: `["command_id", "input"]`
- **decode → `Tool::WriteToLongRunningShellCommand(...)`** (note: input is `bytes` in proto — encode as UTF-8)
- **encode_result:** `Result::WriteToLongRunningShellCommand` — render the resulting snapshot/finished state similar to RunShellCommand.

Same pattern: `name`, `description`, `parameters_schema`, `decode_call_args`, `encode_result_text`, `matches_proto_*`.

- [ ] **Step 1: Create `tools/edit_write.rs` with all 4 implementations.**
- [ ] **Step 2: Register all 4 in `tools.rs::ToolRegistry::new`.**
- [ ] **Step 3: One decode test per tool.**
- [ ] **Step 4: Verify.**
- [ ] **Step 5: Commit:** `feat(ai_provider): implement 4 edit/write tools`.

---

### Task 9: Misc tools (6 tools)

**Files:**
- Create: `crates/ai_provider/src/tools/misc.rs`
- Modify: `crates/ai_provider/src/tools.rs`

Implement 6 tools.

#### 9a. SearchCodebase
- **name:** `"search_codebase"`
- **description:** `"Semantic search across the user's indexed codebase."`
- **parameters:** `{ query: string, path_filters?: [string], codebase_path?: string }`, required: `["query"]`
- **decode → `Tool::SearchCodebase(...)`**
- **encode_result:** Success → list `path:lines` snippets; Error → message.

#### 9b. CallMCPTool
- **name:** `"call_mcp_tool"`
- **description:** `"Invoke a tool exposed by a connected MCP server. The args are an arbitrary JSON object specific to the tool."`
- **parameters:** `{ name: string, args: object, server_id?: string }`, required: `["name", "args"]`
- **decode → `Tool::CallMcpTool(...)`** (the `args: google.protobuf.Struct` is constructed from the JSON object — use `prost_types::Struct` conversion)
- **encode_result:** `Result::CallMcpTool` — render `Success` as JSON; `Error` as message.

#### 9c. UseComputer
- **name:** `"use_computer"`
- **description:** `"Send mouse/keyboard actions to the computer (click, type, drag, scroll, screenshot)."`
- **parameters:** `{ actions: [{ type: string, ... }], action_summary: string }`, required: `["actions", "action_summary"]`
- **decode → `Tool::UseComputer(...)`** — each action's JSON shape mirrors the proto's `Action` oneof; for MVP, support `click`, `type`, `wait` and skip the more exotic ones (return error if encountered).
- **encode_result:** Success → `screenshot` reference + cursor pos; Error → message.

#### 9d. RequestComputerUse
- **name:** `"request_computer_use"`
- **description:** `"Ask the user for permission to drive their computer (mouse/keyboard)."`
- **parameters:** `{ task_summary: string }`, required: `["task_summary"]`
- **decode → `Tool::RequestComputerUse(...)`**
- **encode_result:** Approved → "approved" + screen dims; Rejected → "user_rejected"; Error → message.

#### 9e. InsertReviewComments
- **name:** `"insert_review_comments"`
- **description:** `"Insert PR review comments on a code review."`
- **parameters:** `{ repo_path: string, base_branch: string, comments: [{ file_path, line, body, ... }] }`, required: `["repo_path", "comments"]`
- **decode → `Tool::InsertReviewComments(...)`**
- **encode_result:** Success → "<n> comments inserted"; Error → message.

#### 9f. UploadFileArtifact
- **name:** `"upload_file_artifact"`
- **description:** `"Upload a local file as a conversation artifact."`
- **parameters:** `{ file: { path: string }, description: string }`, required: `["file", "description"]`
- **decode → `Tool::UploadFileArtifact(...)`**
- **encode_result:** Success → `artifact_uid` + size; Error → message.

- [ ] **Step 1: Create `tools/misc.rs` with all 6 implementations.**
- [ ] **Step 2: Register all 6 in `tools.rs::ToolRegistry::new`.**
- [ ] **Step 3: One decode test per tool. UseComputer's complex `actions` array needs a slightly more involved test — at minimum, verify a `click` action decodes correctly.**
- [ ] **Step 4: Verify.**
- [ ] **Step 5: Commit:** `feat(ai_provider): implement 6 miscellaneous tools`.

---

## Phase D: Edge Cases & Tests (3 tasks)

### Task 10: Multi-tool-call in a single response

**Files:**
- Modify: `crates/ai_provider/src/openai.rs`

When the model emits multiple tool calls in one assistant message (e.g., parallel calls to `read_files` and `grep`), our SSE accumulator must produce ONE `ClientAction::AddMessagesToTask` containing multiple `Message::ToolCall` messages — not separate actions per tool.

- [ ] **Step 1: Confirm the existing accumulator handles this**

The `ToolCallAccumulator::ingest_chunk` already handles multiple `tool_calls[i]` indices. `drain_completed` returns all of them. The `body_stream` filter_map maps them to a single `build_client_actions(actions)` call.

Verify with a unit test: feed two synthetic chunks (one for each tool) and confirm both end up in the drained list.

- [ ] **Step 2: Add the unit test**

```rust
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
```

- [ ] **Step 3: Verify**

```bash
cargo nextest run -p ai_provider --lib openai::tests::accumulator_handles_two_parallel_tool_calls
```

- [ ] **Step 4: Commit**

```bash
git add crates/ai_provider/src/openai.rs
git commit -m "test(ai_provider): cover parallel tool calls"
```

---

### Task 11: Tool call error handling (model emits invalid JSON / unknown tool)

**Files:**
- Modify: `crates/ai_provider/src/openai.rs`

- [ ] **Step 1: Verify graceful degradation on malformed args**

`build_tool_call_action` already falls back to `tool_variant: None` when `decode_call_args` fails. Check that the resulting `Message::ToolCall` is still well-formed (just lacks the `tool` field) and document this behavior.

- [ ] **Step 2: Add a test for unknown-tool handling**

```rust
#[test]
fn unknown_tool_name_emits_message_without_tool_variant() {
    // ... feed a chunk with function name "made_up_tool"
    // ... assert the accumulator drains it
    // ... assert build_tool_call_action returns ClientAction::AddMessagesToTask
    //     where Message::ToolCall { tool_call_id: "...", tool: None }
}
```

- [ ] **Step 3: Add a test for malformed JSON args**

```rust
#[test]
fn malformed_args_falls_back_to_empty_object() {
    // feed function arguments of "{not json"
    // assert decode_call_args was called with an empty Value
    // assert tool variant is the corresponding default
}
```

- [ ] **Step 4: Verify and commit**

```bash
cargo check -p ai_provider 2>&1 | tail -3
cargo nextest run -p ai_provider --lib 2>&1 | tail -10
git add crates/ai_provider/src/openai.rs
git commit -m "test(ai_provider): error-path coverage for tool call decoding"
```

---

### Task 12: Mockito integration test — full tool round-trip

**Files:**
- Create or extend: `crates/ai_provider/tests/openai_tools.rs`

- [ ] **Step 1: Create the integration test**

Stage 1: feed the adapter a chat request that should produce a tool call (e.g., user query "list files in /tmp"). Mock OpenAI returns SSE chunks with a `tool_calls` delta for `run_shell_command`. Assert the adapter emits a `ResponseEvent::ClientActions` containing `AddMessagesToTask` with a `Message::ToolCall { tool_call_id, tool: Some(RunShellCommand) }`.

Stage 2: feed the adapter a follow-up request that includes the tool result (`Input::UserInputs[ToolCallResult { tool_call_id, result: RunShellCommand(ShellCommandFinished { exit_code: 0, output: "file_a\nfile_b" }) }]`). Mock OpenAI returns text deltas. Assert the adapter (a) sends an OpenAI `role: tool` message with `tool_call_id` and `content: "exit_code: 0\noutput:\nfile_a\nfile_b"`, then (b) streams the assistant's text response back.

```rust
//! Integration tests for tool calling.
use ai_provider::{AiProvider, OpenAiAdapter, OpenAiConfig};
// ... etc
```

- [ ] **Step 2: Verify**

```bash
cargo nextest run -p ai_provider --test openai_tools
```

- [ ] **Step 3: Commit**

```bash
git add crates/ai_provider/tests/openai_tools.rs
git commit -m "test(ai_provider): mockito coverage for tool call round-trip"
```

---

### Task 13: Manual GUI smoke + clippy + nextest

**Files:** none

- [ ] **Step 1: Clippy**

```bash
cargo clippy -p ai_provider -p warp --tests --all-targets -- -D warnings
```

- [ ] **Step 2: Nextest**

```bash
cargo nextest run -p ai_provider -p warp --no-fail-fast
```

- [ ] **Step 3: Manual GUI smoke**

Run `./script/run`. Open Settings → AI Provider, confirm config is set. Open Agent Mode and ask:

- "Run `ls /tmp` and tell me what's there." — should trigger `run_shell_command`, execute, and respond.
- "Read the contents of `Cargo.toml`." — should trigger `read_files`.
- "Search for the string 'AiProviderSettings' in this repo." — should trigger `grep` or `search_codebase`.

If any tool fails, check the log at `~/Library/Logs/warp-oss.log` for `[ai_provider::openai]` entries.

---

## Self-Review Checklist (run before declaring M1c done)

- [ ] All 18 client-executed tools registered in `ToolRegistry::new`.
- [ ] `cargo check -p ai_provider` clean.
- [ ] `cargo clippy -p ai_provider -p warp --tests --all-targets -- -D warnings` clean.
- [ ] All unit tests + integration tests pass; no new failures vs. M3+M4b baseline.
- [ ] Manual smoke at Task 13 confirms shell command execution end-to-end.
- [ ] Multi-turn tool round-trip works (request includes ToolCallResult → adapter sends OpenAI role:tool message → model responds).
- [ ] Multiple tool calls in a single response work (Task 10).
- [ ] Unknown tool / malformed args degrade gracefully (Task 11).

## Out of scope (deferred)

- **The 14 server-only tools** (SuggestPlan, OpenCodeReview, StartAgent, AskUserQuestion, etc.) are intentionally NOT translated — they're orchestration concerns the OSS fork doesn't run.
- **Streaming partial tool-call results** to the UI as they arrive (we emit complete tool calls only, after `finish_reason == "tool_calls"`).
- **Tool result truncation** for very large outputs — current implementation passes the full text through. If OpenAI's context limit is hit, the user sees a 400 error.
- **Tool capability auto-detection** — if the user's endpoint doesn't support tool calling at all, requests with `tools[]` will return 400 errors. We don't auto-fall-back to text-only. That's M5 (capability detection).

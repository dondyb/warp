# Warp BYO-LLM Fork — Design

**Date:** 2026-04-29
**Status:** Design — pending implementation plan
**Author:** Brainstormed in collaboration; written by Claude

## Context and goals

This is a fork of `warpdotdev/warp` aimed at decoupling Warp's client UI from the warp.dev hosted backend. The fork preserves Warp's UI, terminal, and AI feature surface but replaces the AI network boundary with a user-supplied OpenAI- or Anthropic-compatible endpoint, and removes the warp.dev account login as a precondition for using the app.

End state for a fork user:

1. Configure their own AI provider (endpoint URL + API key + model + protocol) in a Settings panel.
2. Use the full Warp AI feature surface — Agent Mode, inline command suggestions, block summarization, workflow generation, passive suggestions — against their own endpoint.
3. Boot straight into a terminal on first launch. No onboarding, no login wall, no model-picker wizard.
4. No telemetry to warp.dev. No data leaves the user's machine except calls to their configured endpoint.

Cloud features that depend on a warp.dev backend (Drive, Sessions, Workspaces) are hidden in this fork. The architecture leaves a clean seam for a future self-hosted backend project but does not implement one.

## Decisions

| Topic | Choice |
|---|---|
| Path | Warp UI, user's own LLM provider |
| AI scope | All AI features must keep working against the user endpoint |
| Cloud strategy | Strip warp.dev surfaces; architect a clean seam for a future optional self-hosted backend (out of scope to build) |
| Onboarding | Strip everything — no AI wizard, no login slide, no welcome flow |
| Protocol(s) | OpenAI Chat Completions and Anthropic Messages API, both first-class |
| Config shape | Single global `(endpoint, api_key, model_id, protocol, supports_tools)` |
| Config storage | Settings GUI panel; API key in OS secure storage |
| Tool calling fallback | Explicit `supports_tools` flag in config + runtime detection of mis-declarations |
| Sequencing | AI-first: build the adapter and prove end-to-end against a hardcoded config; *then* settings GUI; *then* strip cloud features and onboarding |

## Architecture

Insert a translation layer at the network boundary. Don't touch the rest of Warp.

Today every AI feature constructs an internal `warp_multi_agent_api::Request` protobuf and ships it via `ServerApi::generate_multi_agent_output()` to `https://app.warp.dev/ai/multi-agent`. The fork preserves that internal request shape end-to-end. We replace only the network call.

```
[Warp UI: Agent / suggest / summarize / workflows]
              |
              v
     warp_multi_agent_api::Request  (protobuf, unchanged)
              |
              v
        AiClient trait  (NEW)
              |
   +----------+----------+
   |                     |
   v                     v
OpenAiAdapter      AnthropicAdapter
   |                     |
   v                     v
POST /v1/chat/completions   POST /v1/messages
   (user endpoint)            (user endpoint)
              |
              v
     SSE stream → translated back to ResponseEvent (protobuf)
              |
              v
     [Warp UI consumes events as it does today]
```

Why this shape:

1. **Minimum touch points in the rest of the codebase.** Every AI feature already speaks in `warp_multi_agent_api::Request` / `ResponseEvent`. They keep working unchanged. We translate at one boundary, not in N call sites.
2. **Trait-based provider selection** (`AiClient`) means adding a third protocol later (Ollama-native, vLLM extensions) is one new file, not a rewrite.
3. **Cloud and AI become cleanly separable.** AI no longer goes through `ServerApi`, so removing `ServerApi`'s warp.dev dependencies in a later phase doesn't risk breaking AI.

Where the new code lives:

- `crates/ai_provider/` — new crate; `AiClient` trait, `OpenAiAdapter`, `AnthropicAdapter`, request/response translation, capability detection.
- `app/src/ai/provider_config.rs` — new module; config plumbing (env → settings → defaults), API key access via `keyring`.
- `app/src/settings/views/<ai_provider>` — new "AI Provider" tab in the existing settings UI.
- `app/src/server/server_api.rs` — modified; `generate_multi_agent_output` becomes a thin shim that delegates to `AiClient`.

What stays untouched:

- All Agent Mode UI logic, block execution, tool execution.
- The `warp_multi_agent_api` protobuf types (used as the *internal* IDL).
- All non-AI features.

## Components

### A. New crate: `crates/ai_provider/`

Public surface:

```rust
pub trait AiClient: Send + Sync {
    fn chat_stream(&self, req: Request) -> BoxStream<'_, Result<ResponseEvent, AiError>>;
    fn capabilities(&self) -> ModelCapabilities;
    async fn validate(&self) -> Result<(), AiError>;  // for "Test connection"
}

pub struct ModelCapabilities {
    pub supports_tools: bool,
    pub supports_streaming: bool,
}

pub enum AiError {
    NotConfigured, MissingApiKey, InvalidUrl,
    Network(String), Timeout, Tls,
    Unauthorized, Forbidden, NotFound, RateLimited { retry_after: Option<Duration> },
    ServerError { status: u16 },
    ToolCallingUnsupported, ModelNotFound, ContextTooLong { actual: usize, max: usize },
    InvalidResponse(String), ParseFailure(String), MalformedToolCall { name: String, raw: String },
    Cancelled,
}
```

Two implementations: `OpenAiAdapter` and `AnthropicAdapter`. Each is responsible for:

- **Request shape:** `Request` → JSON body (system prompt, messages, tools array, model, `stream: true`).
- **Streaming response:** SSE chunks → `ResponseEvent` stream (text deltas, tool-call deltas, finish reason, usage).
- **Tool calls:** provider format ↔ Warp's tool-call event.
- **Tool results:** Warp's continuation → OpenAI `role: tool` message / Anthropic `tool_result` content block.
- **Errors:** HTTP failures + provider-specific error envelopes → `AiError`.

The crate has no dependency on the rest of the workspace beyond `warp_multi_agent_api`. Testable in isolation.

### B. `app/src/ai/provider_config.rs`

```rust
pub struct AiProviderConfig {
    pub display_name: String,
    pub endpoint_url: Url,
    pub model_id: String,
    pub protocol: Protocol,        // OpenAi | Anthropic
    pub supports_tools: bool,
}

pub enum Protocol { OpenAi, Anthropic }

pub fn current() -> Option<(AiProviderConfig, SecretString)>;
pub fn save(cfg: AiProviderConfig, key: SecretString) -> Result<()>;
```

Resolution order: **env vars > settings TOML > none.** Env vars: `WARP_AI_ENDPOINT`, `WARP_AI_API_KEY`, `WARP_AI_MODEL`, `WARP_AI_PROTOCOL`, `WARP_AI_SUPPORTS_TOOLS`.

API key in OS secure storage via the `keyring` crate. Settings TOML stores only a reference handle, never the key itself.

On change, swap the `Arc<dyn AiClient>` behind a `RwLock` and notify via `tokio::sync::watch` so existing AI flows pick up the new client without app restart.

### C. Settings GUI panel — "AI Provider" tab

Form fields:

- Display name (free text)
- Endpoint URL (URL-validated)
- API key (masked input; write-only after first save; "Replace API key" button rotates)
- Model ID (free text — `gpt-4o-mini`, `claude-sonnet-4-7`, `llama3.2`, etc.)
- Protocol dropdown: OpenAI / Anthropic
- `supports_tools` checkbox
- "Test connection" button — invokes `AiClient::validate()`, shows ✓/✗ inline with the error message

API key UX: once saved, displayed as `•••`. Stored in OS keyring (macOS Keychain / Linux secret-service / Windows Credential Manager). Settings TOML stores only the keyring handle.

### D. Cloud feature gating

Add `ChannelState::cloud_enabled: bool` (default `false` for the fork). Wherever code today checks `auth_state.is_authenticated()`, also check `cloud_enabled`. UI surfaces hidden when `cloud_enabled == false`:

- Drive sidebar entry
- Sessions menu items
- Workspaces selector
- Sign-in menu item
- Telemetry sender becomes a no-op

The `ForceLogin` feature flag is permanently overridden to `false`.

### E. Onboarding strip

**Delete:**

- `app/src/ai/onboarding.rs` (AI agent onboarding slides)
- `LoginSlideView` and `AuthOnboardingState::LoginSlide` in `app/src/root_view.rs`
- The welcome / theme / keybinding intro slides in `app/src/root_view.rs`

**Modify:** boot path skips `is_onboarded` and goes straight to the terminal view.

### F. Empty-state UX (no AI configured)

`provider_config::current() == None` →

- Agent Mode button **enabled** but clicking opens Settings → AI Provider tab.
- Inline suggestions don't trigger.
- Summarize / workflow buttons disabled with tooltip "Configure AI provider in Settings".
- One-time dismissible banner in the agent input area: *"Configure your AI provider to enable AI features."*

## Data flow

### Flow 1 — Agent Mode chat (single turn, no tools)

1. User types in agent input; existing Warp code constructs `warp_multi_agent_api::Request` with system prompt, conversation history, available tools, model name (whatever's configured locally).
2. `ServerApi::generate_multi_agent_output()` (modified) grabs the current `Arc<dyn AiClient>` and calls `client.chat_stream(req)`.
3. The active adapter (e.g. `OpenAiAdapter`) translates:
   - `req.system_prompt` → `messages[0]` (role: system)
   - `req.history` → `messages[1..]` (role: user/assistant)
   - `req.tools` → `tools[]` (OpenAI function schema)
   - `endpoint_url + "/v1/chat/completions"` with `Authorization: Bearer <api_key>`, `stream: true`.
4. SSE response: each `data: {…}` line is a `chat.completion.chunk`. Adapter inspects `delta`:
   - `delta.content: "..."` → emit `ResponseEvent { kind: TextDelta, text }`
   - `delta.tool_calls[i]` → buffer + emit `ResponseEvent { kind: ToolCallDelta, … }`
   - `finish_reason: "stop"` → emit `ResponseEvent { kind: TurnFinished }`
   - `usage: {…}` (final chunk) → emit `ResponseEvent { kind: Usage, … }`
5. Stream returned to Warp's UI layer — same `Block` rendering, same animations, same persistence.

`AnthropicAdapter` does the same with different translation: `system` (separate field, not a message), `messages` array, `tools` (Anthropic schema with `input_schema`), `/v1/messages`, header `x-api-key` + `anthropic-version`. Stream events differ (`content_block_start`, `content_block_delta`, `message_delta`, `message_stop`) but emit the same `ResponseEvent` variants.

### Flow 2 — Tool call round trip

1. As Flow 1 through step 4, but the model emits a tool call. Adapter buffers `tool_calls[*]` deltas across chunks (OpenAI streams them piecewise), assembles into a complete tool_call, emits `ResponseEvent { kind: ToolCall, name, args, id }`.
2. Warp's existing tool executor runs the tool (file_read, run_command, etc.) and gets a result.
3. Existing code constructs the next `Request` with the tool result added to history. The adapter translates Warp's tool-result representation to:
   - **OpenAI:** push `{ role: "tool", tool_call_id, content }` message in `messages[]`.
   - **Anthropic:** push `{ role: "user", content: [{ type: "tool_result", tool_use_id, content }] }`.
4. Adapter calls the endpoint again with augmented messages. New SSE stream. Repeat.
5. Eventually model emits `finish_reason: stop` (OpenAI) or `stop_reason: end_turn` (Anthropic). Round-trip complete.

**Cancellation:** UI cancel signal → adapter aborts the in-flight `reqwest` request → SSE stream drops → `ResponseEvent { kind: Cancelled }` emitted. No leakage of in-progress tool calls.

### Flow 3 — Configuration change at runtime

1. User opens Settings → AI Provider, edits endpoint or API key, saves.
2. Settings code persists TOML, writes to keyring.
3. `provider_config::reload()` constructs a fresh `AiProviderConfig + SecretString`, picks the adapter based on `protocol`, builds a new `Arc<dyn AiClient>`.
4. Swaps the `Arc` behind the `RwLock`. Sends a tick on the `tokio::sync::watch` channel.
5. Any new `chat_stream` call uses the new client. In-flight calls finish on the old client.
6. Empty-state UI updates: agent button re-enables if previously disabled.

## Error handling

### Error taxonomy → `AiError` variants

| Category | Variants | Source |
|---|---|---|
| Config | `NotConfigured`, `MissingApiKey`, `InvalidUrl` | Caller (no client constructible) |
| Transport | `Network(io_err)`, `Timeout`, `Tls` | reqwest |
| Auth/HTTP | `Unauthorized`, `Forbidden`, `NotFound`, `RateLimited { retry_after }`, `ServerError { status }` | HTTP status mapping |
| Capability | `ToolCallingUnsupported`, `ModelNotFound`, `ContextTooLong { actual, max }` | Provider error envelope or runtime detection |
| Protocol | `InvalidResponse`, `ParseFailure`, `MalformedToolCall { name, raw }` | Adapter cannot decode |
| User | `Cancelled` | UI abort |

### Surfacing rules

- **Inside a conversation:** every error becomes a `ResponseEvent::Error { kind, message, retryable }` that the existing Block UI renders as a red inline bubble with a "Retry" button when `retryable == true`.
- **Inside the Settings panel "Test connection":** inline result, plain language. `Unauthorized` → "API key rejected"; `NotFound` → "Endpoint URL not reachable (404)"; etc.
- **Outside any conversation:** `NotConfigured` is *not* an error — handled by the empty-state UX. It never reaches the conversation layer.

### Retry policy

- **Retryable:** `Network`, `Timeout`, `RateLimited` (after `retry_after`), `ServerError` 5xx (max 2 retries with exponential backoff).
- **Not retryable:** `Unauthorized`, `Forbidden`, `ModelNotFound`, `ToolCallingUnsupported`, `MalformedToolCall`, `InvalidResponse`, `ParseFailure`, `Cancelled`. User must reconfigure or restart.
- Retries happen **inside the adapter**, not the UI. UI sees one stream that either succeeds or emits a single `ResponseEvent::Error`.

### Capability detection (runtime)

If the user has `supports_tools: true` but the endpoint:

- Returns HTTP 400 with body matching `/tools|function.*not.support/i`, **or**
- Accepts the request but returns plain text where a tool call should be (heuristic: model emits `<tool_call>…` style markup but the structured field is missing)

then the adapter emits `ResponseEvent::Error { ToolCallingUnsupported }`, sets a session-scoped `effective_supports_tools = false`, and disables Agent Mode for the rest of this session (button greys with "endpoint reported tool calling not supported — verify your model in Settings"). This override is *session-scoped*, never written to settings without explicit user action.

If the user has `supports_tools: false` from the start, Agent Mode is hidden at the UI layer; suggestions/summary still work.

### Cancellation

- Every `chat_stream` call holds an `AbortHandle`.
- UI cancel button → `abort()` fires → reqwest request drops → SSE stream closes → adapter emits `ResponseEvent::Cancelled`.
- In-flight tool-call buffers are discarded. No partial tool execution.

### Non-goals

- No background telemetry of errors. Errors are user-visible only.
- No automatic API key rotation or refresh. OAuth-style providers are not in scope.
- No "fall back to a different model on error" — single configured model means no fallback.

## Testing

### Layer 1 — Adapter unit tests (new)

In `crates/ai_provider/`, table-driven unit tests for the translation logic. Pure functions, no network.

- *Request → JSON body:* given a `warp_multi_agent_api::Request` fixture, assert the JSON body is byte-equal to a checked-in golden file. One golden file per (protocol × scenario): text-only, with-tools, multi-turn, with-tool-result, system-prompt, empty-history.
- *SSE chunk → `ResponseEvent`:* given a recorded SSE chunk (literal bytes), assert the emitted event sequence matches expected.

Run via `cargo nextest run -p ai_provider`.

### Layer 2 — Adapter integration tests with `wiremock` (new)

A `wiremock` server impersonates OpenAI / Anthropic. Tests:

- Happy path (post `Request`, assert outbound HTTP body, assert returned events).
- Tool call round-trip (mock returns `tool_calls`, test feeds tool result, assert second request includes tool result).
- Error mapping (mock returns 401/429/500/malformed JSON; assert correct `AiError` variant).
- Capability detection (mock returns 400 with "tools not supported"; assert `ToolCallingUnsupported` and session flag flip).
- Cancellation (call `chat_stream`, abort mid-stream, assert `Cancelled` and connection cleanup).

### Layer 3 — Replay fixtures from real providers (new, lightweight)

A small CLI tool (`crates/ai_provider/examples/record_sse.rs`) hits OpenAI / Anthropic / Ollama with a canonical request and writes the raw SSE stream + final HTTP body to `tests/fixtures/`. Layer 1 tests replay these. Re-recorded only when a provider changes their format. Goldens are version-controlled.

### Layer 4 — Settings GUI tests (existing infra)

Use Warp's existing settings test harness:

- Form validation (invalid URL, empty endpoint, missing API key).
- Save round-trip (save → reload → verify TOML on disk + keyring entry exists; use a test keyring backend).
- "Test connection" path (hits Layer 2's wiremock, asserts UI shows ✓/✗).
- Replace API key flow.

### Layer 5 — End-to-end integration test (existing `crates/integration` framework)

One smoke test using Warp's Builder/TestStep framework:

- Start app with env vars pointing at a wiremock endpoint.
- Send an Agent Mode prompt.
- Assert Block contains expected text from the canned response.
- Assert no warp.dev URL is contacted (intercept reqwest, fail on outbound to `*.warp.dev`).

This catches "we accidentally regressed cloud strip" or "AI request went to the wrong host."

### Manual verification checklist (per release)

1. Configure against OpenAI `gpt-4o-mini` → Agent Mode works, suggestions work, summarize works.
2. Configure against Anthropic `claude-sonnet-4-7` → same.
3. Configure against local Ollama `llama3.2` with `supports_tools: false` → Agent button greyed; suggestions and summarize still work.
4. Misconfigure (bad URL, bad key) → error states render correctly, "Test connection" reports failure.
5. Cancel mid-generation → no zombie HTTP request (verify with `lsof` or Activity Monitor).

### What we deliberately don't test

- LLM output quality.
- Specific provider performance / latency.
- Settings GUI rendering pixel-by-pixel.
- Anything that requires real OpenAI/Anthropic API keys in CI. Layer 3 fixtures cover format compliance offline.

## Phasing

Sequencing per Approach 2 (AI-first), broken into shippable milestones:

### M1 — AI client trait + OpenAI adapter (env-var config)

- New `crates/ai_provider/` crate with `AiClient` trait and `OpenAiAdapter`.
- `provider_config.rs` reading from env vars only (no settings UI yet).
- `ServerApi::generate_multi_agent_output()` rewired to use `AiClient`.
- Unit + wiremock integration tests for the OpenAI adapter (Layers 1, 2).
- **Gate:** Agent Mode talks end-to-end to OpenAI / OpenRouter / Ollama via env-var config. Tool calling round-trip works.

### M2 — Anthropic adapter

- `AnthropicAdapter` impl with translation tests (Layers 1, 2).
- Protocol selection wired to env var.
- **Gate:** Agent Mode talks end-to-end to Anthropic via env-var config. Tool calling works.

### M3 — Settings GUI panel

- New "AI Provider" tab in settings.
- Keyring integration.
- "Test connection" button.
- Settings GUI tests (Layer 4).
- Empty-state UX wired up.
- **Gate:** A user with no env vars can configure via the GUI and use AI features.

### M4 — Cloud strip + onboarding strip

- `ChannelState::cloud_enabled = false` flag.
- Hide Drive / Sessions / Workspaces / sign-in surfaces.
- Telemetry no-op.
- Delete `LoginSlideView`, AI onboarding, welcome/theme/keybinding slides.
- Force-disable `ForceLogin` feature flag.
- E2E integration test (Layer 5) verifying no `*.warp.dev` egress.
- **Gate:** Fresh first launch goes straight to terminal; no warp.dev outbound traffic.

### M5 — Capability detection + polish

- Runtime `ToolCallingUnsupported` detection and session-scoped override.
- Retry policy, error rendering, "Replace API key" UX, manual verification pass.
- **Gate:** All five items in the manual verification checklist pass.

## Out of scope (explicit non-goals)

- **Self-hosted backend implementation.** The architecture leaves a clean seam (`override_server_root_url` already exists) for a future project, but writing Drive / Sessions / Workspaces servers is not part of this fork.
- **Multi-model / per-feature model routing.** Single global model is the v1 contract.
- **OAuth-style auth flows for the AI provider.** Static API keys only.
- **ReAct-style fallback for non-tool-capable models.** Capability flag + runtime detection only; `Agent Mode` is unavailable on tool-incapable endpoints.
- **Vision / audio / multimodal capabilities.** Future work; not in v1 capability set.
- **Migrating users from the upstream Warp client.** New install only.
- **A new GUI for the existing Warp model picker.** The existing model picker in Warp's UI is removed or hidden; the only model selection happens in the new Settings panel.

## Open questions for implementation

These do not block the design but should be resolved during the writing-plans phase:

1. Exact behavior for the existing Warp model-picker UI (likely hidden entirely; needs to confirm there's no other code that depends on its presence).
2. What to do with `cli_agent_model`, `computer_use_model`, and other internal feature-specific model overrides — almost certainly reduce them all to point at the single configured model, but verify no behavior depends on differentiation.
3. Whether to rename the binary / app bundle from `warp-oss` to a fork-specific name. Out of scope for this design but worth tracking.
4. Whether to ship a default config that warns "no AI configured" vs. an empty config that hides AI features entirely on first launch. Defaulting to "empty + visible button that opens settings" matches Section 2/F.

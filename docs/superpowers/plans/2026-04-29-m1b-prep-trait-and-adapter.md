# M1b-prep — `AiProvider` Trait, `WarpServerAdapter`, `AIApiError` Move

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Refactor the M1a dispatch seam into a proper trait-based abstraction. Define the `AiProvider` trait in `ai_provider`, move `AIApiError` (plus the colocated `DeserializationError` and two constants) out of `server_api.rs` into `ai_provider`, and add a `WarpServerAdapter` that implements the trait by delegating to a renamed-private version of the existing function. **No app behavior change.** This unblocks M1b-chat (where `OpenAiAdapter` becomes the second impl) and M2 (Anthropic).

**Architecture:** `crates/ai_provider/` becomes a real crate with public surface: `AiProvider` trait + `AIApiError` + `Protocol` enum + env-var resolver. `WarpServerAdapter` lives in the app crate (next to `ServerApi`) because it depends on `Arc<ServerApi>` for HTTP client / auth / channel state. The existing `generate_multi_agent_output` function body is moved verbatim into a private `pub(crate) fn generate_multi_agent_output_via_warp`, and the public function becomes a thin dispatcher that constructs the right adapter for the configured `Protocol` and calls `chat_stream` through the trait.

**Tech Stack:** Rust 2021. `async-trait` (workspace dep), `futures::stream::BoxStream`, plus the existing `reqwest`, `reqwest_eventsource`, `prost`, `thiserror`, `anyhow`, `http_client`, `http` deps that `AIApiError` already pulls in.

---

## Context

After M1a:

- `crates/ai_provider/` exists with just `Protocol` enum + `resolve_protocol_from_env()` and a single unit test. **No deps in Cargo.toml**.
- `app/src/server/server_api.rs:1077–1097` has a 17-line `match` block at the top of `generate_multi_agent_output` that early-returns errors for `Protocol::OpenAi` and `Protocol::Anthropic`, falling through for `Protocol::Warp`.
- `AIApiError` is defined inline in `server_api.rs:156–284` along with helper methods, three `From` impls, and the `DeserializationError` enum at line 149.
- The constants `WARP_ERROR_CODE_HEADER` (line 77) and `WARP_ERROR_CODE_OUT_OF_CREDITS` (line 82) are used by `AIApiError::error_for_429` *and* by 4 other call sites in `server_api.rs` (lines 250, 737, 742, 1052). They must remain accessible to those call sites.
- `AIApiError` is imported externally from at least 6 files in the workspace (verified): `app/src/terminal/view/ambient_agent/model.rs`, `app/src/ai/predict/next_command_model.rs`, `app/src/ai/agent/api.rs`, `app/src/ai/agent/mod.rs`, `app/src/ai/blocklist/controller.rs`, `app/src/ai/get_relevant_files/controller.rs`. Their imports go through `crate::server::server_api::AIApiError` today.

After M1b-prep:

- `AIApiError`, `DeserializationError`, and the two constants live in `crates/ai_provider/src/error.rs`.
- `server_api.rs` has `pub use ai_provider::{AIApiError, DeserializationError, WARP_ERROR_CODE_HEADER, WARP_ERROR_CODE_OUT_OF_CREDITS};` so external callers and same-file uses keep working without a single import edit.
- `AiProvider` trait lives in `crates/ai_provider/src/client.rs`. One method: `chat_stream`.
- `WarpServerAdapter` lives in `app/src/server/warp_adapter.rs` and impls the trait. Holds `Arc<ServerApi>`; delegates to `ServerApi::generate_multi_agent_output_via_warp` (the renamed private fn).
- The dispatcher in `generate_multi_agent_output` constructs the adapter for `Protocol::Warp` and calls it via the trait. For `OpenAi`/`Anthropic`, still inline error (M1b-chat replaces).

## File Structure

**Create:**

| Path | Responsibility |
|---|---|
| `crates/ai_provider/src/error.rs` | `AIApiError`, `DeserializationError`, the two `WARP_ERROR_CODE_*` constants. All `From` impls + helper methods. |
| `crates/ai_provider/src/client.rs` | `AiProvider` trait. |
| `app/src/server/warp_adapter.rs` | `WarpServerAdapter` struct + `impl AiProvider`. |

**Modify:**

| Path | Change |
|---|---|
| `crates/ai_provider/Cargo.toml` | Add deps: `anyhow`, `async-trait`, `futures`, `http`, `http_client`, `reqwest`, `reqwest_eventsource`, `serde_json`, `thiserror`, `warp_multi_agent_api`. |
| `crates/ai_provider/src/lib.rs` | Add `pub mod {client, error};` and `pub use {client::AiProvider, error::*};`. |
| `app/src/server/server_api.rs` | Delete inline `AIApiError`, `DeserializationError`, the two constants, and the `From` impls (lines 77, 82, 145–284). Add `pub use ai_provider::...` re-exports. Rename existing fn body to `generate_multi_agent_output_via_warp` (private, `pub(crate)`). Replace dispatcher to use `WarpServerAdapter::new(self.clone()).chat_stream(req)` for `Protocol::Warp`; keep inline errors for `OpenAi`/`Anthropic`. Declare `mod warp_adapter;`. |

---

## Tasks

### Task 1: Add workspace deps + module skeleton to `ai_provider`

**Files:**
- Modify: `crates/ai_provider/Cargo.toml`
- Modify: `crates/ai_provider/src/lib.rs`

- [ ] **Step 1: Update `crates/ai_provider/Cargo.toml`**

Replace the contents of `crates/ai_provider/Cargo.toml` with:

```toml
[package]
name = "ai_provider"
version = "0.1.0"
edition = "2021"
publish.workspace = true
license.workspace = true

[dependencies]
anyhow.workspace = true
async-trait.workspace = true
futures.workspace = true
http.workspace = true
http_client.workspace = true
prost.workspace = true
reqwest.workspace = true
reqwest_eventsource.workspace = true
serde_json.workspace = true
thiserror.workspace = true
warp_multi_agent_api.workspace = true
```

- [ ] **Step 2: Verify all those deps exist in workspace `Cargo.toml`**

Run `rg -nE "^(anyhow|async-trait|futures|http|http_client|prost|reqwest|reqwest_eventsource|serde_json|thiserror|warp_multi_agent_api) ?=" /Users/dondy/Codes/warp/Cargo.toml`. All 11 should appear. If any are missing (unlikely given they're widely used), STOP and report `BLOCKED` — adding workspace deps is out of scope for this plan.

- [ ] **Step 3: Stub the new module files**

Create `crates/ai_provider/src/error.rs` with just:

```rust
//! Canonical AI API error type, moved from `app/src/server/server_api.rs` so
//! the `AiProvider` trait can reference it without a circular dependency.
//! Populated in Task 2.
```

Create `crates/ai_provider/src/client.rs` with just:

```rust
//! `AiProvider` trait — the network-boundary abstraction for Warp's AI calls.
//! Populated in Task 3.
```

- [ ] **Step 4: Update `crates/ai_provider/src/lib.rs`**

The current `lib.rs` (from M1a) defines `Protocol`, `resolve_protocol_from_env`, and a `#[cfg(test)] mod tests` directly at the file's top level — there's no submodule structure yet. Add `pub mod client;` and `pub mod error;` declarations near the top of `lib.rs` *and* a re-export block, while leaving everything else (the existing `Protocol` enum, the resolver function, and the test module) **in place and unchanged**.

The simplest concrete edit is at the top of the file. Insert after the existing top-level module doc comment (which ends around line 12):

```rust
pub mod client;
pub mod error;

pub use client::AiProvider;
pub use error::{
    AIApiError, DeserializationError, WARP_ERROR_CODE_HEADER, WARP_ERROR_CODE_OUT_OF_CREDITS,
};
```

Do NOT touch the existing `pub enum Protocol`, `pub fn resolve_protocol_from_env`, or the `mod tests` block below it — they stay where they are. `Protocol` and `resolve_protocol_from_env` are already accessible as `ai_provider::Protocol` and `ai_provider::resolve_protocol_from_env` from outside.

- [ ] **Step 5: Verify the crate compiles standalone**

Run: `cargo check -p ai_provider`
Expected: ERROR — `AIApiError`/`DeserializationError`/etc. don't exist yet (we only stubbed `error.rs`). **This is correct for Task 1.** Comment out the `pub use error::{ ... };` line temporarily so we can verify the OTHER changes are good. Run `cargo check -p ai_provider` again — should PASS now.

> Don't forget to UNcomment the line in Task 2 once `error.rs` is populated.

- [ ] **Step 6: Commit**

```bash
cd /Users/dondy/Codes/warp
git add crates/ai_provider
git commit -m "build(ai_provider): add deps and stub modules for trait+error"
```

After committing, verify: `git log --oneline -1` should show this commit on top.

---

### Task 2: Move `AIApiError`, `DeserializationError`, and the two constants to `ai_provider/src/error.rs`

**Files:**
- Modify: `crates/ai_provider/src/error.rs` (paste moved code)
- Modify: `app/src/server/server_api.rs` (delete moved code, add re-exports)
- Modify: `crates/ai_provider/src/lib.rs` (re-enable the `pub use error::{...};` line if commented out in Task 1)

- [ ] **Step 1: Read the current code one more time to ground the move**

Run:
```bash
sed -n '77,82p' /Users/dondy/Codes/warp/app/src/server/server_api.rs   # constants
sed -n '145,285p' /Users/dondy/Codes/warp/app/src/server/server_api.rs # error types + impls
```

Confirm:
- Line 77: `const WARP_ERROR_CODE_HEADER: &str = "X-Warp-Error-Code";`
- Line 82: `const WARP_ERROR_CODE_OUT_OF_CREDITS: &str = "OUT_OF_CREDITS";`
- Lines 145–155: `pub enum DeserializationError`
- Lines 156–185: `pub enum AIApiError`
- Lines 186–202: `From<http_client::ResponseError>`, `From<reqwest::Error>`, `From<serde_json::Error>` impls
- Lines 204–284: `impl AIApiError { fn from_response_error, fn from_transport_error, fn error_for_429, async fn from_stream_error }`

If line numbers have drifted, use `rg -n "pub enum AIApiError|pub enum DeserializationError|impl AIApiError|impl From<.* for AIApiError|const WARP_ERROR_CODE" /Users/dondy/Codes/warp/app/src/server/server_api.rs` to anchor.

- [ ] **Step 2: Replace `crates/ai_provider/src/error.rs` with the full block**

Replace its placeholder contents with this (exact text):

```rust
//! Canonical AI API error type, moved from `app/src/server/server_api.rs` so
//! the `AiProvider` trait can reference it without a circular dependency.

use anyhow::anyhow;

/// Header name used by Warp's hosted backend to communicate fine-grained
/// reasons for HTTP 429 responses.
pub const WARP_ERROR_CODE_HEADER: &str = "X-Warp-Error-Code";

/// Value of `WARP_ERROR_CODE_HEADER` indicating the user has exhausted
/// their AI credits (vs. the server being overloaded).
pub const WARP_ERROR_CODE_OUT_OF_CREDITS: &str = "OUT_OF_CREDITS";

/// Wrapper for deserialization errors. This covers both:
/// * Using `serde` directly
/// * Using `reqwest` decoding utilities
#[derive(thiserror::Error, Debug)]
pub enum DeserializationError {
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    #[error(transparent)]
    Transport(reqwest::Error),
}

#[derive(thiserror::Error, Debug)]
pub enum AIApiError {
    #[error("Request failed due to lack of AI quota.")]
    QuotaLimit,

    #[error("Warp is currently overloaded. Please try again later.")]
    ServerOverloaded,

    #[error("Internal error occurred at transport layer.")]
    Transport(#[source] reqwest::Error),

    #[error("Failed to deserialize API response.")]
    Deserialization(#[source] DeserializationError),

    #[error("No context found on context search.")]
    NoContextFound,

    #[error("Failed with status code {0}: {1}")]
    ErrorStatus(http::StatusCode, String),

    #[error(transparent)]
    Other(#[from] anyhow::Error),

    #[error("Got error when streaming {stream_type}: {source:#}")]
    Stream {
        stream_type: &'static str,
        #[source]
        source: anyhow::Error,
    },
}

impl From<http_client::ResponseError> for AIApiError {
    fn from(err: http_client::ResponseError) -> Self {
        Self::from_response_error(err.source, &err.headers)
    }
}

impl From<reqwest::Error> for AIApiError {
    fn from(err: reqwest::Error) -> Self {
        Self::from_transport_error(err)
    }
}

impl From<serde_json::Error> for AIApiError {
    fn from(err: serde_json::Error) -> Self {
        AIApiError::Deserialization(err.into())
    }
}

impl AIApiError {
    /// Converts a reqwest error to an AIApiError, using response headers to distinguish
    /// between different types of 429 errors.
    fn from_response_error(err: reqwest::Error, headers: &::http::HeaderMap) -> Self {
        // For HTTP 429 errors, check the X-Warp-Error-Code header to distinguish
        // between out-of-credits and server-overload.
        if err.status() == Some(http::StatusCode::TOO_MANY_REQUESTS) {
            return Self::error_for_429(headers);
        }

        Self::from_transport_error(err)
    }

    /// Converts a transport-level reqwest error (no HTTP response) to an AIApiError.
    fn from_transport_error(err: reqwest::Error) -> Self {
        // Unfortunately, `reqwest` reports some non-decoding errors as decoding errors (e.g.
        // unexpected disconnects or timeouts while deserializing a response body). Since we
        // render deserialization and transport errors differently, we try to detect those cases
        // here.
        if err.is_timeout() {
            return AIApiError::Transport(err);
        }
        if err.is_decode() {
            #[cfg(not(target_family = "wasm"))]
            {
                use std::error::Error as _;
                let mut source = err.source();
                while let Some(underlying) = source {
                    if underlying.is::<hyper::Error>() {
                        return AIApiError::Transport(err);
                    }

                    source = underlying.source();
                }
            }

            return AIApiError::Deserialization(DeserializationError::Transport(err));
        }

        AIApiError::Transport(err)
    }

    /// Returns the appropriate error for a 429 response by checking the X-Warp-Error-Code header.
    fn error_for_429(headers: &::http::HeaderMap) -> Self {
        if headers
            .get(WARP_ERROR_CODE_HEADER)
            .and_then(|v| v.to_str().ok())
            == Some(WARP_ERROR_CODE_OUT_OF_CREDITS)
        {
            AIApiError::QuotaLimit
        } else {
            AIApiError::ServerOverloaded
        }
    }

    /// Format a stream error into a human-readable error message. This will read the response
    /// body if there is one.
    pub async fn from_stream_error(
        stream_type: &'static str,
        err: reqwest_eventsource::Error,
    ) -> Self {
        match err {
            reqwest_eventsource::Error::InvalidStatusCode(
                http::StatusCode::TOO_MANY_REQUESTS,
                ref res,
            ) => Self::error_for_429(res.headers()),
            reqwest_eventsource::Error::InvalidStatusCode(status, res) => Self::ErrorStatus(
                status,
                res.text()
                    .await
                    .unwrap_or_else(|e| format!("(no response body: {e:#})")),
            ),
            reqwest_eventsource::Error::Transport(err) => Self::from_transport_error(err),
            err => AIApiError::Stream {
                stream_type,
                // On WASM, `reqwest_eventsource::Error` doesn't implement `Into<anyhow::Error>` or
                // `Send` because it may contain a `wasm_bindgen` JS value.
                #[cfg(target_family = "wasm")]
                source: anyhow!("{err:#?}"),
                #[cfg(not(target_family = "wasm"))]
                source: anyhow!(err),
            },
        }
    }
}
```

> **Visibility note:** `from_response_error`, `from_transport_error`, and `error_for_429` are private helpers — keep them `fn` (no visibility). `from_stream_error` is bumped from `async fn` (private) to `pub async fn` because it's called from outside the impl in `server_api.rs:1144`.

- [ ] **Step 3: Verify `ai_provider` compiles standalone**

If you commented out the `pub use error::{ ... };` line in `lib.rs` during Task 1, re-enable it now.

Run: `cargo check -p ai_provider 2>&1 | tail -10`
Expected: PASS, no errors.

If a compile error fires complaining about `hyper::Error` not being a workspace dep — note that the `is::<hyper::Error>()` check is inside `#[cfg(not(target_family = "wasm"))]`. On macOS/Linux/Windows we DO need `hyper` available transitively through reqwest (which uses hyper). If the compiler can't find `hyper` directly, add `hyper.workspace = true` to `crates/ai_provider/Cargo.toml` (verify it exists in workspace `Cargo.toml` first via `rg "^hyper ?=" /Users/dondy/Codes/warp/Cargo.toml`). If `hyper` isn't a direct workspace dep, the existing `is::<hyper::Error>()` works through type inference — try without the explicit dep first.

- [ ] **Step 4: Delete the moved code from `server_api.rs` and add the re-export**

In `app/src/server/server_api.rs`:

1. Delete lines 77 and 82 (the two constants).
2. Delete lines 145–284 (DeserializationError + AIApiError + impls). The exact range to delete is everything from the doc comment `/// Wrapper for deserialization errors...` through the closing `}` of `impl AIApiError`. Use `rg -n "pub enum DeserializationError|^}$" app/src/server/server_api.rs | head -8` to find the matching close brace if the line numbers are uncertain.
3. Add this re-export near the top of the file, in the imports section (after the existing `use ai_provider::{...}` line if any, otherwise add it):

```rust
pub use ai_provider::{AIApiError, DeserializationError, WARP_ERROR_CODE_HEADER, WARP_ERROR_CODE_OUT_OF_CREDITS};
```

The other 4 in-file references to `WARP_ERROR_CODE_HEADER` (lines 250, 737, 742, 1052) and 3 references to `WARP_ERROR_CODE_OUT_OF_CREDITS` (lines 252, 744, 1054) keep working unchanged via the re-export — Rust resolves them through the local module.

- [ ] **Step 5: Verify the workspace builds**

Run: `cargo check --workspace 2>&1 | tail -15`
Expected: PASS.

Common failure modes:
- *"cannot find type `AIApiError`"* in some external file — that file may have been doing `use crate::server::server_api::AIApiError;`. The re-export keeps that path valid, so this shouldn't happen. If it does, the re-export is misplaced — make sure it's at the top level of `server_api.rs`, not inside an `impl` block or `mod`.
- *"private function `from_stream_error`"* — confirm Step 2 made it `pub async fn`.
- *"unresolved import `ai_provider::AIApiError`"* — confirm `lib.rs` re-exports `error::AIApiError`.

- [ ] **Step 6: Run tests for both crates**

Run: `cargo nextest run -p ai_provider -p warp 2>&1 | tail -15`
Expected: no NEW failures vs. M1a's known set (`test_migration_does_not_rerun_when_marker_present` etc.). The `ai_provider` crate has 4 unit tests + 2 integration tests from M1a — they should all still pass.

- [ ] **Step 7: Commit**

```bash
cd /Users/dondy/Codes/warp
git add crates/ai_provider app/src/server/server_api.rs
git commit -m "refactor(ai_provider): move AIApiError out of server_api"
```

After committing: `git log --oneline -2` — your new commit should be on top.

---

### Task 3: Define the `AiProvider` trait

**Files:**
- Modify: `crates/ai_provider/src/client.rs`

- [ ] **Step 1: Replace the placeholder with the trait definition**

Replace `crates/ai_provider/src/client.rs` contents with:

```rust
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
```

- [ ] **Step 2: Verify it compiles**

Run: `cargo check -p ai_provider`
Expected: PASS.

- [ ] **Step 3: Add a unit test that a stub impl satisfies the trait**

Append to `crates/ai_provider/tests/dispatch.rs` (which already exists from M1a):

```rust
use ai_provider::{AiProvider, ResponseEventStream};

struct StubProvider;

#[async_trait::async_trait]
impl AiProvider for StubProvider {
    async fn chat_stream(
        &self,
        _request: &warp_multi_agent_api::Request,
    ) -> std::result::Result<ResponseEventStream, std::sync::Arc<ai_provider::AIApiError>> {
        Ok(Box::pin(futures::stream::iter(std::iter::empty())))
    }
}

#[tokio::test]
async fn stub_provider_returns_empty_stream() {
    let provider: Box<dyn AiProvider> = Box::new(StubProvider);
    let req = warp_multi_agent_api::Request::default();
    let mut stream = provider.chat_stream(&req).await.expect("stream");
    use futures::StreamExt;
    assert!(stream.next().await.is_none());
}
```

> Don't add `[dev-dependencies]` for `tokio`, `async-trait`, `futures` — they're already accessible through the existing test infrastructure if `dispatch.rs` already imports them. If a missing-dev-dep error fires, add `tokio = { workspace = true, features = ["macros", "rt-multi-thread"] }` and `async-trait.workspace = true` and `futures.workspace = true` to `crates/ai_provider/Cargo.toml`'s `[dev-dependencies]`.

- [ ] **Step 4: Run the new test**

Run: `cargo nextest run -p ai_provider --test dispatch -E 'test(stub_provider_returns_empty_stream)'`
Expected: 1 test passes.

- [ ] **Step 5: Commit**

```bash
cd /Users/dondy/Codes/warp
git add crates/ai_provider
git commit -m "feat(ai_provider): define AiProvider trait"
```

---

### Task 4: Rename existing function body to `_via_warp` and add the dispatcher's trait branch

**Files:**
- Modify: `app/src/server/server_api.rs`
- Create: `app/src/server/warp_adapter.rs`

- [ ] **Step 1: Find the existing function shape**

Run: `sed -n '1071,1100p' /Users/dondy/Codes/warp/app/src/server/server_api.rs`

The function `pub async fn generate_multi_agent_output(...)` starts there. Its body has the M1a dispatch prologue (the `match ai_provider::resolve_protocol_from_env() { ... }` block) followed by the original implementation that POSTs to `/ai/multi-agent`.

- [ ] **Step 2: Split the function**

Edit `app/src/server/server_api.rs`. Replace the entire `generate_multi_agent_output` function (from `pub async fn generate_multi_agent_output(` through its closing `}`) with TWO functions:

```rust
    pub async fn generate_multi_agent_output(
        self: &Arc<Self>,
        request: &warp_multi_agent_api::Request,
    ) -> std::result::Result<AIOutputStream<warp_multi_agent_api::ResponseEvent>, Arc<AIApiError>>
    {
        // Protocol dispatch. For Warp's hosted backend (the default), construct
        // a WarpServerAdapter and route through the AiProvider trait. For other
        // protocols, M1b-chat/M2 will plug in their own adapters; until then we
        // return a clear error rather than silently calling Warp.
        match ai_provider::resolve_protocol_from_env() {
            Protocol::Warp => {
                let adapter = warp_adapter::WarpServerAdapter::new(self.clone());
                ai_provider::AiProvider::chat_stream(&adapter, request).await
            }
            Protocol::OpenAi => Err(Arc::new(AIApiError::Other(anyhow!(
                "WARP_AI_PROTOCOL=openai requested, but the OpenAI adapter \
                 is not yet implemented (planned for M1b-chat)"
            )))),
            Protocol::Anthropic => Err(Arc::new(AIApiError::Other(anyhow!(
                "WARP_AI_PROTOCOL=anthropic requested, but the Anthropic \
                 adapter is not yet implemented (planned for M2)"
            )))),
        }
    }

    /// The original Warp-hosted implementation of `generate_multi_agent_output`,
    /// renamed and made `pub(crate)` so `WarpServerAdapter` can call it.
    /// Behavior is byte-identical to the M1a state.
    pub(crate) async fn generate_multi_agent_output_via_warp(
        &self,
        request: &warp_multi_agent_api::Request,
    ) -> std::result::Result<AIOutputStream<warp_multi_agent_api::ResponseEvent>, Arc<AIApiError>>
    {
        // (paste the original body here — everything from `let auth_token = ...`
        //  through the final `Ok(output_stream.boxed())` block)
    }
```

> Replace the `// (paste...)` comment with the **exact original body** that was inside `generate_multi_agent_output` AFTER the M1a dispatch prologue. That body starts with `let auth_token = self.get_or_refresh_access_token()...` and ends with the `cfg_if!` block that returns `Ok(output_stream.boxed())` or `boxed_local()`.

> The receiver of the public function changes from `&self` to `self: &Arc<Self>` because `WarpServerAdapter::new` takes `Arc<ServerApi>`. Audit call sites — `app/src/ai/agent/api/impl.rs:132`, `app/src/ai/blocklist/passive_suggestions/maa.rs:177`, `app/src/ai/blocklist/controller/response_stream.rs:97,160` already use `Arc<ServerApi>` per the survey, so this should compile. If any callsite errors with "expected `&Arc<Self>`", the fix is one of:
>
> 1. Caller has `&server_api` where `server_api: Arc<ServerApi>` — change call to `server_api.clone()` or just pass `&server_api` directly (auto-deref handles `&Arc<Self>` from `&server_api`). Most cases work without changes.
> 2. Caller has `&ServerApi` (raw reference) — escalate as `BLOCKED` with the call site listed; receiver type change is more invasive than this task allows. **Stay on this task scope** — don't refactor caller chains.

- [ ] **Step 3: Verify it compiles**

Run: `cargo check -p warp 2>&1 | tail -10`
Expected: PASS.

If it fails because `warp_adapter::WarpServerAdapter` doesn't exist yet, that's correct — Step 4 creates it.

- [ ] **Step 4: Create `app/src/server/warp_adapter.rs`**

Create the file with:

```rust
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
```

> Note: the return type `ResponseEventStream` (the trait's typedef) and the existing `AIOutputStream<ResponseEvent>` should be the same concrete type. If they're not — i.e., if the compiler complains about a type mismatch — adjust `ResponseEventStream` in `crates/ai_provider/src/client.rs` to match the existing `AIOutputStream` definition exactly.

- [ ] **Step 5: Wire the module declaration**

In `app/src/server/server_api.rs` (or wherever submodules of `server` are declared — search with `rg -n "pub mod " /Users/dondy/Codes/warp/app/src/server/mod.rs /Users/dondy/Codes/warp/app/src/server.rs 2>/dev/null`), add:

```rust
pub mod warp_adapter;
```

If `app/src/server/mod.rs` doesn't exist (server might be `app/src/server.rs` with inline submodule declarations), add the `pub mod warp_adapter;` near the top of `server_api.rs` itself. If unsure, run `find /Users/dondy/Codes/warp/app/src/server -maxdepth 2 -type f` to see the layout.

- [ ] **Step 6: Verify the workspace builds**

Run: `cargo check --workspace 2>&1 | tail -10`
Expected: PASS.

- [ ] **Step 7: Run the M1a integration tests to confirm no regression**

Run: `cargo nextest run -p warp -E 'test(openai_protocol_returns_not_implemented_error) | test(anthropic_protocol_returns_not_implemented_error)'`
Expected: 2 tests pass. (M1a's tests: setting `WARP_AI_PROTOCOL=openai/anthropic` produces the documented error.) The error messages may have changed slightly — they now reference "M1b-chat" and "M2" instead of "M1b" and "M2" per the new dispatcher text. **If the test-message substring is now stale, update the test's expected substring** in the same commit:

  Old assert: `.contains("OpenAI adapter is not yet implemented")`
  New assert: same — that exact phrase is preserved in both old and new text.

- [ ] **Step 8: Commit**

```bash
cd /Users/dondy/Codes/warp
git add app/src/server/server_api.rs app/src/server/warp_adapter.rs
git commit -m "refactor(server): route Warp dispatch through AiProvider trait"
```

After committing: `git log --oneline -3`.

---

### Task 5: Verify and finalize

**Files:** none

- [ ] **Step 1: Full clippy on touched crates**

Run:
```bash
cargo clippy -p ai_provider -p warp --tests --all-targets -- -D warnings 2>&1 | tail -10
```
Expected: 0 errors. Fix any in place.

- [ ] **Step 2: Full nextest on touched crates**

Run:
```bash
cargo nextest run -p ai_provider -p warp --no-fail-fast 2>&1 | tail -15
```
Expected: no NEW failures vs. M1a's known set. Pre-existing failures (`test_migration_does_not_rerun_when_marker_present`, the SSH ones, the 3 flaky terminal::view ones if they appear) are acceptable.

- [ ] **Step 3: Manual smoke**

Run: `./script/run`. Send a simple prompt in Agent Mode (default Warp protocol — your existing login state determines whether it works as before). The behavior should be IDENTICAL to before M1b-prep — same login UI, same Agent Mode behavior. The only thing different is now requests flow through the trait instead of the inline match.

If the Warp-default flow doesn't work as before, the most likely culprits are:

1. The function body was pasted incorrectly into `generate_multi_agent_output_via_warp` (missing `cfg_if!` block, wrong return type alias, etc.) — diff against M1a's state to find the discrepancy.
2. `Arc<Self>` receiver mismatch at a call site — escalate.

Quit cleanly.

- [ ] **Step 4: Confirm git log is clean**

Run: `git log --oneline m1a-protocol-dispatch..HEAD`
Expected: 4 commits, in this order from oldest to newest:
1. `build(ai_provider): add deps and stub modules for trait+error`
2. `refactor(ai_provider): move AIApiError out of server_api`
3. `feat(ai_provider): define AiProvider trait`
4. `refactor(server): route Warp dispatch through AiProvider trait`

If the count is off or commits look different, escalate.

---

## Self-Review Checklist (run before declaring M1b-prep done)

- [ ] `cargo check --workspace` passes.
- [ ] `cargo clippy -p ai_provider -p warp --tests --all-targets -- -D warnings` clean.
- [ ] `cargo nextest run -p ai_provider -p warp` shows no new failures.
- [ ] `AIApiError` is now defined in `crates/ai_provider/src/error.rs`. Verified by `rg "pub enum AIApiError" crates/ai_provider/`.
- [ ] `app/src/server/server_api.rs` no longer contains `pub enum AIApiError`. Verified by `rg "pub enum AIApiError" app/`.
- [ ] External callers that did `use crate::server::server_api::AIApiError;` still work (via the re-export). Verified by `cargo check --workspace` passing.
- [ ] `WarpServerAdapter` exists at `app/src/server/warp_adapter.rs` and impls `AiProvider`.
- [ ] `generate_multi_agent_output_via_warp` is `pub(crate) async fn` and contains the body that was originally in `generate_multi_agent_output`.
- [ ] Manual smoke (Task 5 Step 3) shows no behavior regression.
- [ ] `git log --oneline m1a-protocol-dispatch..HEAD` shows 4 commits with the expected messages.

## Out of scope for M1b-prep (deferred to M1b-chat)

- **`OpenAiAdapter` implementation.** The trait now exists; M1b-chat plugs in the second impl that talks OpenAI Chat Completions.
- **Provider config beyond env vars.** Settings GUI (M3) is far down the road.
- **Tool definition translation.** M1c.
- **System prompt construction from `TaskContext`.** M1b-chat — needs the real adapter to consume it.
- **Removing the `AIOutputStream` alias** in favor of `ResponseEventStream` everywhere. Possible follow-up; out of scope here.

# M1a — Protocol Dispatch Seam Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a single env-var-driven protocol dispatch point inside `ServerApi::generate_multi_agent_output` so that M1b (OpenAI adapter) and M2 (Anthropic adapter) can plug in without churn. Default path (`WARP_AI_PROTOCOL` unset) is byte-for-byte identical to today's behavior.

**Architecture:** New leaf crate `crates/ai_provider/` defining a `Protocol` enum (`Warp` | `OpenAi` | `Anthropic`) and `resolve_protocol_from_env()`. `generate_multi_agent_output` reads the protocol at function entry; `Protocol::Warp` falls through to the existing implementation, `Protocol::OpenAi` and `Protocol::Anthropic` early-return a clear `AIApiError::Other("not yet implemented (M1b/M2)")` so misconfiguration is surfaced immediately rather than silently. **No `AiProvider` trait, no `WarpServerAdapter`, no error-type relocation in this milestone** — those land in M1b *with* the OpenAI adapter that justifies them.

**Tech Stack:** Rust 2021, workspace deps only (`thiserror`, no new external deps for M1a). Tests via `cargo nextest`.

---

## Context

The existing function at `app/src/server/server_api.rs:1071–1163` POSTs Warp's protobuf request to `${ChannelState::server_root_url()}/ai/multi-agent` (or `/ai/passive-suggestions`) and decodes a base64-encoded SSE stream of `warp_multi_agent_api::ResponseEvent`. It's called by `app/src/ai/agent/api/impl.rs:132`, `app/src/ai/blocklist/passive_suggestions/maa.rs:177`, and `app/src/ai/blocklist/controller/response_stream.rs:97,160`.

`AIApiError` is defined inline in `server_api.rs:155–284` (the enum, three `From` impls, and four helper methods including async `from_stream_error`). It pulls in `DeserializationError`, `WARP_ERROR_CODE_HEADER`, `WARP_ERROR_CODE_OUT_OF_CREDITS`, `http_client::ResponseError`, plus `hyper::Error` (in a `cfg(not(target_family = "wasm"))` arm). **Moving it is a real refactor; we defer that to M1b** when an OpenAI adapter genuinely needs to construct `AIApiError` variants from outside the file.

**Naming:** there's already a widely-used `pub trait AIClient` (capital I) at `app/src/server/server_api/ai.rs:731` covering ~30 other AI methods. To avoid confusion when M1b introduces our trait, we use **`AiProvider`** (matches the crate name). M1a just reserves the crate name; the trait lands in M1b.

`mockito` (not `wiremock`) is the in-tree HTTP mock at workspace `Cargo.toml:354`. M1b/M1c plans will use it.

## File Structure

**New:**

| Path | Responsibility |
|---|---|
| `crates/ai_provider/Cargo.toml` | New crate manifest, edition 2021 |
| `crates/ai_provider/src/lib.rs` | Crate root with `Protocol` enum + `resolve_protocol_from_env` |

**Modified:**

| Path | Change |
|---|---|
| `Cargo.toml` (root) | Add `ai_provider = { path = "crates/ai_provider" }` to `[workspace.dependencies]` |
| `app/Cargo.toml` | Add `ai_provider.workspace = true` to `[dependencies]` |
| `app/src/server/server_api.rs:1071` | Insert protocol-dispatch prologue at the top of `generate_multi_agent_output` |

---

## Tasks

### Task 1: Create the `ai_provider` crate

**Files:**
- Create: `crates/ai_provider/Cargo.toml`
- Create: `crates/ai_provider/src/lib.rs`

- [ ] **Step 1: Create the manifest**

Create `crates/ai_provider/Cargo.toml`:

```toml
[package]
name = "ai_provider"
version = "0.1.0"
edition = "2021"
publish.workspace = true
license.workspace = true

[dependencies]
```

> Empty `[dependencies]` is intentional — M1a only needs `std::env`. Workspace deps come in M1b.

- [ ] **Step 2: Create `lib.rs` with `Protocol` and the resolver**

Create `crates/ai_provider/src/lib.rs`:

```rust
//! AI provider abstraction for Warp.
//!
//! M1a: defines [`Protocol`] and [`resolve_protocol_from_env`] — used by
//! `ServerApi::generate_multi_agent_output` to decide whether to use Warp's
//! hosted backend (default) or a user-supplied OpenAI/Anthropic-compatible
//! endpoint. The `OpenAi` and `Anthropic` variants are accepted by the
//! resolver but are *not yet implemented* by callers; selecting them in M1a
//! produces a clear `not yet implemented` error from the dispatcher.
//!
//! M1b adds the `AiProvider` trait, `OpenAiAdapter`, and a `WarpServerAdapter`
//! wrapping the current behavior.

/// Selected backend for AI requests.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Protocol {
    /// Warp's hosted multi-agent service (default).
    Warp,
    /// User-supplied OpenAI-compatible endpoint (M1b).
    OpenAi,
    /// User-supplied Anthropic Messages API endpoint (M2).
    Anthropic,
}

/// Read [`Protocol`] from `WARP_AI_PROTOCOL`. Unknown / unset values fall back
/// to [`Protocol::Warp`], preserving existing behavior.
pub fn resolve_protocol_from_env() -> Protocol {
    match std::env::var("WARP_AI_PROTOCOL").ok().as_deref() {
        Some("openai") => Protocol::OpenAi,
        Some("anthropic") => Protocol::Anthropic,
        _ => Protocol::Warp,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Helper: set `WARP_AI_PROTOCOL` to `value` for the duration of `f`,
    /// restoring the previous value afterward. Needed because env vars are
    /// process-global and tests may run in any order.
    fn with_env<F: FnOnce()>(value: Option<&str>, f: F) {
        let prev = std::env::var("WARP_AI_PROTOCOL").ok();
        match value {
            Some(v) => std::env::set_var("WARP_AI_PROTOCOL", v),
            None => std::env::remove_var("WARP_AI_PROTOCOL"),
        }
        f();
        match prev {
            Some(v) => std::env::set_var("WARP_AI_PROTOCOL", v),
            None => std::env::remove_var("WARP_AI_PROTOCOL"),
        }
    }

    #[test]
    fn defaults_to_warp_when_unset() {
        with_env(None, || {
            assert_eq!(resolve_protocol_from_env(), Protocol::Warp);
        });
    }

    #[test]
    fn picks_openai_for_explicit_value() {
        with_env(Some("openai"), || {
            assert_eq!(resolve_protocol_from_env(), Protocol::OpenAi);
        });
    }

    #[test]
    fn picks_anthropic_for_explicit_value() {
        with_env(Some("anthropic"), || {
            assert_eq!(resolve_protocol_from_env(), Protocol::Anthropic);
        });
    }

    #[test]
    fn falls_back_to_warp_for_unknown_value() {
        with_env(Some("garbage"), || {
            assert_eq!(resolve_protocol_from_env(), Protocol::Warp);
        });
    }
}
```

- [ ] **Step 3: Verify the crate compiles standalone**

Run: `cargo check -p ai_provider`
Expected: PASS, no warnings.

If a warning fires for unused imports / dead code, fix it before proceeding.

- [ ] **Step 4: Run the unit tests in isolation**

Run: `cargo nextest run -p ai_provider`
Expected: All four tests PASS.

If tests race (env var is process-global), the four `with_env` helpers should still be safe because they restore state per test and `cargo nextest` runs tests in separate processes by default. If a flake appears, add `serial_test` as a workspace dev-dep — most likely unnecessary.

- [ ] **Step 5: Commit**

```bash
git add crates/ai_provider
git commit -m "feat(ai_provider): scaffold crate with Protocol enum"
```

---

### Task 2: Register `ai_provider` in the workspace

**Files:**
- Modify: `Cargo.toml` (workspace root)
- Modify: `app/Cargo.toml`

- [ ] **Step 1: Add to workspace dependencies**

In the repo root `Cargo.toml`, find `[workspace.dependencies]` and add the entry alphabetically (the existing line `ai = { path = "crates/ai" }` is around line 32):

```toml
ai_provider = { path = "crates/ai_provider" }
```

Run: `rg -n "^ai = " Cargo.toml` to find the line if its position has shifted; insert directly after it.

- [ ] **Step 2: Add to `app/Cargo.toml`**

In `app/Cargo.toml`, find `[dependencies]` and add (alphabetically near `ai.workspace = true`):

```toml
ai_provider.workspace = true
```

- [ ] **Step 3: Verify the workspace still builds**

Run: `cargo check --workspace`
Expected: PASS.

If failure mentions `ai_provider` not in workspace members: the `crates/*` glob in `[workspace] members` should already include it. Verify with `rg "members" Cargo.toml`. If `crates/*` is the pattern, the new crate is auto-discovered.

- [ ] **Step 4: Commit**

```bash
git add Cargo.toml app/Cargo.toml
git commit -m "build: register ai_provider in workspace"
```

---

### Task 3: Add the dispatch prologue to `generate_multi_agent_output`

**Files:**
- Modify: `app/src/server/server_api.rs:1071-1080` (insert before the existing function body)

- [ ] **Step 1: Read the current function header to ground the edit**

Run: `sed -n '1071,1090p' app/src/server/server_api.rs`. The function declaration spans lines 1071–1075:

```rust
pub async fn generate_multi_agent_output(
    &self,
    request: &warp_multi_agent_api::Request,
) -> std::result::Result<AIOutputStream<warp_multi_agent_api::ResponseEvent>, Arc<AIApiError>>
{
```

The first line of the body is at line 1076: `let auth_token = self.get_or_refresh_access_token()...`

- [ ] **Step 2: Insert the dispatch prologue before line 1076**

Edit `app/src/server/server_api.rs`. Immediately after the opening brace of `generate_multi_agent_output` (line 1075) and before `let auth_token = ...` (line 1076), insert:

```rust
        // Protocol dispatch (M1a). When the user has not selected a custom
        // provider, fall through to the existing Warp-hosted implementation.
        // OpenAI and Anthropic adapters land in M1b/M2; for now they
        // produce a clear error rather than silently calling Warp.
        match ai_provider::resolve_protocol_from_env() {
            ai_provider::Protocol::Warp => {
                // fall through to the existing implementation below
            }
            ai_provider::Protocol::OpenAi => {
                return Err(Arc::new(AIApiError::Other(anyhow::anyhow!(
                    "WARP_AI_PROTOCOL=openai requested, but the OpenAI adapter \
                     is not yet implemented (planned for M1b)"
                ))));
            }
            ai_provider::Protocol::Anthropic => {
                return Err(Arc::new(AIApiError::Other(anyhow::anyhow!(
                    "WARP_AI_PROTOCOL=anthropic requested, but the Anthropic \
                     adapter is not yet implemented (planned for M2)"
                ))));
            }
        }
```

> The match expression's arm bodies use `return`, so reaching the existing code below is only possible when the protocol is `Warp`. The remaining function body (lines 1076–1163) is unchanged.

- [ ] **Step 3: Verify the file compiles**

Run: `cargo check -p app`
Expected: PASS.

If the import `ai_provider` isn't recognized, the `app/Cargo.toml` change from Task 2 didn't apply — re-check.

- [ ] **Step 4: Run the full workspace test suite**

Run: `cargo nextest run --workspace --no-fail-fast`
Expected: PASS or pre-existing flakes only. The Warp-default code path is unchanged so all existing tests should still pass.

If any *new* failure appears, the most likely cause is a test that explicitly sets `WARP_AI_PROTOCOL` for unrelated reasons (unlikely — `rg "WARP_AI_PROTOCOL" -t rust` will confirm). Fix in place if surfaced.

- [ ] **Step 5: Commit**

```bash
git add app/src/server/server_api.rs
git commit -m "feat(server): dispatch generate_multi_agent_output by Protocol"
```

---

### Task 4: Integration test — verify the non-Warp early-return path

**Files:**
- Modify: `app/src/server/server_api.rs` (append to existing `#[cfg(test)] mod tests`, or create one if absent)

- [ ] **Step 1: Locate the existing test module**

Run: `rg -n "#\[cfg\(test\)\]" app/src/server/server_api.rs | head -3`. The existing `new_for_test()` helper lives at line 425 inside `impl ServerApi { ... }`, which means tests in this file already use that helper. There may or may not be a top-level `#[cfg(test)] mod tests` block — check with `rg -n "^mod tests" app/src/server/server_api.rs`.

If a `#[cfg(test)] mod tests` block exists: append to it.
If none: add one at the bottom of the file.

- [ ] **Step 2: Add the test**

Append (creating the module if needed):

```rust
#[cfg(test)]
mod m1a_dispatch_tests {
    use super::*;

    /// Helper from the same file (`impl ServerApi { fn new_for_test() ... }`).
    /// Visibility is `fn` (private to the impl), but accessible from this test
    /// module because `super::*` includes the impl's items.
    fn server_api_for_test() -> ServerApi {
        ServerApi::new_for_test()
    }

    /// Helper that scopes an env-var change to a single test.
    fn with_env<F: FnOnce()>(value: Option<&str>, f: F) {
        let prev = std::env::var("WARP_AI_PROTOCOL").ok();
        match value {
            Some(v) => std::env::set_var("WARP_AI_PROTOCOL", v),
            None => std::env::remove_var("WARP_AI_PROTOCOL"),
        }
        f();
        match prev {
            Some(v) => std::env::set_var("WARP_AI_PROTOCOL", v),
            None => std::env::remove_var("WARP_AI_PROTOCOL"),
        }
    }

    #[tokio::test]
    async fn openai_protocol_returns_not_implemented_error() {
        let server_api = server_api_for_test();
        let request = warp_multi_agent_api::Request::default();

        let result = std::sync::Arc::new(tokio::sync::Mutex::new(None::<
            std::result::Result<
                AIOutputStream<warp_multi_agent_api::ResponseEvent>,
                std::sync::Arc<AIApiError>,
            >,
        >));
        let result_for_set = std::sync::Arc::clone(&result);

        with_env(Some("openai"), || {
            // We need a Tokio context to call the async function from a sync
            // closure. Build a tiny runtime.
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("tokio rt");
            let res = rt.block_on(server_api.generate_multi_agent_output(&request));
            *result_for_set.blocking_lock() = Some(res);
        });

        let res = result.lock().await.take().expect("result was set");
        let err = res.err().expect("expected Err for openai protocol");
        let s = format!("{err:#}");
        assert!(
            s.contains("OpenAI adapter is not yet implemented"),
            "unexpected error: {s}"
        );
    }

    #[tokio::test]
    async fn anthropic_protocol_returns_not_implemented_error() {
        let server_api = server_api_for_test();
        let request = warp_multi_agent_api::Request::default();

        let result = std::sync::Arc::new(tokio::sync::Mutex::new(None::<
            std::result::Result<
                AIOutputStream<warp_multi_agent_api::ResponseEvent>,
                std::sync::Arc<AIApiError>,
            >,
        >));
        let result_for_set = std::sync::Arc::clone(&result);

        with_env(Some("anthropic"), || {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("tokio rt");
            let res = rt.block_on(server_api.generate_multi_agent_output(&request));
            *result_for_set.blocking_lock() = Some(res);
        });

        let res = result.lock().await.take().expect("result was set");
        let err = res.err().expect("expected Err for anthropic protocol");
        let s = format!("{err:#}");
        assert!(
            s.contains("Anthropic adapter is not yet implemented"),
            "unexpected error: {s}"
        );
    }
}
```

> The blocking_lock dance is needed because `with_env` is sync (env-var reset must run after the async call resolves). Cleaner alternative: skip `with_env` and just `set_var`/`remove_var` directly, accepting the test isn't fully isolated. If `cargo nextest` is configured to run each test in its own process (the default), direct `set_var`/`remove_var` is safe and you can simplify the test bodies dramatically:

```rust
    #[tokio::test]
    async fn openai_protocol_returns_not_implemented_error() {
        std::env::set_var("WARP_AI_PROTOCOL", "openai");
        let server_api = ServerApi::new_for_test();
        let request = warp_multi_agent_api::Request::default();
        let err = server_api
            .generate_multi_agent_output(&request)
            .await
            .err()
            .expect("expected Err");
        assert!(format!("{err:#}").contains("OpenAI adapter is not yet implemented"));
        std::env::remove_var("WARP_AI_PROTOCOL");
    }
```

**Use the simpler form** unless `cargo nextest --no-default-features` or local config disables process isolation. Verify with `rg "test-threads|isolated" .config/nextest.toml` and `rg "test_threads" Cargo.toml` — if no overrides, isolation is on by default.

- [ ] **Step 3: Run the new tests**

Run: `cargo nextest run -p app -E 'test(openai_protocol_returns_not_implemented_error) | test(anthropic_protocol_returns_not_implemented_error)'`
Expected: Both PASS.

If `ServerApi::new_for_test()` isn't accessible (visibility issue), check the `fn` line at `app/src/server/server_api.rs:425`. If it's `fn` (private), find an existing test in the same file that uses it as a model — if other tests call it, the visibility works for our tests too. If not, change `fn new_for_test` to `pub(crate) fn new_for_test` and add a brief comment explaining why.

- [ ] **Step 4: Commit**

```bash
git add app/src/server/server_api.rs
git commit -m "test(server): cover Protocol dispatch error paths"
```

---

### Task 5: Manual verification

**Files:** none

- [ ] **Step 1: Default path — confirm no behavior change**

Run: `./script/run`

In the GUI:
1. Open a fresh terminal pane.
2. Open Agent Mode.
3. Send a simple prompt: `what is 2+2?`
4. Confirm a response streams in normally — this exercises the Warp-default branch and verifies M1a hasn't broken anything.
5. Quit the app cleanly.

- [ ] **Step 2: OpenAI path — confirm clear error**

Run: `WARP_AI_PROTOCOL=openai ./script/run`

In the GUI:
1. Open Agent Mode.
2. Send any prompt.
3. Confirm an error surfaces (the exact UX depends on how `generate_multi_agent_output` errors render today — typically a red toast or inline error block). The error message should mention "OpenAI adapter is not yet implemented".
4. Quit cleanly.

- [ ] **Step 3: Anthropic path — confirm clear error**

Run: `WARP_AI_PROTOCOL=anthropic ./script/run`

Same as Step 2 but expecting "Anthropic adapter is not yet implemented".

- [ ] **Step 4: Garbage value — confirm fallback to Warp**

Run: `WARP_AI_PROTOCOL=banana ./script/run`

Confirm Agent Mode works normally — the resolver falls back to `Protocol::Warp` for unrecognized values.

- [ ] **Step 5: Verify the working tree is clean (no uncommitted changes from manual smoke)**

Run: `git status`
Expected: working tree clean.

---

### Task 6: Run presubmit and finalize

**Files:** none

- [ ] **Step 1: Full presubmit (fmt, clippy, tests)**

Run: `./script/presubmit`
Expected: PASS.

If clippy or fmt fail on the new code, fix in place. If a workspace-wide presubmit issue surfaces unrelated to M1a, follow the `fix-errors` skill.

- [ ] **Step 2: Confirm git log looks clean**

Run: `git log --oneline -10`
Expected: Four (or five, if the test-helper visibility was bumped to `pub(crate)`) M1a commits with descriptive messages, in order. No "fixup", "wip", or "tmp" commits.

If the history is messy from iteration, *do not rewrite* — leave it. The reviewer will see the actual diff in the PR.

---

## Self-Review Checklist (run before declaring M1a done)

- [ ] `cargo check --workspace` passes.
- [ ] `cargo nextest run --workspace --no-fail-fast` shows no new failures vs. main.
- [ ] `./script/presubmit` passes.
- [ ] All four `Protocol` variants resolve correctly (Task 1 unit tests + Task 4 integration tests cover this).
- [ ] Default path (`WARP_AI_PROTOCOL` unset) produces *byte-identical* behavior to pre-M1a — verified by Task 5 Step 1.
- [ ] Setting `WARP_AI_PROTOCOL=openai` or `=anthropic` produces a clear, actionable error (no panic, no silent fallback) — verified by Task 5 Steps 2 and 3.
- [ ] Garbage env var values fall back to Warp — verified by Task 5 Step 4.
- [ ] The new crate has zero non-workspace dependencies (M1a doesn't justify them yet).
- [ ] No code outside `ai_provider/src/lib.rs` and the dispatch prologue in `server_api.rs` was touched. Verify with `git diff main --stat` — exactly four files changed: two new, two modified.

## Out of scope for M1a (deferred to later plans)

- `AiProvider` trait definition (M1b — lands with the first impl)
- `WarpServerAdapter` wrapper (M1b — lands when the trait exists)
- Moving `AIApiError` to `ai_provider` (M1b — driven by OpenAI adapter needing to construct variants)
- OpenAI Chat Completions client (M1b)
- System prompt construction from `TaskContext` (M1b — needs the OpenAI adapter as a consumer)
- Anthropic Messages API client (M2)
- Tool definition translation (`ToolType` → OpenAI `tools[]`, M1c)
- Tool call delta parsing (M1c)
- `ToolCallResult` variant translation (M1c — 30+ variants, table-driven)
- Settings GUI panel (M3)
- Cloud feature stripping (M4)
- Capability detection / runtime tool support flag (M5)

## Why M1a is this small

The original M1 plan (and the design's M1 description) bundled "trait + adapter + OpenAI client" into one milestone. Surveying the proto schema + existing error types revealed two facts that make a smaller M1a strictly better:

1. **Moving `AIApiError` is a real refactor**, pulling in `DeserializationError`, two header constants, an `http_client::ResponseError` `From` impl, and a `hyper::Error` conditional. Doing this *before* there's a consumer that needs it is speculative.
2. **An `AiProvider` trait with no implementations is dead code**. YAGNI.

M1a establishes the dispatch point only. M1b is now naturally scoped: introduce the trait (because the OpenAI adapter implements it), introduce `WarpServerAdapter` (because the trait needs the existing path as the second impl), and move `AIApiError` (because the OpenAI adapter needs to construct its variants from outside the file). All three changes land together with their consumer, exactly when YAGNI says they should.

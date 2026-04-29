# M4a — Skip Onboarding & Login on First Launch (Implementation Plan)

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** When the binary is built for the OSS channel (the default for this fork), the app must boot **straight to the terminal** on first launch — no welcome slides, no AI agent onboarding, no login slide, no "Sign up" / "Sign in" UI on the boot path. The cloud surfaces (Drive, Sessions, Workspaces) remain visible in the UI but inert; M4b strips those.

**Architecture:** A single semantic check at the auth-state branch in `RootView::new`. We add `ChannelState::is_cloud_enabled() -> bool` (returns `false` for `Channel::Oss`, `true` for any other channel) and short-circuit the auth-onboarding chain to `AuthOnboardingState::Terminal(...)` before any of the existing branches are evaluated. **No code is removed**, no enum variants are deleted — this is purely an early-return that bypasses the existing logic. M4b will follow up by hiding the cloud-feature UI surfaces; M4c will gate telemetry and verify no warp.dev egress.

**Tech Stack:** Rust 2021 only. Two file edits, ~15 lines of code, plus 2 unit tests.

---

## Context

Today, `app/src/root_view.rs:1745–1786` decides initial UI state via this chain (paraphrased):

```
if auth_state.is_logged_in()           → Terminal
else if ForceLogin enabled              → Auth (login screen)
else if pre-login onboarding eligible   → Onboarding (AI agent slides)
else if SkipFirebaseAnonymousUser flag  → Terminal
else                                    → Auth (login screen)
```

Result on a fresh OSS install: the user lands on `Auth` (the "Sign up / Sign in" screen) because none of the other gates fire. The user reported this as the first thing they want gone.

The `Channel::Oss` enum variant already exists at `crates/warp_core/src/channel/mod.rs:33` and is used to gate telemetry/crash-reporting today. `ChannelState::init()` defaults to `Channel::Oss` (`crates/warp_core/src/channel/state.rs:39`), so any non-rebranded build of this fork is OSS by default.

We add a `is_cloud_enabled()` static method (mirroring the existing `enable_debug_features()` at `crates/warp_core/src/channel/state.rs:81–83`) and consult it once in `root_view.rs`. Done.

## File Structure

**Modified:**

| Path | Change |
|---|---|
| `crates/warp_core/src/channel/state.rs` | Add `pub fn is_cloud_enabled() -> bool` static method on `impl ChannelState` (around line 84, near `enable_debug_features`). Add a `#[cfg(test)]` test for it. |
| `app/src/root_view.rs` | Insert a short-circuit branch at the top of the `let auth_onboarding_state = if ...` chain (around line 1745). Branch returns `AuthOnboardingState::Terminal(...)` when `ChannelState::is_cloud_enabled()` is `false`. |

No new files. No deletions. No new dependencies.

---

## Tasks

### Task 1: Add `ChannelState::is_cloud_enabled()` helper

**Files:**
- Modify: `crates/warp_core/src/channel/state.rs` (around lines 81–83 + test module at the bottom)

- [ ] **Step 1: Read the existing helper that this one mirrors**

Run: `sed -n '77,90p' /Users/dondy/Codes/warp/crates/warp_core/src/channel/state.rs`. The pattern to follow is `enable_debug_features` at line 81–83:

```rust
pub fn enable_debug_features() -> bool {
    cfg!(debug_assertions) || matches!(Self::channel(), Channel::Local | Channel::Dev)
}
```

- [ ] **Step 2: Find where `Self::channel()` is defined**

Run: `rg -n "pub fn channel\(" /Users/dondy/Codes/warp/crates/warp_core/src/channel/state.rs`. Confirm the static accessor exists. (It must — `enable_debug_features` already calls `Self::channel()`.)

- [ ] **Step 3: Add the new helper immediately after `enable_debug_features`**

Edit `crates/warp_core/src/channel/state.rs`. After the closing `}` of `enable_debug_features` (around line 83), add:

```rust
    /// Returns `true` if the build participates in warp.dev cloud features
    /// (Drive, Sessions, Workspaces, login, telemetry). Returns `false` for
    /// the OSS channel — boot path skips onboarding/login when this is `false`.
    pub fn is_cloud_enabled() -> bool {
        !matches!(Self::channel(), Channel::Oss)
    }
```

- [ ] **Step 4: Verify it compiles**

Run: `cargo check -p warp_core`
Expected: PASS, no warnings.

- [ ] **Step 5: Add a unit test for the helper**

Find the existing `#[cfg(test)] mod tests` block in `state.rs` if any (`rg -n "mod tests" /Users/dondy/Codes/warp/crates/warp_core/src/channel/state.rs`). If none, append at the bottom of the file:

```rust
#[cfg(test)]
mod m4a_tests {
    use super::*;

    /// `init()` defaults to `Channel::Oss`, so a fresh `ChannelState` should
    /// report cloud disabled. This is the only path M4a's runtime behavior
    /// depends on — the inverse direction (non-Oss → cloud enabled) is
    /// covered by inspection of the one-line implementation.
    #[test]
    fn fresh_state_is_oss_with_cloud_disabled() {
        ChannelState::set(ChannelState::init());
        assert_eq!(ChannelState::is_cloud_enabled(), false);
    }
}
```

> A second test exercising the non-Oss path (e.g., `Channel::Stable → is_cloud_enabled() == true`) would be ideal but requires constructing a non-default `ChannelConfig`. There's no `ChannelConfig::default_for_channel(...)` constructor in this crate (verified). The existing `ChannelState::new(channel, config)` requires a fully-built `ChannelConfig`, which involves several non-trivial sub-configs (`WarpServerConfig`, `OzConfig`, etc.). Adding a test fixture for that is out of scope for M4a — the helper is a one-line `!matches!(...)` whose negative direction is self-evident from the source. If you'd rather have full coverage, do it in M4b when more `cloud_enabled` checks are added and a fixture is genuinely useful.

- [ ] **Step 6: Run the new test**

Run: `cargo nextest run -p warp_core -E 'test(m4a_tests)'`
Expected: 1 test passes.

- [ ] **Step 7: Commit**

```bash
git add crates/warp_core/src/channel/state.rs
git commit -m "feat(warp_core): add ChannelState::is_cloud_enabled helper"
```

---

### Task 2: Short-circuit auth-state branching for non-cloud builds

**Files:**
- Modify: `app/src/root_view.rs:1745–1786` (the `let auth_onboarding_state = if ... else { ... };` block)

- [ ] **Step 1: Re-read the current shape**

Run: `sed -n '1745,1790p' /Users/dondy/Codes/warp/app/src/root_view.rs`. Confirm the structure starts with `let auth_onboarding_state = if auth_state.is_logged_in() { ... } else { cfg_if! { ... } };`. The line numbers may have drifted slightly — anchor on the `let auth_onboarding_state =` assignment.

- [ ] **Step 2: Confirm `ChannelState` is in scope**

Run: `rg -n "ChannelState" /Users/dondy/Codes/warp/app/src/root_view.rs | head -5`. If no hits, add `use warp_core::channel::ChannelState;` to the imports near the top of the file (at the alphabetically-correct location among other `use warp_core::...` lines).

- [ ] **Step 3: Wrap the existing chain in the OSS short-circuit**

Edit `app/src/root_view.rs`. Find the line `let auth_onboarding_state = if auth_state.is_logged_in() {` (around line 1745) and replace just that opening line with:

```rust
        let auth_onboarding_state = if !ChannelState::is_cloud_enabled() {
            // OSS / fork build: warp.dev integration is disabled. Skip
            // onboarding and login slides; boot straight to the terminal.
            // M4b will hide the cloud-feature surfaces inside the workspace;
            // M4c gates telemetry. Until then, those surfaces are visible
            // but inert.
            AuthOnboardingState::Terminal(workspace_args.create_workspace(ctx))
        } else if auth_state.is_logged_in() {
```

The rest of the chain (the `else { cfg_if! { ... } }` and the closing `};`) is unchanged. Effectively we converted `if A { … } else { … }` into `if S { … } else if A { … } else { … }` where `S` is the short-circuit.

- [ ] **Step 4: Verify it compiles**

Run: `cargo check -p warp`
Expected: PASS.

If `ChannelState` was not imported in Step 2 and Step 3 fails with "cannot find type `ChannelState`", go back and add the use line.

- [ ] **Step 5: Run a focused test that exercises the auth-state branching**

Find existing tests that touch `RootView` or `AuthOnboardingState`:

```bash
rg -ln "AuthOnboardingState|RootView::new" /Users/dondy/Codes/warp/app/src
```

If a test file exists (likely `app/src/root_view_tests.rs`), run that target:

```bash
cargo nextest run -p warp -E 'test(root_view)' 2>&1 | tail -15
```

If no targeted tests cover the branch, that's acceptable for M4a — the branching is purely a `match`/`if` on a global flag and can be covered manually in Task 3.

- [ ] **Step 6: Commit**

```bash
git add app/src/root_view.rs
git commit -m "feat(root_view): boot straight to terminal in OSS channel"
```

---

### Task 3: Manual GUI verification + presubmit

**Files:** none

- [ ] **Step 1: Build + launch**

Run: `./script/run`
Expected: GUI window appears.

- [ ] **Step 2: Confirm the user-visible change**

When the window appears, you should see:

- **No** "Sign up" / "Sign in" / "Continue" screen.
- **No** welcome slides.
- **No** AI agent onboarding (model picker).
- A normal terminal pane, ready for input.

If any of the above appears, M4a's short-circuit isn't working — go back to Task 2 and check that `ChannelState::is_cloud_enabled()` is actually returning `false`. To debug, temporarily add `eprintln!("cloud_enabled: {}", ChannelState::is_cloud_enabled());` near the top of `RootView::new` and re-run. The output will appear in `~/Library/Logs/warp-oss.log` or stderr.

- [ ] **Step 3: Verify the workspace still loads correctly**

Type a command in the terminal pane (e.g. `pwd` or `ls`). Confirm it executes normally. The cloud surfaces (Drive sidebar, etc.) should still be visible — that's fine for M4a; M4b removes them.

- [ ] **Step 4: Quit the app**

`Cmd+Q`. The bg `./script/run` job should exit shortly after.

- [ ] **Step 5: Run clippy on the changed crates**

Run:

```bash
cargo clippy -p warp_core -p warp --tests --all-targets -- -D warnings 2>&1 | tail -10
```

Expected: no errors.

- [ ] **Step 6: Run nextest for the touched crates**

Run:

```bash
cargo nextest run -p warp_core -p warp --no-fail-fast 2>&1 | tail -15
```

Expected: no *new* failures vs. M1a's known pre-existing failures (the SSH integration tests, the settings migration test, the 3 flaky terminal::view tests).

If a test fails specifically because it asserts the auth-onboarding state machine reaches `Auth` or `Onboarding`, that's an *expected* assertion failure — the test was written for cloud-enabled builds and now needs to be updated to skip on the OSS channel. Update the test to add `#[cfg_attr(not(feature = "...some cloud feature flag..."), ignore = "...")]` or — simpler — skip the assertion when `!ChannelState::is_cloud_enabled()`. **Document any such adjustment as a separate commit titled `test(root_view): skip cloud-only assertions in OSS channel`.**

---

## Self-Review Checklist (run before declaring M4a done)

- [ ] On a fresh `./script/run`, the app boots straight to the terminal — no slides, no auth screen.
- [ ] `ChannelState::is_cloud_enabled()` returns `false` by default (verified by Task 1's first test).
- [ ] The diff is small: 2 files modified, ~15 lines of code, 2 commits (feat + feat) plus optional test-update commit.
- [ ] No code was *deleted* — the existing onboarding/login UI is untouched, only bypassed. (M4b/M4c will deal with cleanup.)
- [ ] `cargo clippy -p warp_core -p warp` passes with `-D warnings`.
- [ ] No new test failures vs. M1a's known set.
- [ ] Manual smoke (Task 3 Steps 1–4) completed by the user.

## Out of scope for M4a (deferred to M4b and M4c)

- **Hiding the cloud feature UI surfaces.** Drive sidebar entry, Sessions menu item, Workspaces selector, "Sign in" banner in the workspace top bar, "Sign up" entry in user menu — all still rendered in M4a. They become inert (no user action triggers a cloud call because there's no auth) but visible. M4b adds `cloud_enabled` checks at each surface to hide them.
- **Telemetry suppression.** Today the telemetry sender at `app/src/server/telemetry/mod.rs:213–263` will still attempt to send events. Most events will fail silently (no auth), but they'll consume CPU. M4c makes it a no-op when cloud is disabled.
- **`ForceLogin` neutralization.** Today the `FeatureFlag::ForceLogin` is checked at `app/src/root_view.rs:1768` and `app/src/auth/auth_view_body.rs:188`. The M4a short-circuit fires *before* this check, so it doesn't matter for the boot path. M4b can additionally force the flag to `false` so anyone reading the code gets a clear signal.
- **Welcome slide / theme picker code deletion.** The `crates/onboarding/src/` slide files are still compiled. They're just never rendered. Removing them is an M5+ cleanup (lots of dead-code propagation; not worth doing in the same PR as the visible behavior change).
- **Egress test.** Verifying no `*.warp.dev` outbound HTTP requests is M4c's job.

## Why M4a is this small

The user's complaint was "I still see sign up and sign in." That's a single point in the UI tree. Fixing it doesn't require ripping out the underlying machinery — it just requires not *rendering* it. The smallest patch that achieves the visible effect is:

1. One static helper to express "is cloud enabled" semantically.
2. One `if !cloud_enabled { Terminal }` short-circuit at the boot path.

Everything else (hiding surfaces, gating telemetry, deleting code) is meaningful work but **not required** for the fork to feel right on first launch. Splitting them into M4b/M4c keeps each commit reviewable and each gate verifiable.

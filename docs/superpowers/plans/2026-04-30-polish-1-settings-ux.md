# Polish-1 — Settings UX cleanup

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Six follow-up polish items the user spotted after dogfooding M1c. Each one is independent and small-to-medium. Pragmatic fixes, no over-engineering.

**Items:**

1. Clicking the gear icon opens Settings directly (no popover/dropdown).
2. **Keychain re-prompt every build** → fix by storing the API key in the same TOML as endpoint/model/protocol (no more Keychain).
3. Hide the **Account** sidebar section in OSS mode.
4. **Model field becomes a dropdown** populated by `GET {endpoint}/v1/models` after the API key is entered. "Test connection" button is replaced by a "Connect" button that fetches the model list. Selecting a model from the dropdown saves it to settings.
5. Hide the per-profile **Base model** + **Full terminal use model** dropdowns (Profile editor) in OSS mode — single global model only.
6. Hide the input-area **/MODEL** picker pulldown in OSS mode — same reason.

---

## File Structure

**Modify:**

| Path | Change |
|---|---|
| `app/src/settings_view/mod.rs` | Hide `SettingsSection::Account` when `!ChannelState::is_cloud_enabled()`. |
| `app/src/settings_view/ai_provider_page.rs` | Replace API key Keychain calls with TOML settings. Replace "Test connection" with "Connect" that fetches `/v1/models`. Replace model text input with a dropdown populated from the fetched list. |
| `app/src/settings/ai_provider.rs` | Add `api_key: String` setting (TOML-stored). Add `available_models: Vec<String>` ephemeral cache (in-memory only — not synced). |
| `crates/ai_provider/src/openai.rs` | Update `runtime_config` plumbing if any code reads the API key from secure storage — switch to settings-driven. |
| Find: profile editor base-model + full-terminal-model UI | Wrap in `if ChannelState::is_cloud_enabled() { ... }`. |
| Find: input-area /MODEL picker | Wrap in `if ChannelState::is_cloud_enabled() { ... }`. |
| Find: gear icon dropdown | Replace dropdown with direct `dispatch_typed_action(Open Settings)` call. |

---

## Task 1: Hide Account section in sidebar

**Files:**
- Modify: `app/src/settings_view/mod.rs`

- [ ] **Step 1:** Find the existing nav-list construction (around line 1186, after T1's filter for cloud tabs). The line `items.push(SettingsNavItem::Page(SettingsSection::Account))` sits in the unconditional section.

- [ ] **Step 2:** Wrap the Account push in `if cloud {}` (alongside the existing cloud-tabs gate). With cloud disabled, the Account tab is dropped.

- [ ] **Step 3:** Verify compile + manual smoke (Account no longer in sidebar).

- [ ] **Step 4:** Commit: `feat(settings): hide Account section in OSS mode`.

---

## Task 2: Migrate API key from Keychain to TOML

**Files:**
- Modify: `app/src/settings/ai_provider.rs` (add `api_key` setting)
- Modify: `app/src/settings_view/ai_provider_page.rs` (replace Keychain calls)

The Keychain re-prompt issue is caused by ad-hoc signing changing on every rebuild — macOS treats each rebuild as a new app and re-prompts for Keychain access. For an OSS dev fork, the simplest fix is to store the API key in the existing settings TOML alongside endpoint/model/protocol.

The TOML lives at `~/Library/Application Support/dev.warp.WarpOss/settings.toml` (chmod 600 by default). For a single-user OSS fork this trades "Keychain secure" for "no prompts" — acceptable tradeoff.

- [ ] **Step 1:** Add an `api_key` setting in `app/src/settings/ai_provider.rs`:

```rust
api_key: AiProviderApiKey {
    type: String,
    default: "".to_string(),
    storage_key: "AiProviderApiKey",
    toml_path: "ai_provider.api_key",
    private: true,    // mark as private so it doesn't sync to cloud
    // (other fields matching theme.rs's existing pattern)
},
```

> **Important:** mark `private: true` and `sync_to_cloud: SyncToCloud::Never` (or whatever the off-value is per theme.rs's enum). The API key must NOT be synced anywhere off the local machine.

- [ ] **Step 2:** In `ai_provider_page.rs`:
   - **Delete** `load_byo_api_key`, `save_byo_api_key`, `API_KEY_STORAGE_KEY` (Keychain helpers).
   - In the API key editor's `subscribe_to_view` callback, use `AiProviderSettings::handle(ctx).update(ctx, |s, ctx| { let _ = s.api_key.set_value(text, ctx); })` like the other fields.
   - At page-open time, read the saved api_key from `AiProviderSettings::as_ref(ctx).api_key.value()` (mirrors how `endpoint` is loaded).
   - **Remove the `use warpui_extras::secure_storage::AppContextExt as _;` import** since we no longer need Keychain.

- [ ] **Step 3:** Update `push_runtime_config_from_settings(ctx)` — read the api_key from settings, not secure_storage:

```rust
let api_key = settings.api_key.value().to_string();
// Drop the ctx.secure_storage() call entirely.
```

- [ ] **Step 4:** Update the dispatcher's `Protocol::OpenAi` arm in `app/src/server/server_api.rs` (the `openai_config_from_settings_or_env` helper, or equivalent). It probably also reads from `secure_storage` — replace with settings.

- [ ] **Step 5:** Remove the `AiProviderSettings::register` call to update — actually it should still register, just with the additional field.

- [ ] **Step 6:** Verify: rebuild + relaunch + saved api_key persists in `settings.toml` (no Keychain prompts).

- [ ] **Step 7:** Commit: `refactor(settings): store AI Provider API key in TOML instead of Keychain`.

---

## Task 3: Gear icon opens Settings directly

**Files:**
- Find via grep: `rg -n "gear\|cog\|SettingsButton\|settings_button" /Users/dondy/Codes/warp/app/src 2>/dev/null | head -10`

- [ ] **Step 1:** Locate the gear/cog icon's click handler. Likely fires a popover/dropdown menu with Settings + Sign out + ... items.

- [ ] **Step 2:** Replace the handler so it directly dispatches the `OpenSettings` action (or whatever opens the settings window) — no popover.

- [ ] **Step 3:** If the popover has OTHER useful items (besides Settings), we may need to find a different home for them. List them in your report — for now, just open Settings on click and document any items that got hidden.

- [ ] **Step 4:** Verify visually: clicking the gear opens Settings directly.

- [ ] **Step 5:** Commit: `feat(ui): gear icon opens Settings directly in OSS mode`.

---

## Task 4: /v1/models dropdown + "Connect" button

**Files:**
- Modify: `app/src/settings_view/ai_provider_page.rs`

This task replaces the model **text input** with a **dropdown** populated by querying `GET {endpoint}/v1/models` with the configured API key. The "Test connection" button is repurposed/replaced as a "Connect" button that fetches the list.

- [ ] **Step 1:** Add a new method in `OpenAiAdapter` (or as a free function in `ai_provider`) to fetch the model list:

```rust
pub async fn fetch_available_models(
    endpoint: &str,
    api_key: &str,
) -> Result<Vec<String>, Arc<AIApiError>> {
    let url = format!("{}/models", endpoint.trim_end_matches('/'));
    let client = reqwest::Client::new();
    let resp = client
        .get(&url)
        .header("Authorization", format!("Bearer {}", api_key))
        .send()
        .await
        .map_err(|e| Arc::new(AIApiError::Other(anyhow::anyhow!(
            "fetch_models: HTTP error: {e:#}"
        ))))?;
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
    Ok(models)
}
```

> Standard OpenAI `/v1/models` returns `{"data": [{"id": "...", ...}]}`. Many compatible endpoints (LiteLLM, Ollama, OpenRouter) return the same shape.

- [ ] **Step 2:** In `ai_provider_page.rs`:
   - Replace the model `EditorView::single_line` with `Dropdown<...>`.
   - The dropdown starts empty (or with the saved model as the only entry).
   - On "Connect" button click, fetch `/v1/models`, populate the dropdown items with the returned IDs, and pre-select the saved model if present.
   - Selecting a model → save via `AiProviderSettings::model.set_value(...)` and `push_runtime_config_from_settings`.

- [ ] **Step 3:** Rename "Test connection" to "Connect" and update the click handler to call `fetch_available_models` instead of `chat_stream`. Status text:
   - `Idle` (no status)
   - `InProgress` ("Connecting…")
   - `Success(N)` ("✓ Found N models")
   - `Failure(msg)` ("✗ <msg>")

- [ ] **Step 4:** Verify manually: enter endpoint + API key → click Connect → dropdown populated → select a model → saved.

- [ ] **Step 5:** Commit: `feat(settings): /v1/models dropdown + Connect button replaces Test`.

---

## Task 5: Hide per-profile Base model + Full terminal use model dropdowns

**Files:**
- Find: the Profile editor view that renders Base model + Full terminal use model dropdowns. Search:

```bash
rg -n "Base model\|base_model\|Full terminal use" /Users/dondy/Codes/warp/app/src 2>/dev/null | head -10
```

- [ ] **Step 1:** Find the file + function rendering those two dropdowns.

- [ ] **Step 2:** Wrap their construction in `if ChannelState::is_cloud_enabled() { ... }`. In OSS mode, the section disappears (or shows a placeholder text saying "Model is configured in Settings → AI Provider").

- [ ] **Step 3:** Verify visually: opening the Profile editor in OSS mode shows no Models section.

- [ ] **Step 4:** Commit: `feat(profile): hide per-profile model dropdowns in OSS mode`.

---

## Task 6: Hide /MODEL picker in input area

**Files:**
- Find: the input-area pulldown that shows "/MODEL Base / Full Terminal Use" tabs with model options.

```bash
rg -n "/MODEL\|model_picker\|ModelPicker" /Users/dondy/Codes/warp/app/src/terminal/input/ 2>/dev/null | head -10
```

- [ ] **Step 1:** Find the file rendering the /MODEL picker.

- [ ] **Step 2:** Wrap the render in `if ChannelState::is_cloud_enabled() { ... }`.

- [ ] **Step 3:** Verify visually.

- [ ] **Step 4:** Commit: `feat(input): hide /MODEL picker in OSS mode`.

---

## Final verification (Task 7)

- [ ] **Step 1:** Clippy:
   ```bash
   cargo clippy -p ai_provider -p warp --tests --all-targets -- -D warnings
   ```

- [ ] **Step 2:** Nextest:
   ```bash
   cargo nextest run -p ai_provider -p warp --no-fail-fast
   ```

- [ ] **Step 3:** Manual smoke:
   1. Rebuild + launch (no Keychain prompts).
   2. Sidebar: no Account, no cloud sections.
   3. AI Provider tab: enter endpoint + API key → click Connect → models populate → pick one → save.
   4. Profile editor: no Base model / Full terminal use model dropdowns.
   5. Terminal input: no /MODEL pulldown.
   6. Gear icon: opens Settings directly.
   7. Send `/agent` prompt → executes tool calls + responds.

---

## Out of scope (deferred)

- Multi-model per session (different models per Agent feature). The current single-global-model design is fine for OSS. If the user ever wants this, M1c-fanout-style task can re-add the Profile editor's per-feature model picker, this time consuming the `/v1/models` dropdown.
- Encrypted-at-rest TOML for the API key. Plain TOML at file mode 0600 in user's home is acceptable for OSS dev. If hardening is needed later, switch back to Keychain with stable signing.
- Auto-refresh model list when the user changes endpoint/key (without clicking Connect). For MVP, manual Connect button is fine.

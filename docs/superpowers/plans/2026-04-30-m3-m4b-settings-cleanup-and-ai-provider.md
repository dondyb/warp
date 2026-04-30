# M3 + M4b — Strip Cloud Entries from Settings & Add "AI Provider" Tab

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make the Warp Settings panel feel like *this fork's* settings, not a hosted Warp's:

1. **Hide cloud-tied tabs** in the Settings sidebar when `ChannelState::is_cloud_enabled() == false`: `Referrals`, `BillingAndUsage`, `Teams`, `WarpDrive`. Plus hide cloud-tied widgets in remaining tabs (Sign up button on Account page, Upgrade CTAs on AI page).
2. **Add a new "AI Provider" tab** with form fields (endpoint URL, API key, model, protocol dropdown) and a "Test connection" button. Persist non-secret values to TOML; persist the API key to OS secure storage. Read these values into `OpenAiConfig` so users no longer need to set env vars.

**Architecture:** Add `SettingsSection::AiProvider` to the existing settings enum. Build a new page at `app/src/settings_view/ai_provider_page.rs` that follows the existing `Appearance` tab pattern (a `PageType` containing `SettingsWidget` trait objects). Persist via `define_settings_group!` macro for plain values; reuse the existing `ctx.secure_storage()` API (already used by `crates/ai/src/api_keys.rs`) under a fork-specific key like `"BringYourOwnLlmApiKey"`. Update `OpenAiConfig::from_env()` to also try `from_settings(ctx)` so the dispatcher can construct the adapter from either source.

For the cloud-strip: filter the nav list in `mod.rs:1186` based on `ChannelState::is_cloud_enabled()` (already added in M4a). Sub-page widgets that show cloud CTAs gain `if ChannelState::is_cloud_enabled() { ... }` guards.

**Tech Stack:** Rust 2021. Reuses existing UI primitives (`SubmittableTextInput`, `Dropdown`, `ButtonVariant`). No new external crates.

---

## Context

After M4a (already on this fork), `ChannelState::is_cloud_enabled()` returns `false` for the OSS channel — used today only by the boot path. This plan extends that gate to the Settings UI.

Per the codebase survey:

- **Settings enum:** `app/src/settings_view/mod.rs:188-225` — `pub enum SettingsSection { About, Account, MCPServers, BillingAndUsage, Appearance, Features, Keybindings, Privacy, Referrals, Teams, WarpDrive, ... }`.
- **Sidebar nav list:** `app/src/settings_view/mod.rs:1186-1215` — the `SettingsNavItem::Page(SettingsSection::X)` entries in order.
- **Cloud-tied tabs:** `Referrals`, `BillingAndUsage`, `Teams`, `WarpDrive` — entire pages are cloud-only.
- **Cloud-tied widgets within other tabs:** "Sign up" button on `main_page.rs:340-343`; "Upgrade" CTAs on `main_page.rs:126`, `teams_page.rs:2159-2163`, `ai_page.rs:6170-6208`.
- **Settings persistence:** TOML at `~/Library/Application Support/Warp/settings.toml`. The `define_settings_group!` macro generates the read/write API.
- **Secure storage:** `crates/warpui_extras/src/secure_storage` — `SecureStorage` trait. Existing usage at `crates/ai/src/api_keys.rs:7,176` (constant `SECURE_STORAGE_KEY`, `ctx.secure_storage().write_value/read_value`).
- **Form primitives:** `SubmittableTextInput`, `Dropdown`, `ButtonVariant` — all in `app/src/view_components/`.
- **Existing AI Settings page** at `app/src/settings_view/ai_page.rs` is 6k+ lines of Warp-cloud-AI specifics. **Don't surgery this** — leave it alone (M4a already hides it from the OSS boot path indirectly because the Account features it surfaces require auth). Eventually a future plan can hide the entire `ai_page.rs` tab when cloud is disabled, but it's not strictly necessary because Agent Mode now uses our `OpenAiAdapter`.

After this plan:

- The Settings sidebar shows only fork-relevant tabs (About, Appearance, Features, Keybindings, Privacy, **AI Provider**, MCPServers).
- A new "AI Provider" tab lets the user enter endpoint / key / model / protocol and persists them.
- `OpenAiConfig::from_settings(ctx)` returns the saved values (or `None` if the user hasn't configured yet).
- `OpenAiAdapter::from_env()` is renamed/extended so it tries env vars first, then settings — naming TBD during implementation.
- Other cloud surfaces (Sign up button, Upgrade CTAs in main/account/AI pages) are gated behind `ChannelState::is_cloud_enabled()`.

**Out of scope** (deferred):

- A real "Test connection" success indicator beyond a green checkmark / red error message — UI polish for later.
- Settings sync (cloud-stored settings) for the AI Provider config.
- The existing `ai_page.rs` (the Warp-hosted AI settings page) staying as-is — not stripped, just hidden from the nav. M4b-followup or later.
- Multi-provider config (one model per feature) — single global OpenAI provider only.
- Anthropic / other protocols — protocol dropdown is *present* but only OpenAI is wired up. Anthropic hooks into the same dropdown when M2 lands.

## File Structure

**Create:**

| Path | Responsibility |
|---|---|
| `app/src/settings_view/ai_provider_page.rs` | New `AiProviderPage` view + widgets (endpoint URL, API key, model, protocol dropdown, Test connection button). |
| `app/src/settings/ai_provider.rs` | New `define_settings_group!` for the non-secret AI Provider config (endpoint, model, protocol, supports_tools). |

**Modify:**

| Path | Change |
|---|---|
| `app/src/settings_view/mod.rs` | Add `SettingsSection::AiProvider` enum variant + Display label. Register page in nav list. **Filter the nav list** by `ChannelState::is_cloud_enabled()` to drop cloud-tied tabs. |
| `app/src/settings_view/main_page.rs` | Gate the "Sign up" button (line 340-343) and "Upgrade" handler (line 126) behind `ChannelState::is_cloud_enabled()`. |
| `app/src/settings/mod.rs` | Declare `pub mod ai_provider;`. |
| `crates/ai_provider/src/openai.rs` | Add `OpenAiConfig::from_settings(...)` (or `from_env_or_settings`). The function returns `Option<OpenAiConfig>` based on what's saved. The existing `from_env()` stays. |
| `app/src/server/server_api.rs` | The `Protocol::OpenAi` arm now tries `OpenAiAdapter::from_env_or_settings(ctx)` so settings-saved values flow through. |

---

## Tasks

### Task 1: Filter cloud-tied tabs from the Settings sidebar

**Files:**
- Modify: `app/src/settings_view/mod.rs`

- [ ] **Step 1: Read the nav-list construction**

Run: `sed -n '1180,1220p' /Users/dondy/Codes/warp/app/src/settings_view/mod.rs`. Find the block that pushes `SettingsNavItem::Page(...)` entries. The structure is roughly:

```rust
let mut items: Vec<SettingsNavItem> = vec![
    SettingsNavItem::Page(SettingsSection::Account),
    // ...
    SettingsNavItem::Page(SettingsSection::Referrals),
    SettingsNavItem::Page(SettingsSection::WarpDrive),
    // ...
];
```

(If structure differs, anchor on the `SettingsNavItem::Page(SettingsSection::Account)` line and read the surrounding ~40 lines.)

- [ ] **Step 2: Wrap each cloud-tied tab in `is_cloud_enabled()` guards**

Add `use warp_core::channel::ChannelState;` at the top of the file if not already imported. Then turn the static `vec![...]` into conditional pushes. The cleanest pattern:

```rust
let cloud = ChannelState::is_cloud_enabled();
let mut items: Vec<SettingsNavItem> = Vec::new();
items.push(SettingsNavItem::Page(SettingsSection::About));
items.push(SettingsNavItem::Page(SettingsSection::Account));   // we'll gate widgets inside, not the whole tab
items.push(SettingsNavItem::Page(SettingsSection::AiProvider)); // NEW — added in Task 4
items.push(SettingsNavItem::Page(SettingsSection::MCPServers));
items.push(SettingsNavItem::Page(SettingsSection::Appearance));
items.push(SettingsNavItem::Page(SettingsSection::Features));
items.push(SettingsNavItem::Page(SettingsSection::Keybindings));
items.push(SettingsNavItem::Page(SettingsSection::Privacy));
if cloud {
    items.push(SettingsNavItem::Page(SettingsSection::BillingAndUsage));
    items.push(SettingsNavItem::Page(SettingsSection::Referrals));
    items.push(SettingsNavItem::Page(SettingsSection::Teams));
    items.push(SettingsNavItem::Page(SettingsSection::WarpDrive));
}
// ... whatever else is in the original list
```

> **Note:** This task introduces a forward reference to `SettingsSection::AiProvider` which Task 4 adds. To avoid a compile error at this commit, **either** stage the order — do Tasks 4 (enum variant) before Task 1 (nav filtering) — **or** comment out the AiProvider line for now and add it in Task 4. The plan text below assumes Task 4 lands the enum variant *before* this task uses it; if you implement out of order, swap accordingly.

> **Better order:** do Task 4 first (enum variant + Display impl), then Task 1 (nav filtering). The plan is written linearly but the implementer should flip these two if it simplifies compilation.

- [ ] **Step 3: Verify the workspace builds**

Run: `cargo check -p warp 2>&1 | tail -10`
Expected: PASS.

- [ ] **Step 4: Commit**

```bash
cd /Users/dondy/Codes/warp
git add app/src/settings_view/mod.rs
git commit -m "feat(settings): hide cloud-tied tabs when cloud disabled"
```

---

### Task 2: Gate cloud widgets in the Account page

**Files:**
- Modify: `app/src/settings_view/main_page.rs`

- [ ] **Step 1: Find the cloud-tied widgets**

Run:
```bash
rg -n "Sign up|MainPageAction::Upgrade|render_anonymous_account_info" /Users/dondy/Codes/warp/app/src/settings_view/main_page.rs | head -10
```

Expected hits:
- "Sign up" button label (~line 340)
- `MainPageAction::Upgrade` handler (~line 126)
- `render_anonymous_account_info` function (~line 318)

- [ ] **Step 2: Wrap the "Sign up" button render**

Find the `render_anonymous_account_info` function (or similar). It returns a UI element with a "Sign up" button for anonymous users. Wrap the button construction so it only appears when `ChannelState::is_cloud_enabled()`:

```rust
if ChannelState::is_cloud_enabled() {
    // existing button construction
}
```

If the function unconditionally returns the button, refactor so the button is `Option<...>` or wrap the entire panel in a cloud-gate. Path of least resistance: add a single `if` around the button construction.

- [ ] **Step 3: Make the `MainPageAction::Upgrade` handler a no-op when cloud disabled**

Find the `match` on `MainPageAction::Upgrade { team_uid, user_id } => { ... }` (~line 126). Wrap the handler body:

```rust
MainPageAction::Upgrade { team_uid, user_id } => {
    if !ChannelState::is_cloud_enabled() {
        return; // no-op in OSS fork
    }
    // existing handler body
}
```

The button that *triggers* `Upgrade` may still render. To suppress it in the UI, find the render site (search `MainPageAction::Upgrade` in the same file or in `render_account_info`) and gate it the same way as Step 2.

- [ ] **Step 4: Verify the workspace builds**

Run: `cargo check -p warp 2>&1 | tail -5`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add app/src/settings_view/main_page.rs
git commit -m "feat(settings): gate Account-page cloud widgets behind cloud_enabled"
```

---

### Task 3: Gate Upgrade CTAs on the AI page

**Files:**
- Modify: `app/src/settings_view/ai_page.rs`

- [ ] **Step 1: Find the Upgrade CTAs**

Run: `rg -n "Upgrade|upgrade_plan" /Users/dondy/Codes/warp/app/src/settings_view/ai_page.rs | head -10`. Expected hits around lines 6170-6208 (per survey). Read those lines to understand the surrounding render code.

- [ ] **Step 2: Wrap each Upgrade CTA**

Add `use warp_core::channel::ChannelState;` if not already imported. Around each Upgrade CTA render block, wrap with `if ChannelState::is_cloud_enabled() { ... }`. If the CTAs are constructed in helper functions (e.g., `render_upgrade_cta()`), gate the call sites rather than the helper itself.

- [ ] **Step 3: Verify build**

Run: `cargo check -p warp 2>&1 | tail -5`
Expected: PASS.

- [ ] **Step 4: Commit**

```bash
git add app/src/settings_view/ai_page.rs
git commit -m "feat(settings): hide AI page Upgrade CTAs when cloud disabled"
```

---

### Task 4: Add `SettingsSection::AiProvider` enum variant + Display label

**Files:**
- Modify: `app/src/settings_view/mod.rs`

- [ ] **Step 1: Add the variant to the enum**

Find the `pub enum SettingsSection` (around line 188). Add a new variant `AiProvider` alphabetically (after `Account`, before `Appearance`):

```rust
pub enum SettingsSection {
    About,
    #[default]
    Account,
    AiProvider,  // NEW
    Appearance,
    BillingAndUsage,
    Features,
    Keybindings,
    MCPServers,
    Privacy,
    Referrals,
    Teams,
    WarpDrive,
    // ... existing variants
}
```

- [ ] **Step 2: Add the Display impl arm**

Find `impl Display for SettingsSection` (around line 230) and add:

```rust
SettingsSection::AiProvider => write!(f, "AI Provider"),
```

- [ ] **Step 3: Verify compile**

Run: `cargo check -p warp 2>&1 | tail -10`
Expected: warnings about non-exhaustive matches OR a compile error if there are any `match self { ... }` blocks elsewhere that don't have a wildcard. If errors fire, the offending matches need to add `SettingsSection::AiProvider => { /* fall through */ }` arms, OR the matches that determine routing need the new variant routed somewhere — but Tasks 5 and 6 establish the page so the routing-match update happens there.

If errors are routing-related (e.g., `match section { SettingsSection::Account => Page::Account, ... }` is non-exhaustive), add a placeholder `SettingsSection::AiProvider => unimplemented!("populated in Task 5")` and Task 5 replaces it.

If errors are unrelated (e.g., other modules pattern-match `SettingsSection`), audit and add the appropriate arms.

- [ ] **Step 4: Commit**

```bash
git add app/src/settings_view/mod.rs
git commit -m "feat(settings): add SettingsSection::AiProvider enum variant"
```

---

### Task 5: Create `ai_provider_page.rs` skeleton

**Files:**
- Create: `app/src/settings_view/ai_provider_page.rs`
- Modify: `app/src/settings_view/mod.rs` (declare module + route the variant)

- [ ] **Step 1: Read an existing page to model the structure**

Open `app/src/settings_view/appearance_page.rs` for reference. Note the structure:

- A `pub struct AppearancePage { /* widgets */ }`
- Implements `SettingsPageMeta` (or whatever the trait is)
- A `render` method or `widgets()` method that produces the page content

Run: `head -100 /Users/dondy/Codes/warp/app/src/settings_view/appearance_page.rs` to see the imports + struct layout.

- [ ] **Step 2: Create `ai_provider_page.rs` with a stub**

Create the file with a minimal implementation that compiles and shows a "Coming soon" message:

```rust
//! Settings page for configuring a custom AI provider (OpenAI- or
//! Anthropic-compatible endpoint). Replaces the env-var-only configuration
//! path established in M1b-chat.
//!
//! M3 MVP: form fields for endpoint, API key, model, protocol — persisted
//! to disk (TOML for plain values, OS secure storage for the API key).

// Imports — match the pattern from appearance_page.rs.
// Replace these with whatever appearance_page.rs uses; this file should
// follow the same convention.

use crate::settings_view::settings_page::{PageType, SettingsPageMeta};

pub struct AiProviderPage {
    // widgets added in Task 6
}

impl AiProviderPage {
    pub fn new() -> Self {
        Self {}
    }
}

impl SettingsPageMeta for AiProviderPage {
    // implement whatever the trait requires — copy the shape from
    // AppearancePage and stub-return empty content for now.
}
```

> **Imports + trait shape:** the exact import paths and trait method signatures depend on Warp's settings infrastructure. Copy the imports and trait skeleton from `appearance_page.rs` (or another existing page) verbatim, replacing only the body.

- [ ] **Step 3: Declare the module and route the variant**

In `app/src/settings_view/mod.rs`:

1. Add `pub mod ai_provider_page;` near the other `pub mod *_page;` declarations.
2. Wherever `SettingsSection` is converted to a `PageType` (search for `match` blocks that produce `PageType` from `SettingsSection`), replace any `unimplemented!()` from Task 4 with the constructed `AiProviderPage`. Pattern likely looks like:
   ```rust
   match section {
       SettingsSection::Appearance => PageType::Appearance(...),
       SettingsSection::AiProvider => PageType::AiProvider(AiProviderPage::new()),
       // ...
   }
   ```
3. Also add a `PageType::AiProvider(AiProviderPage)` variant to the `PageType` enum if that's the pattern.

The exact wiring depends on how the existing pages are routed. Mirror the Appearance page's wiring step-for-step.

- [ ] **Step 4: Verify compile**

Run: `cargo check -p warp 2>&1 | tail -10`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add app/src/settings_view/ai_provider_page.rs app/src/settings_view/mod.rs
git commit -m "feat(settings): scaffold AI Provider page"
```

---

### Task 6: Add form widgets — endpoint, API key, model, protocol dropdown

**Files:**
- Modify: `app/src/settings_view/ai_provider_page.rs`

- [ ] **Step 1: Identify the widget primitives to use**

Read existing pages for examples:
- `submittable_text_input.rs` for text fields with validation
- `dropdown.rs` for the protocol selector
- `appearance_page.rs` for how a page wires multiple widgets together

Open `appearance_page.rs` and look for places it uses `SubmittableTextInput` and `Dropdown` — copy that pattern.

- [ ] **Step 2: Define the page's widget fields**

Replace `AiProviderPage` with:

```rust
use crate::view_components::dropdown::Dropdown;
use crate::view_components::submittable_text_input::SubmittableTextInput;
// (adjust imports per actual paths)

pub struct AiProviderPage {
    endpoint_input: ViewHandle<SubmittableTextInput>,
    api_key_input: ViewHandle<SubmittableTextInput>,
    model_input: ViewHandle<SubmittableTextInput>,
    protocol_dropdown: ViewHandle<Dropdown<ProtocolDropdownAction>>,
    test_connection_button: MouseStateHandle,
}
```

> **`ProtocolDropdownAction`** is a placeholder name — define a small enum:
>
> ```rust
> #[derive(Clone, Action)]
> pub enum ProtocolDropdownAction {
>     SelectOpenAi,
>     SelectAnthropic,
> }
> ```
>
> Match the existing Action-derive pattern from another page.

- [ ] **Step 3: Implement `render()` — the form layout**

Inside the page's render function, lay out the four fields + button vertically. Use the appearance_page's layout helpers (`column!`, `row!`, etc.) for consistency. Each field gets a label.

For the **API key** field, use the API-key input pattern from `ai_page.rs:6110-6145` (`render_api_key_input` function). It currently does NOT mask the input as `•••` — the survey flagged this. For MVP, leave it unmasked (visible plaintext while typing). A future polish task can add masking.

The "Test connection" button is just a `Button` with variant `Secondary` (or `Accent`) and a `MouseStateHandle`. Wire its click handler in Task 9.

- [ ] **Step 4: Verify compile**

Run: `cargo check -p warp 2>&1 | tail -10`
Expected: PASS.

- [ ] **Step 5: Visual smoke (optional but recommended)**

Run: `./script/run`. Open Settings → AI Provider. Confirm the form appears with the four fields + button. Don't test functionality yet (persistence is Task 7) — just verify the layout looks reasonable.

If the layout is off, iterate on the render code. Don't worry about pixel-perfect — the goal is functional form fields visible.

- [ ] **Step 6: Commit**

```bash
git add app/src/settings_view/ai_provider_page.rs
git commit -m "feat(settings): add AI Provider form fields"
```

---

### Task 7: Persist non-secret values to settings TOML

**Files:**
- Create: `app/src/settings/ai_provider.rs`
- Modify: `app/src/settings/mod.rs` (declare the module)
- Modify: `app/src/settings_view/ai_provider_page.rs` (wire save/load)

- [ ] **Step 1: Read an existing settings group for the pattern**

Open `app/src/settings/theme.rs` to see how `define_settings_group!` is used. Note:
- The macro takes a struct name (`ThemeSettings`)
- Each setting has `type`, `default`, `storage_key`, `toml_path`
- Reading: `ThemeSettings::theme_kind.value(ctx)` (or similar)
- Writing: `ThemeSettings::theme_kind.set_value(new_value, ctx)`

- [ ] **Step 2: Create `app/src/settings/ai_provider.rs`**

Follow the theme.rs pattern. Define:

```rust
use settings::define_settings_group;

define_settings_group!(AiProviderSettings, settings: [
    endpoint: AiProviderEndpoint {
        type: String,
        default: "https://api.openai.com/v1".to_string(),
        storage_key: "AiProviderEndpoint",
        toml_path: "ai_provider.endpoint",
    },
    model: AiProviderModel {
        type: String,
        default: "".to_string(),
        storage_key: "AiProviderModel",
        toml_path: "ai_provider.model",
    },
    protocol: AiProviderProtocol {
        type: String,
        default: "openai".to_string(),
        storage_key: "AiProviderProtocol",
        toml_path: "ai_provider.protocol",
    },
]);
```

> The exact macro syntax may differ — match what `theme.rs` actually uses. Common variations: types are enum vs string, the macro wants a `serde::Serialize + serde::Deserialize` bound, etc.

> **Don't include the API key here** — it goes to secure storage (Task 8).

- [ ] **Step 3: Declare the module**

In `app/src/settings/mod.rs`, add `pub mod ai_provider;` near the other `pub mod *;` declarations.

- [ ] **Step 4: Wire save/load in `ai_provider_page.rs`**

In the page's render flow:

- On page load, read the saved values via `AiProviderSettings::endpoint.value(ctx)` and populate the input fields.
- On field-edit (or "Save" button if your page has one), write back via `AiProviderSettings::endpoint.set_value(new_value, ctx)`.

If your page uses an "auto-save on change" pattern (like Appearance does), wire the input's on-change handler to save immediately. If it uses a "Save" button, add one.

- [ ] **Step 5: Verify compile**

Run: `cargo check -p warp 2>&1 | tail -5`
Expected: PASS.

- [ ] **Step 6: Manual round-trip smoke**

Run `./script/run`. Open Settings → AI Provider. Type a value in the endpoint field. Quit. Re-launch. Confirm the value persists.

- [ ] **Step 7: Commit**

```bash
git add app/src/settings/ ai_provider_page.rs app/src/settings_view/ai_provider_page.rs
git commit -m "feat(settings): persist AI Provider non-secret config to TOML"
```

---

### Task 8: Persist API key to OS secure storage

**Files:**
- Modify: `app/src/settings_view/ai_provider_page.rs`

- [ ] **Step 1: Read the existing pattern**

Open `crates/ai/src/api_keys.rs:7,176-196`. The existing pattern:

```rust
const SECURE_STORAGE_KEY: &str = "AiApiKeys";

fn load_keys_from_secure_storage(ctx: &mut ModelContext<Self>) -> ApiKeys {
    let key_json = match ctx.secure_storage().read_value(SECURE_STORAGE_KEY) {
        Ok(json) => json,
        Err(e) => { /* log and return default */ }
    };
    serde_json::from_str(&key_json).unwrap_or_default()
}
```

- [ ] **Step 2: Add API-key save/load to the page**

In `ai_provider_page.rs`, define a fork-specific storage key (different from the existing `"AiApiKeys"` so we don't collide):

```rust
const API_KEY_STORAGE_KEY: &str = "BringYourOwnLlmApiKey";

fn load_api_key_from_secure_storage(ctx: &impl AppContextExt) -> Option<String> {
    ctx.secure_storage().read_value(API_KEY_STORAGE_KEY).ok()
}

fn save_api_key_to_secure_storage(ctx: &impl AppContextExt, key: &str) {
    if let Err(e) = ctx.secure_storage().write_value(API_KEY_STORAGE_KEY, key) {
        log::warn!("failed to save AI provider API key: {e:#}");
    }
}
```

> **`AppContextExt` trait** — the existing pattern in `api_keys.rs:176` uses `ctx.secure_storage()`. Find the trait this method comes from and import it: `rg -n "trait AppContextExt|fn secure_storage" /Users/dondy/Codes/warp/crates/warpui_extras/src/secure_storage/`.

- [ ] **Step 3: Wire the API-key input field**

On page load, populate the API key input from `load_api_key_from_secure_storage`. On field change/submit, call `save_api_key_to_secure_storage`. Use the same on-change pattern Task 7 used for the non-secret fields.

> **Display behavior:** when a key is loaded from storage, the UI can show the actual value or a placeholder (e.g., `"●●●● ●●●● ●●●●"`). MVP: just show the actual value (matches the existing Warp pattern in `ai_page.rs`).

- [ ] **Step 4: Verify compile**

Run: `cargo check -p warp 2>&1 | tail -5`
Expected: PASS.

- [ ] **Step 5: Manual round-trip smoke**

Run `./script/run`. Open Settings → AI Provider. Type an API key. Quit. Re-launch. Confirm the key persists.

- [ ] **Step 6: Commit**

```bash
git add app/src/settings_view/ai_provider_page.rs
git commit -m "feat(settings): persist AI Provider API key to secure storage"
```

---

### Task 9: Wire `OpenAiConfig::from_settings(ctx)` and the Test connection button

**Files:**
- Modify: `crates/ai_provider/src/openai.rs` (add `from_settings` constructor)
- Modify: `app/src/server/server_api.rs` (use `from_env_or_settings`)
- Modify: `app/src/settings_view/ai_provider_page.rs` (wire Test connection button)

- [ ] **Step 1: Add `from_settings` to `OpenAiConfig`**

Note: `crates/ai_provider/` does NOT depend on `app/`, so it cannot directly import `app/src/settings/ai_provider.rs`. Instead, add a constructor that takes an explicit struct:

```rust
impl OpenAiConfig {
    pub fn from_parts(
        endpoint: String,
        api_key: String,
        model: String,
    ) -> std::result::Result<Self, Arc<AIApiError>> {
        if api_key.trim().is_empty() {
            return Err(Arc::new(AIApiError::Other(anyhow::anyhow!(
                "API key is required"
            ))));
        }
        if model.trim().is_empty() {
            return Err(Arc::new(AIApiError::Other(anyhow::anyhow!(
                "Model is required"
            ))));
        }
        let endpoint = if endpoint.is_empty() {
            Self::DEFAULT_ENDPOINT.to_string()
        } else {
            endpoint
        };
        Ok(Self { endpoint, api_key, model })
    }
}
```

- [ ] **Step 2: Add a settings-aware constructor to `OpenAiAdapter`**

In the `app` crate (e.g., a new method on `ServerApi`), add a function that reads settings + secure storage and builds `OpenAiConfig`:

```rust
fn openai_config_from_settings_or_env(ctx: &AppContext) -> Result<OpenAiConfig, Arc<AIApiError>> {
    // Env vars take precedence (allows ad-hoc override).
    if let Ok(cfg) = OpenAiConfig::from_env() {
        return Ok(cfg);
    }
    // Otherwise read from settings + secure storage.
    let endpoint = AiProviderSettings::endpoint.value(ctx);
    let model = AiProviderSettings::model.value(ctx);
    let api_key = ctx.secure_storage().read_value(API_KEY_STORAGE_KEY).unwrap_or_default();
    OpenAiConfig::from_parts(endpoint, api_key, model)
}
```

- [ ] **Step 3: Update the dispatcher**

In `server_api.rs`, the `Protocol::OpenAi` arm currently calls `OpenAiAdapter::from_env()`. Change it to:

```rust
Protocol::OpenAi => {
    let config = openai_config_from_settings_or_env(ctx)?;
    let adapter = OpenAiAdapter::new(config);
    AiProvider::chat_stream(&adapter, request).await
}
```

- [ ] **Step 4: Wire the Test connection button**

In `ai_provider_page.rs`, click handler for the Test button:

1. Read current input values from the four input widgets.
2. Build an `OpenAiConfig::from_parts(endpoint, api_key, model)?`.
3. Instantiate `OpenAiAdapter::new(config)`.
4. Send a one-shot test chat: a `Request` with input `"hello"`. The first stream event should arrive within 5 seconds.
5. Update a status indicator next to the button: ✓ "Connection OK" or ✗ "<error message>".

For MVP, the status indicator can just be a `Text` element that updates. Don't worry about animations or polish.

- [ ] **Step 5: Verify build**

Run: `cargo check --workspace 2>&1 | tail -10`
Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add crates/ai_provider/src/openai.rs app/src/server/server_api.rs app/src/settings_view/ai_provider_page.rs
git commit -m "feat(settings): wire AI Provider settings to OpenAiConfig + Test connection"
```

---

### Task 10: Final verification

**Files:** none

- [ ] **Step 1: Clippy**

```bash
cargo clippy -p ai_provider -p warp --tests --all-targets -- -D warnings 2>&1 | tail -10
```
Expected: PASS.

- [ ] **Step 2: Nextest**

```bash
cargo nextest run -p ai_provider -p warp --no-fail-fast 2>&1 | tail -15
```
Expected: no NEW failures vs. M1b-chat baseline.

- [ ] **Step 3: Manual GUI smoke** — coordinated with the user

Run: `./script/run`

Verify:
1. Settings sidebar shows: About, Account, **AI Provider** (new), Appearance, Features, Keybindings, MCPServers, Privacy. **Does NOT show:** Referrals, Billing, Teams, Warp Drive.
2. Account page does NOT show "Sign up" button.
3. Settings → AI Provider page shows form with endpoint, API key, model, protocol fields + Test connection button.
4. Save a config (endpoint=`https://api.openai.com/v1`, key=`sk-...`, model=`gpt-4o-mini`).
5. Click Test connection. Status indicator shows ✓ or ✗.
6. Open Agent Mode, send a prompt: "what is 2+2?". Confirm a streaming response from the configured endpoint (no env var needed).
7. Quit, relaunch. Confirm config persists.

---

## Self-Review Checklist (run before declaring this plan done)

- [ ] Settings sidebar hides cloud-tied tabs when `is_cloud_enabled() == false`.
- [ ] Account page hides "Sign up" button.
- [ ] AI page hides Upgrade CTAs.
- [ ] New "AI Provider" tab appears with 4 form fields + Test connection button.
- [ ] Non-secret values persist to `settings.toml`.
- [ ] API key persists to OS secure storage under `BringYourOwnLlmApiKey`.
- [ ] `OpenAiAdapter` reads config via `openai_config_from_settings_or_env(ctx)` (env vars > settings > error).
- [ ] Test connection button fires a real chat completion and shows ✓/✗.
- [ ] Manual smoke shows end-to-end: save config → send Agent prompt → response from custom endpoint.
- [ ] Clippy passes with `-D warnings`.
- [ ] No new test failures.

## Out of scope (deferred to future plans)

- **Hide the existing `ai_page.rs` tab entirely.** It's still in the sidebar (under the AI section) — for now, only its Upgrade CTAs are gated. Stripping it fully needs careful work since it has bundled-skill UI that may still be useful.
- **Multi-provider config** (one model per feature).
- **Anthropic protocol** (M2 lands the actual translation; the dropdown UX shows it as "coming soon" or disabled).
- **API key masking** (`●●●●●`) — UI polish for later.
- **Settings sync to cloud** for the AI Provider config — not relevant for the OSS fork.
- **Tool calling** (M1c) — still parked.

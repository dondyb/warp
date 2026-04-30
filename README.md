# WarpOss — fork with bring-your-own LLM endpoint

This is a fork of [warpdotdev/warp](https://github.com/warpdotdev/warp) that strips out the warp.dev account, login wizard, and cloud surfaces, and replaces them with a Settings page where you point Warp at any OpenAI- or Anthropic-compatible LLM endpoint (LiteLLM, OpenRouter, Ollama, vLLM, your own gateway — anything that speaks `/v1/chat/completions`).

It runs as the `Channel::Oss` build (`WarpOss.app`) and ships with full agentic tool use (18 client-side tools — shell, file edits, grep, ripgrep, MCP, etc.) wired through the same Warp protobuf transaction protocol the upstream client uses.

## What's different from upstream

- **No login required.** OSS channel boots straight to a terminal — no onboarding wizard, no warp.dev account.
- **AI Provider settings tab.** Configure endpoint URL, API key, model, and protocol (OpenAI or Anthropic) in Settings → AI Provider. Click **Connect** to fetch the endpoint's `/v1/models` list and pick a model from the dropdown.
- **API key stored in macOS UserDefaults.** Lives in `~/Library/Preferences/dev.warp.WarpOss.plist` (mode 0600), keyed by `AiProviderApiKey`. No more macOS Keychain re-prompt on every rebuild — UserDefaults is gated by bundle ID, not code signature.
- **Cloud surfaces hidden in OSS mode.** Account page, billing, teams, referrals, Warp Drive, the avatar/user-menu pulldown, the per-profile Base/Full-terminal model dropdowns, the inline `/MODEL` picker, and the Resource Center lightbulb are all gated behind `ChannelState::is_cloud_enabled()`.
- **Single global model.** One model per Warp instance, set in AI Provider settings.

## Build & run

### Prerequisites (macOS)

- Xcode Command Line Tools
- Rust toolchain (rustup) — installs the version pinned in `rust-toolchain.toml` automatically
- Node + yarn (via corepack) — `corepack enable`

### Build the OSS app

```bash
cd app
cargo bundle --bin warp-oss
```

The bundled app lands at:

```
target/debug/bundle/osx/WarpOss.app
```

For a release build:

```bash
cd app
cargo bundle --bin warp-oss --release
```

### Launch

```bash
open target/debug/bundle/osx/Woz.app
```

On first launch macOS may show a Gatekeeper prompt because the binary is ad-hoc signed. Right-click → Open if needed.

### Configure your LLM endpoint

1. Open Settings (gear icon in the top tab bar).
2. Go to **AI Provider**.
3. Fill in:
   - **Endpoint URL** — e.g. `https://api.openai.com/v1`, `http://localhost:11434/v1` (Ollama), `http://localhost:4000/v1` (LiteLLM), etc.
   - **API key**.
   - **Protocol** — OpenAI for now (Anthropic native is stubbed).
4. Click **Connect**. The Model dropdown populates with whatever the endpoint returns from `/v1/models`. Pick one.
5. Open a terminal and try `/agent`.

### Where the AI Provider settings live

Settings are split between two stores depending on how `private` they are:

| Setting    | Store                                                    | Why                                            |
|------------|----------------------------------------------------------|------------------------------------------------|
| `endpoint` | `~/.warp-oss/settings.toml` `[ai_provider]`              | Public TOML, hand-editable                     |
| `model`    | `~/.warp-oss/settings.toml` `[ai_provider]`              | Public TOML, hand-editable                     |
| `protocol` | `~/.warp-oss/settings.toml` `[ai_provider]`              | Public TOML, hand-editable                     |
| `api_key`  | `~/Library/Preferences/dev.warp.WarpOss.plist` (`AiProviderApiKey`) | macOS UserDefaults; mode 0600. Read with `defaults read dev.warp.WarpOss AiProviderApiKey` |

UserDefaults is gated by bundle ID (`dev.warp.WarpOss`), not by code signature, so you don't get re-prompted across rebuilds the way Keychain would.

## Tests & lints

From the repo root:

```bash
cargo clippy -p ai_provider -p warp --tests --all-targets -- -D warnings
cargo nextest run -p ai_provider --no-fail-fast
```

The `ai_provider` crate has integration tests (mockito-backed) covering the OpenAI adapter happy path, error paths, and tool-call round-trip.

## Repository layout (additions specific to this fork)

```
crates/ai_provider/        OpenAI adapter, tool definitions, dispatcher glue
app/src/settings/ai_provider.rs       Settings group (endpoint/api_key/model/protocol)
app/src/settings_view/ai_provider_page.rs   Settings UI
docs/superpowers/specs/    Design docs for the fork
docs/superpowers/plans/    Implementation plans (M1a → M1c → polish-1)
```

## Upstream sync

This fork pins to a snapshot of `warpdotdev/warp`. To pull upstream changes manually:

```bash
git fetch upstream
git merge upstream/master   # resolve any conflicts
```

Push to upstream is disabled on this clone (`upstream` push URL is `DISABLE`).

## Licensing

Inherited from upstream:

- Warp's UI framework (`warpui_core` and `warpui` crates) is [MIT](LICENSE-MIT).
- The rest of the code is [AGPL v3](LICENSE-AGPL).

This fork's additions are released under the same terms as the file they live in.

## Acknowledgements

All credit for the terminal and the agentic block UI goes to [the Warp team](https://github.com/warpdotdev/warp). This fork only swaps out the LLM provider plumbing and trims the cloud-tied UI; everything else is upstream's work.

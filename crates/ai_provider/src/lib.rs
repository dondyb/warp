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

pub mod client;
pub mod error;
pub mod openai;

pub use client::{AiProvider, ResponseEventStream};
pub use error::{
    AIApiError, DeserializationError, WARP_ERROR_CODE_HEADER, WARP_ERROR_CODE_OUT_OF_CREDITS,
};
pub use openai::OpenAiConfig;

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

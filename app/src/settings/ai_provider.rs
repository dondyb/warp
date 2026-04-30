//! Settings for the custom AI Provider — endpoint URL, model, protocol.
//! API key is stored in OS secure storage, not in TOML.

use settings::{macros::define_settings_group, SupportedPlatforms, SyncToCloud};

define_settings_group!(AiProviderSettings, settings: [
    endpoint: AiProviderEndpoint {
        type: String,
        default: "https://api.openai.com/v1".to_string(),
        supported_platforms: SupportedPlatforms::ALL,
        sync_to_cloud: SyncToCloud::Never,
        private: false,
        storage_key: "AiProviderEndpoint",
        toml_path: "ai_provider.endpoint",
        description: "The endpoint URL for the custom AI provider.",
    },
    model: AiProviderModel {
        type: String,
        default: "".to_string(),
        supported_platforms: SupportedPlatforms::ALL,
        sync_to_cloud: SyncToCloud::Never,
        private: false,
        storage_key: "AiProviderModel",
        toml_path: "ai_provider.model",
        description: "The model name for the custom AI provider.",
    },
    protocol: AiProviderProtocol {
        type: String,
        default: "openai".to_string(),
        supported_platforms: SupportedPlatforms::ALL,
        sync_to_cloud: SyncToCloud::Never,
        private: false,
        storage_key: "AiProviderProtocol",
        toml_path: "ai_provider.protocol",
        description: "The protocol for the custom AI provider (openai or anthropic).",
    },
]);

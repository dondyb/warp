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

    /// True iff this tool corresponds to the given proto `ToolCall::Tool` variant
    /// (used when reconstructing the assistant's previous tool_calls from history).
    fn matches_proto_call(&self, tool: &message::tool_call::Tool) -> bool;

    /// True iff this tool corresponds to the given proto `ToolCallResult::Result`
    /// variant from `Request.input` (used when translating incoming results to
    /// OpenAI `role: tool` messages).
    fn matches_proto_result(&self, result: &input::tool_call_result::Result) -> bool;

    /// Encode the proto `ToolCall::Tool` variant back into the JSON args form
    /// used when reconstructing the assistant's previous tool_calls in
    /// multi-turn requests. Default: return empty object (sufficient until
    /// Phase B's concrete impls override it).
    fn encode_call_args(&self, _tool: &message::tool_call::Tool) -> Value {
        Value::Object(serde_json::Map::new())
    }
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
                &edit_write::APPLY_FILE_DIFFS,
                &edit_write::EDIT_DOCUMENTS,
                &edit_write::CREATE_DOCUMENTS,
                &edit_write::WRITE_TO_LONG_RUNNING_SHELL_COMMAND,
                &misc::SEARCH_CODEBASE,
                &misc::CALL_MCP_TOOL,
                &misc::USE_COMPUTER,
                &misc::REQUEST_COMPUTER_USE,
                &misc::INSERT_REVIEW_COMMENTS,
                &misc::UPLOAD_FILE_ARTIFACT,
            ],
        }
    }

    /// Look up a tool by its OpenAI function name.
    pub fn by_name(&self, name: &str) -> Option<&'static dyn ToolDefinition> {
        self.tools.iter().find(|t| t.name() == name).copied()
    }

    /// Look up a tool by matching its proto `ToolCall::Tool` variant.
    /// Returns `None` when no concrete tool is registered (e.g. Phase A).
    pub fn tool_for_proto(
        &self,
        tool: &message::tool_call::Tool,
    ) -> Option<&'static dyn ToolDefinition> {
        self.tools.iter().find(|t| t.matches_proto_call(tool)).copied()
    }

    /// Look up a tool by matching its proto `ToolCallResult` result variant
    /// from `Request.input`. Returns `None` when no concrete tool is registered.
    pub fn tool_for_proto_result(
        &self,
        result: &input::tool_call_result::Result,
    ) -> Option<&'static dyn ToolDefinition> {
        self.tools.iter().find(|t| t.matches_proto_result(result)).copied()
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
pub mod run_shell_command;
pub mod read_search;
pub mod edit_write;
pub mod misc;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_emits_tools_array() {
        let reg = ToolRegistry::new();
        let tools = reg.openai_tools_json();
        let arr = tools.as_array().unwrap();
        assert!(!arr.is_empty());
        assert!(arr.iter().any(|t| t["function"]["name"] == "run_shell_command"));
    }

    #[test]
    fn registry_lookup_misses_unknown() {
        let reg = ToolRegistry::new();
        assert!(reg.by_name("anything_unknown").is_none());
    }

    #[test]
    fn registry_lookup_hits_run_shell_command() {
        let reg = ToolRegistry::new();
        assert!(reg.by_name("run_shell_command").is_some());
    }
}

//! Miscellaneous tools: SearchCodebase, CallMCPTool, UseComputer,
//! RequestComputerUse, InsertReviewComments, UploadFileArtifact.

use serde_json::{json, Value};
use std::sync::Arc;
use warp_multi_agent_api::message;
use warp_multi_agent_api::request::input;

use crate::tools::ToolDefinition;
use crate::AIApiError;

pub static SEARCH_CODEBASE: SearchCodebaseTool = SearchCodebaseTool;
pub static CALL_MCP_TOOL: CallMcpToolTool = CallMcpToolTool;
pub static USE_COMPUTER: UseComputerTool = UseComputerTool;
pub static REQUEST_COMPUTER_USE: RequestComputerUseTool = RequestComputerUseTool;
pub static INSERT_REVIEW_COMMENTS: InsertReviewCommentsTool = InsertReviewCommentsTool;
pub static UPLOAD_FILE_ARTIFACT: UploadFileArtifactTool = UploadFileArtifactTool;

// ---------------------------------------------------------------------------
// 9a. SearchCodebase
// ---------------------------------------------------------------------------

pub struct SearchCodebaseTool;

impl ToolDefinition for SearchCodebaseTool {
    fn name(&self) -> &'static str {
        "search_codebase"
    }

    fn description(&self) -> &'static str {
        "Semantic search across the user's indexed codebase."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "description": "The natural-language or keyword query to search for."
                },
                "path_filters": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Optional list of path prefixes to restrict the search."
                },
                "codebase_path": {
                    "type": "string",
                    "description": "Absolute path to the codebase root. Defaults to the user's current directory."
                }
            },
            "required": ["query"]
        })
    }

    fn decode_call_args(
        &self,
        args: Value,
    ) -> std::result::Result<message::tool_call::Tool, Arc<AIApiError>> {
        let query = args
            .get("query")
            .and_then(|v| v.as_str())
            .ok_or_else(|| {
                Arc::new(AIApiError::Other(anyhow::anyhow!(
                    "search_codebase: missing required `query` argument"
                )))
            })?
            .to_string();

        let path_filters: Vec<String> = args
            .get("path_filters")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default();

        let codebase_path = args
            .get("codebase_path")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        Ok(message::tool_call::Tool::SearchCodebase(
            message::tool_call::SearchCodebase {
                query,
                path_filters,
                codebase_path,
            },
        ))
    }

    fn encode_call_args(&self, tool: &message::tool_call::Tool) -> Value {
        if let message::tool_call::Tool::SearchCodebase(sc) = tool {
            json!({
                "query": sc.query,
                "path_filters": sc.path_filters,
                "codebase_path": sc.codebase_path,
            })
        } else {
            Value::Object(Default::default())
        }
    }

    fn encode_result_text(
        &self,
        result: &input::tool_call_result::Result,
    ) -> std::result::Result<String, Arc<AIApiError>> {
        if let input::tool_call_result::Result::SearchCodebase(sc_result) = result {
            use warp_multi_agent_api::search_codebase_result::Result as Inner;
            match sc_result.result.as_ref() {
                Some(Inner::Success(success)) => {
                    let snippets: Vec<String> = success
                        .files
                        .iter()
                        .map(|f| {
                            let range = f
                                .line_range
                                .as_ref()
                                .map(|lr| format!(":{}-{}", lr.start, lr.end))
                                .unwrap_or_default();
                            format!("{}{}:\n{}", f.file_path, range, f.content)
                        })
                        .collect();
                    Ok(snippets.join("\n---\n"))
                }
                Some(Inner::Error(e)) => Ok(format!("error: {}", e.message)),
                None => Ok("(no result)".to_string()),
            }
        } else {
            Err(Arc::new(AIApiError::Other(anyhow::anyhow!(
                "search_codebase: result variant mismatch"
            ))))
        }
    }

    fn matches_proto_call(&self, tool: &message::tool_call::Tool) -> bool {
        matches!(tool, message::tool_call::Tool::SearchCodebase(_))
    }

    fn matches_proto_result(&self, result: &input::tool_call_result::Result) -> bool {
        matches!(result, input::tool_call_result::Result::SearchCodebase(_))
    }
}

// ---------------------------------------------------------------------------
// 9b. CallMCPTool
// ---------------------------------------------------------------------------

pub struct CallMcpToolTool;

/// Convert a `serde_json::Value::Object` into a `prost_types::Struct`.
/// Falls back to an empty Struct on any error (non-object values, etc.).
fn json_object_to_prost_struct(v: &Value) -> prost_types::Struct {
    fn json_to_prost_value(v: &Value) -> prost_types::Value {
        use prost_types::value::Kind;
        let kind = match v {
            Value::Null => Kind::NullValue(0),
            Value::Bool(b) => Kind::BoolValue(*b),
            Value::Number(n) => Kind::NumberValue(n.as_f64().unwrap_or(0.0)),
            Value::String(s) => Kind::StringValue(s.clone()),
            Value::Array(arr) => Kind::ListValue(prost_types::ListValue {
                values: arr.iter().map(json_to_prost_value).collect(),
            }),
            Value::Object(obj) => Kind::StructValue(prost_types::Struct {
                fields: obj
                    .iter()
                    .map(|(k, v)| (k.clone(), json_to_prost_value(v)))
                    .collect(),
            }),
        };
        prost_types::Value { kind: Some(kind) }
    }

    match v.as_object() {
        Some(obj) => prost_types::Struct {
            fields: obj
                .iter()
                .map(|(k, v)| (k.clone(), json_to_prost_value(v)))
                .collect(),
        },
        None => prost_types::Struct::default(),
    }
}

impl ToolDefinition for CallMcpToolTool {
    fn name(&self) -> &'static str {
        "call_mcp_tool"
    }

    fn description(&self) -> &'static str {
        "Invoke a tool exposed by a connected MCP server. \
         The args are an arbitrary JSON object specific to the tool."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "name": {
                    "type": "string",
                    "description": "The name of the MCP tool to invoke."
                },
                "args": {
                    "type": "object",
                    "description": "Tool-specific arguments as a JSON object.",
                    "additionalProperties": true
                },
                "server_id": {
                    "type": "string",
                    "description": "Optional ID of the MCP server that exposes this tool."
                }
            },
            "required": ["name", "args"]
        })
    }

    fn decode_call_args(
        &self,
        args: Value,
    ) -> std::result::Result<message::tool_call::Tool, Arc<AIApiError>> {
        let name = args
            .get("name")
            .and_then(|v| v.as_str())
            .ok_or_else(|| {
                Arc::new(AIApiError::Other(anyhow::anyhow!(
                    "call_mcp_tool: missing required `name` argument"
                )))
            })?
            .to_string();

        // `args` field is required — default to empty object if missing
        let binding = Value::Object(Default::default());
        let tool_args = args.get("args").unwrap_or(&binding);
        let prost_args = json_object_to_prost_struct(tool_args);

        let server_id = args
            .get("server_id")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        Ok(message::tool_call::Tool::CallMcpTool(
            message::tool_call::CallMcpTool {
                name,
                args: Some(prost_args),
                server_id,
            },
        ))
    }

    fn encode_call_args(&self, tool: &message::tool_call::Tool) -> Value {
        if let message::tool_call::Tool::CallMcpTool(ct) = tool {
            json!({
                "name": ct.name,
                "server_id": ct.server_id,
            })
        } else {
            Value::Object(Default::default())
        }
    }

    fn encode_result_text(
        &self,
        result: &input::tool_call_result::Result,
    ) -> std::result::Result<String, Arc<AIApiError>> {
        if let input::tool_call_result::Result::CallMcpTool(ct_result) = result {
            use warp_multi_agent_api::call_mcp_tool_result::Result as Inner;
            match ct_result.result.as_ref() {
                Some(Inner::Success(success)) => {
                    // Render content[] as JSON-like text
                    use warp_multi_agent_api::call_mcp_tool_result::success::result::Result as ContentVariant;
                    let parts: Vec<String> = success
                        .results
                        .iter()
                        .filter_map(|r| match r.result.as_ref()? {
                            ContentVariant::Text(t) => Some(t.text.clone()),
                            ContentVariant::Image(img) => {
                                Some(format!("[image: {} bytes, {}]", img.data.len(), img.mime_type))
                            }
                            ContentVariant::Resource(res) => Some(format!("[resource: {}]", res.uri)),
                        })
                        .collect();
                    Ok(parts.join("\n"))
                }
                Some(Inner::Error(e)) => Ok(format!("error: {}", e.message)),
                None => Ok("(no result)".to_string()),
            }
        } else {
            Err(Arc::new(AIApiError::Other(anyhow::anyhow!(
                "call_mcp_tool: result variant mismatch"
            ))))
        }
    }

    fn matches_proto_call(&self, tool: &message::tool_call::Tool) -> bool {
        matches!(tool, message::tool_call::Tool::CallMcpTool(_))
    }

    fn matches_proto_result(&self, result: &input::tool_call_result::Result) -> bool {
        matches!(result, input::tool_call_result::Result::CallMcpTool(_))
    }
}

// ---------------------------------------------------------------------------
// 9c. UseComputer
// ---------------------------------------------------------------------------

pub struct UseComputerTool;

impl ToolDefinition for UseComputerTool {
    fn name(&self) -> &'static str {
        "use_computer"
    }

    fn description(&self) -> &'static str {
        "Send mouse/keyboard actions to the computer (click, type, drag, scroll, screenshot)."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "actions": {
                    "type": "array",
                    "description": "List of actions to perform on the computer.",
                    "items": {
                        "type": "object",
                        "properties": {
                            "type": {
                                "type": "string",
                                "enum": ["click", "type", "wait", "mouse_move", "mouse_down", "mouse_up", "scroll", "key_down", "key_up"],
                                "description": "The type of action."
                            },
                            "x": { "type": "integer", "description": "X coordinate (for click, mouse_move, etc.)." },
                            "y": { "type": "integer", "description": "Y coordinate (for click, mouse_move, etc.)." },
                            "text": { "type": "string", "description": "Text to type (for type action)." },
                            "duration_ms": { "type": "integer", "description": "Duration in milliseconds (for wait action)." },
                            "button": {
                                "type": "string",
                                "enum": ["left", "right", "middle"],
                                "description": "Mouse button (for click/mouse_down/mouse_up)."
                            }
                        },
                        "required": ["type"]
                    }
                },
                "action_summary": {
                    "type": "string",
                    "description": "User-facing description of what the actions are doing."
                }
            },
            "required": ["actions", "action_summary"]
        })
    }

    fn decode_call_args(
        &self,
        args: Value,
    ) -> std::result::Result<message::tool_call::Tool, Arc<AIApiError>> {
        let action_summary = args
            .get("action_summary")
            .and_then(|v| v.as_str())
            .ok_or_else(|| {
                Arc::new(AIApiError::Other(anyhow::anyhow!(
                    "use_computer: missing required `action_summary` argument"
                )))
            })?
            .to_string();

        let actions_val = args
            .get("actions")
            .and_then(|v| v.as_array())
            .ok_or_else(|| {
                Arc::new(AIApiError::Other(anyhow::anyhow!(
                    "use_computer: missing required `actions` argument"
                )))
            })?;

        let mut actions: Vec<message::tool_call::use_computer::Action> =
            Vec::with_capacity(actions_val.len());

        for action_val in actions_val {
            let action_type = action_val
                .get("type")
                .and_then(|v| v.as_str())
                .ok_or_else(|| {
                    Arc::new(AIApiError::Other(anyhow::anyhow!(
                        "use_computer: action missing `type` field"
                    )))
                })?;

            use message::tool_call::use_computer::action;

            let variant = match action_type {
                "click" => {
                    // A click is mouse_down + mouse_up at coordinates.
                    // We model it as a mouse_down for simplicity at the MVP level.
                    let x = action_val.get("x").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
                    let y = action_val.get("y").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
                    let button = parse_mouse_button(action_val);
                    action::Type::MouseDown(action::MouseDown {
                        button,
                        at: Some(warp_multi_agent_api::Coordinates { x, y }),
                    })
                }
                "mouse_move" => {
                    let x = action_val.get("x").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
                    let y = action_val.get("y").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
                    action::Type::MouseMove(action::MouseMove {
                        to: Some(warp_multi_agent_api::Coordinates { x, y }),
                    })
                }
                "type" => {
                    let text = action_val
                        .get("text")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    action::Type::TypeText(action::TypeText { text })
                }
                "wait" => {
                    let duration_ms =
                        action_val.get("duration_ms").and_then(|v| v.as_i64()).unwrap_or(0);
                    let secs = duration_ms / 1000;
                    let nanos = ((duration_ms % 1000) * 1_000_000) as i32;
                    action::Type::Wait(action::Wait {
                        duration: Some(prost_types::Duration { seconds: secs, nanos }),
                    })
                }
                other => {
                    return Err(Arc::new(AIApiError::Other(anyhow::anyhow!(
                        "use_computer: unsupported action type `{}`; \
                         MVP supports: click, mouse_move, type, wait",
                        other
                    ))));
                }
            };

            actions.push(message::tool_call::use_computer::Action {
                r#type: Some(variant),
            });
        }

        Ok(message::tool_call::Tool::UseComputer(
            message::tool_call::UseComputer {
                actions,
                action_summary,
                post_actions_screenshot_params: None,
            },
        ))
    }

    fn encode_call_args(&self, tool: &message::tool_call::Tool) -> Value {
        if let message::tool_call::Tool::UseComputer(uc) = tool {
            json!({
                "action_summary": uc.action_summary,
                "action_count": uc.actions.len(),
            })
        } else {
            Value::Object(Default::default())
        }
    }

    fn encode_result_text(
        &self,
        result: &input::tool_call_result::Result,
    ) -> std::result::Result<String, Arc<AIApiError>> {
        if let input::tool_call_result::Result::UseComputer(uc_result) = result {
            use warp_multi_agent_api::use_computer_result::Result as Inner;
            match uc_result.result.as_ref() {
                Some(Inner::Success(success)) => {
                    let cursor = success
                        .cursor_position
                        .as_ref()
                        .map(|pos| format!("({},{})", pos.x, pos.y))
                        .unwrap_or_else(|| "unknown".to_string());
                    Ok(format!("screenshot taken, cursor at {}", cursor))
                }
                Some(Inner::Error(e)) => Ok(format!("error: {}", e.message)),
                None => Ok("(no result)".to_string()),
            }
        } else {
            Err(Arc::new(AIApiError::Other(anyhow::anyhow!(
                "use_computer: result variant mismatch"
            ))))
        }
    }

    fn matches_proto_call(&self, tool: &message::tool_call::Tool) -> bool {
        matches!(tool, message::tool_call::Tool::UseComputer(_))
    }

    fn matches_proto_result(&self, result: &input::tool_call_result::Result) -> bool {
        matches!(result, input::tool_call_result::Result::UseComputer(_))
    }
}

/// Parse a `"button"` field in an action JSON object into a proto `MouseButton` i32.
fn parse_mouse_button(action_val: &Value) -> i32 {
    use message::tool_call::use_computer::action::MouseButton;
    let btn = match action_val.get("button").and_then(|v| v.as_str()) {
        Some("right") => MouseButton::Right,
        Some("middle") => MouseButton::Middle,
        _ => MouseButton::Left,
    };
    btn as i32
}

// ---------------------------------------------------------------------------
// 9d. RequestComputerUse
// ---------------------------------------------------------------------------

pub struct RequestComputerUseTool;

impl ToolDefinition for RequestComputerUseTool {
    fn name(&self) -> &'static str {
        "request_computer_use"
    }

    fn description(&self) -> &'static str {
        "Ask the user for permission to drive their computer (mouse/keyboard)."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "task_summary": {
                    "type": "string",
                    "description": "A brief description of what the agent wants to do with computer use."
                }
            },
            "required": ["task_summary"]
        })
    }

    fn decode_call_args(
        &self,
        args: Value,
    ) -> std::result::Result<message::tool_call::Tool, Arc<AIApiError>> {
        let task_summary = args
            .get("task_summary")
            .and_then(|v| v.as_str())
            .ok_or_else(|| {
                Arc::new(AIApiError::Other(anyhow::anyhow!(
                    "request_computer_use: missing required `task_summary` argument"
                )))
            })?
            .to_string();

        Ok(message::tool_call::Tool::RequestComputerUse(
            message::tool_call::RequestComputerUse {
                task_summary,
                screenshot_params: None,
            },
        ))
    }

    fn encode_call_args(&self, tool: &message::tool_call::Tool) -> Value {
        if let message::tool_call::Tool::RequestComputerUse(rcu) = tool {
            json!({ "task_summary": rcu.task_summary })
        } else {
            Value::Object(Default::default())
        }
    }

    fn encode_result_text(
        &self,
        result: &input::tool_call_result::Result,
    ) -> std::result::Result<String, Arc<AIApiError>> {
        if let input::tool_call_result::Result::RequestComputerUse(rcu_result) = result {
            use warp_multi_agent_api::request_computer_use_result::Result as Inner;
            match rcu_result.result.as_ref() {
                Some(Inner::Approved(approved)) => {
                    let dims = approved
                        .screen_dimensions
                        .as_ref()
                        .map(|d| format!("{}x{}", d.width_px, d.height_px))
                        .unwrap_or_else(|| "unknown".to_string());
                    Ok(format!("approved, screen_dimensions: {}", dims))
                }
                Some(Inner::Rejected(_)) => Ok("user_rejected".to_string()),
                Some(Inner::Error(e)) => Ok(format!("error: {}", e.message)),
                None => Ok("(no result)".to_string()),
            }
        } else {
            Err(Arc::new(AIApiError::Other(anyhow::anyhow!(
                "request_computer_use: result variant mismatch"
            ))))
        }
    }

    fn matches_proto_call(&self, tool: &message::tool_call::Tool) -> bool {
        matches!(tool, message::tool_call::Tool::RequestComputerUse(_))
    }

    fn matches_proto_result(&self, result: &input::tool_call_result::Result) -> bool {
        matches!(
            result,
            input::tool_call_result::Result::RequestComputerUse(_)
        )
    }
}

// ---------------------------------------------------------------------------
// 9e. InsertReviewComments
// ---------------------------------------------------------------------------

pub struct InsertReviewCommentsTool;

impl ToolDefinition for InsertReviewCommentsTool {
    fn name(&self) -> &'static str {
        "insert_review_comments"
    }

    fn description(&self) -> &'static str {
        "Insert PR review comments on a code review."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "repo_path": {
                    "type": "string",
                    "description": "Path to the local git repository."
                },
                "base_branch": {
                    "type": "string",
                    "description": "The base branch to compare against (e.g. main, master)."
                },
                "comments": {
                    "type": "array",
                    "description": "List of review comments to insert.",
                    "items": {
                        "type": "object",
                        "properties": {
                            "comment_id": { "type": "string" },
                            "author": { "type": "string" },
                            "comment_body": {
                                "type": "string",
                                "description": "The body of the review comment."
                            },
                            "file_path": {
                                "type": "string",
                                "description": "Path of the file the comment is attached to."
                            },
                            "diff_hunk": {
                                "type": "string",
                                "description": "The diff hunk the comment is attached to."
                            }
                        },
                        "required": ["comment_body"]
                    }
                }
            },
            "required": ["repo_path", "comments"]
        })
    }

    fn decode_call_args(
        &self,
        args: Value,
    ) -> std::result::Result<message::tool_call::Tool, Arc<AIApiError>> {
        let repo_path = args
            .get("repo_path")
            .and_then(|v| v.as_str())
            .ok_or_else(|| {
                Arc::new(AIApiError::Other(anyhow::anyhow!(
                    "insert_review_comments: missing required `repo_path` argument"
                )))
            })?
            .to_string();

        let base_branch = args
            .get("base_branch")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        let comments_val = args
            .get("comments")
            .and_then(|v| v.as_array())
            .ok_or_else(|| {
                Arc::new(AIApiError::Other(anyhow::anyhow!(
                    "insert_review_comments: missing required `comments` argument"
                )))
            })?;

        let comments: Vec<message::tool_call::insert_review_comments::Comment> = comments_val
            .iter()
            .map(|c| {
                let comment_id = c.get("comment_id").and_then(|v| v.as_str()).unwrap_or("").to_string();
                let author = c.get("author").and_then(|v| v.as_str()).unwrap_or("").to_string();
                let comment_body = c.get("comment_body").and_then(|v| v.as_str()).unwrap_or("").to_string();

                let location = {
                    let file_path = c.get("file_path").and_then(|v| v.as_str()).unwrap_or("").to_string();
                    let diff_hunk = c.get("diff_hunk").and_then(|v| v.as_str()).unwrap_or("").to_string();
                    if file_path.is_empty() && diff_hunk.is_empty() {
                        None
                    } else {
                        Some(message::tool_call::insert_review_comments::CommentLocation {
                            file_path,
                            line: Some(message::tool_call::insert_review_comments::CommentLineRange {
                                diff_hunk,
                                range: None,
                                side: 0, // NEW (right side)
                            }),
                        })
                    }
                };

                message::tool_call::insert_review_comments::Comment {
                    comment_id,
                    author,
                    comment_body,
                    location,
                    ..Default::default()
                }
            })
            .collect();

        Ok(message::tool_call::Tool::InsertReviewComments(
            message::tool_call::InsertReviewComments {
                repo_path,
                base_branch,
                comments,
            },
        ))
    }

    fn encode_call_args(&self, tool: &message::tool_call::Tool) -> Value {
        if let message::tool_call::Tool::InsertReviewComments(irc) = tool {
            json!({
                "repo_path": irc.repo_path,
                "base_branch": irc.base_branch,
                "comment_count": irc.comments.len(),
            })
        } else {
            Value::Object(Default::default())
        }
    }

    fn encode_result_text(
        &self,
        result: &input::tool_call_result::Result,
    ) -> std::result::Result<String, Arc<AIApiError>> {
        if let input::tool_call_result::Result::InsertReviewComments(irc_result) = result {
            use warp_multi_agent_api::insert_review_comments_result::Result as Inner;
            match irc_result.result.as_ref() {
                Some(Inner::Success(_)) => Ok("comments inserted".to_string()),
                Some(Inner::Error(e)) => Ok(format!("error: {}", e.message)),
                None => Ok("(no result)".to_string()),
            }
        } else {
            Err(Arc::new(AIApiError::Other(anyhow::anyhow!(
                "insert_review_comments: result variant mismatch"
            ))))
        }
    }

    fn matches_proto_call(&self, tool: &message::tool_call::Tool) -> bool {
        matches!(tool, message::tool_call::Tool::InsertReviewComments(_))
    }

    fn matches_proto_result(&self, result: &input::tool_call_result::Result) -> bool {
        matches!(
            result,
            input::tool_call_result::Result::InsertReviewComments(_)
        )
    }
}

// ---------------------------------------------------------------------------
// 9f. UploadFileArtifact
// ---------------------------------------------------------------------------

pub struct UploadFileArtifactTool;

impl ToolDefinition for UploadFileArtifactTool {
    fn name(&self) -> &'static str {
        "upload_file_artifact"
    }

    fn description(&self) -> &'static str {
        "Upload a local file as a conversation artifact."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "file": {
                    "type": "object",
                    "description": "The file to upload.",
                    "properties": {
                        "path": {
                            "type": "string",
                            "description": "Absolute path to the local file."
                        }
                    },
                    "required": ["path"]
                },
                "description": {
                    "type": "string",
                    "description": "Human-readable description of the artifact."
                }
            },
            "required": ["file", "description"]
        })
    }

    fn decode_call_args(
        &self,
        args: Value,
    ) -> std::result::Result<message::tool_call::Tool, Arc<AIApiError>> {
        let file_path = args
            .get("file")
            .and_then(|f| f.get("path"))
            .and_then(|v| v.as_str())
            .ok_or_else(|| {
                Arc::new(AIApiError::Other(anyhow::anyhow!(
                    "upload_file_artifact: missing required `file.path` argument"
                )))
            })?
            .to_string();

        let description = args
            .get("description")
            .and_then(|v| v.as_str())
            .ok_or_else(|| {
                Arc::new(AIApiError::Other(anyhow::anyhow!(
                    "upload_file_artifact: missing required `description` argument"
                )))
            })?
            .to_string();

        Ok(message::tool_call::Tool::UploadFileArtifact(
            warp_multi_agent_api::UploadFileArtifact {
                file: Some(warp_multi_agent_api::FilePathReference { file_path }),
                description,
            },
        ))
    }

    fn encode_call_args(&self, tool: &message::tool_call::Tool) -> Value {
        if let message::tool_call::Tool::UploadFileArtifact(ufa) = tool {
            json!({
                "file": {
                    "path": ufa.file.as_ref().map(|f| f.file_path.as_str()).unwrap_or(""),
                },
                "description": ufa.description,
            })
        } else {
            Value::Object(Default::default())
        }
    }

    fn encode_result_text(
        &self,
        result: &input::tool_call_result::Result,
    ) -> std::result::Result<String, Arc<AIApiError>> {
        if let input::tool_call_result::Result::UploadFileArtifact(ufa_result) = result {
            use warp_multi_agent_api::upload_file_artifact_result::Result as Inner;
            match ufa_result.result.as_ref() {
                Some(Inner::Success(success)) => Ok(format!(
                    "artifact_uid: {}, size_bytes: {}",
                    success.artifact_uid, success.size_bytes
                )),
                Some(Inner::Error(e)) => Ok(format!("error: {}", e.message)),
                None => Ok("(no result)".to_string()),
            }
        } else {
            Err(Arc::new(AIApiError::Other(anyhow::anyhow!(
                "upload_file_artifact: result variant mismatch"
            ))))
        }
    }

    fn matches_proto_call(&self, tool: &message::tool_call::Tool) -> bool {
        matches!(tool, message::tool_call::Tool::UploadFileArtifact(_))
    }

    fn matches_proto_result(&self, result: &input::tool_call_result::Result) -> bool {
        matches!(
            result,
            input::tool_call_result::Result::UploadFileArtifact(_)
        )
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decode_search_codebase_minimal() {
        let args = json!({ "query": "find all TODO comments" });
        let tool = SEARCH_CODEBASE.decode_call_args(args).expect("decode");
        match tool {
            message::tool_call::Tool::SearchCodebase(sc) => {
                assert_eq!(sc.query, "find all TODO comments");
                assert!(sc.path_filters.is_empty());
                assert!(sc.codebase_path.is_empty());
            }
            _ => panic!("expected SearchCodebase variant"),
        }
    }

    #[test]
    fn decode_search_codebase_with_filters() {
        let args = json!({
            "query": "async fn",
            "path_filters": ["src/", "lib/"],
            "codebase_path": "/home/user/project"
        });
        let tool = SEARCH_CODEBASE.decode_call_args(args).expect("decode");
        match tool {
            message::tool_call::Tool::SearchCodebase(sc) => {
                assert_eq!(sc.query, "async fn");
                assert_eq!(sc.path_filters, vec!["src/", "lib/"]);
                assert_eq!(sc.codebase_path, "/home/user/project");
            }
            _ => panic!("expected SearchCodebase variant"),
        }
    }

    #[test]
    fn decode_call_mcp_tool_minimal() {
        let args = json!({
            "name": "my_mcp_tool",
            "args": { "key": "value", "count": 42 }
        });
        let tool = CALL_MCP_TOOL.decode_call_args(args).expect("decode");
        match tool {
            message::tool_call::Tool::CallMcpTool(ct) => {
                assert_eq!(ct.name, "my_mcp_tool");
                assert!(ct.args.is_some());
                let prost_struct = ct.args.unwrap();
                assert!(prost_struct.fields.contains_key("key"));
                assert!(prost_struct.fields.contains_key("count"));
            }
            _ => panic!("expected CallMcpTool variant"),
        }
    }

    #[test]
    fn decode_use_computer_click_action() {
        let args = json!({
            "actions": [
                { "type": "click", "x": 100, "y": 200, "button": "left" }
            ],
            "action_summary": "Click the submit button"
        });
        let tool = USE_COMPUTER.decode_call_args(args).expect("decode");
        match tool {
            message::tool_call::Tool::UseComputer(uc) => {
                assert_eq!(uc.action_summary, "Click the submit button");
                assert_eq!(uc.actions.len(), 1);
                let action = &uc.actions[0];
                match action.r#type.as_ref().expect("action type") {
                    message::tool_call::use_computer::action::Type::MouseDown(md) => {
                        let coords = md.at.as_ref().expect("coords");
                        assert_eq!(coords.x, 100);
                        assert_eq!(coords.y, 200);
                    }
                    _ => panic!("expected MouseDown (click) variant"),
                }
            }
            _ => panic!("expected UseComputer variant"),
        }
    }

    #[test]
    fn decode_use_computer_type_action() {
        let args = json!({
            "actions": [
                { "type": "type", "text": "hello world" }
            ],
            "action_summary": "Type greeting"
        });
        let tool = USE_COMPUTER.decode_call_args(args).expect("decode");
        match tool {
            message::tool_call::Tool::UseComputer(uc) => {
                assert_eq!(uc.actions.len(), 1);
                match uc.actions[0].r#type.as_ref().expect("action type") {
                    message::tool_call::use_computer::action::Type::TypeText(tt) => {
                        assert_eq!(tt.text, "hello world");
                    }
                    _ => panic!("expected TypeText variant"),
                }
            }
            _ => panic!("expected UseComputer variant"),
        }
    }

    #[test]
    fn decode_use_computer_unsupported_action_returns_error() {
        let args = json!({
            "actions": [{ "type": "drag", "from_x": 0, "from_y": 0, "to_x": 100, "to_y": 100 }],
            "action_summary": "Drag element"
        });
        let err = USE_COMPUTER.decode_call_args(args).expect_err("should fail");
        assert!(format!("{err:#}").contains("unsupported action type"));
    }

    #[test]
    fn decode_request_computer_use_minimal() {
        let args = json!({ "task_summary": "Automate form filling" });
        let tool = REQUEST_COMPUTER_USE.decode_call_args(args).expect("decode");
        match tool {
            message::tool_call::Tool::RequestComputerUse(rcu) => {
                assert_eq!(rcu.task_summary, "Automate form filling");
                assert!(rcu.screenshot_params.is_none());
            }
            _ => panic!("expected RequestComputerUse variant"),
        }
    }

    #[test]
    fn decode_insert_review_comments_minimal() {
        let args = json!({
            "repo_path": "/home/user/my-repo",
            "base_branch": "main",
            "comments": [
                {
                    "comment_body": "This could be simplified.",
                    "file_path": "src/main.rs",
                    "diff_hunk": "@@ -10,5 +10,4 @@"
                }
            ]
        });
        let tool = INSERT_REVIEW_COMMENTS.decode_call_args(args).expect("decode");
        match tool {
            message::tool_call::Tool::InsertReviewComments(irc) => {
                assert_eq!(irc.repo_path, "/home/user/my-repo");
                assert_eq!(irc.base_branch, "main");
                assert_eq!(irc.comments.len(), 1);
                assert_eq!(irc.comments[0].comment_body, "This could be simplified.");
            }
            _ => panic!("expected InsertReviewComments variant"),
        }
    }

    #[test]
    fn decode_upload_file_artifact_minimal() {
        let args = json!({
            "file": { "path": "/tmp/output.txt" },
            "description": "Command output log"
        });
        let tool = UPLOAD_FILE_ARTIFACT.decode_call_args(args).expect("decode");
        match tool {
            message::tool_call::Tool::UploadFileArtifact(ufa) => {
                let file = ufa.file.as_ref().expect("file");
                assert_eq!(file.file_path, "/tmp/output.txt");
                assert_eq!(ufa.description, "Command output log");
            }
            _ => panic!("expected UploadFileArtifact variant"),
        }
    }
}

//! Read and search tools: ReadFiles, ReadDocuments, Grep, FileGlobV2,
//! ReadShellCommandOutput, ReadSkill, ReadMCPResource.

use serde_json::{json, Value};
use std::sync::Arc;
use warp_multi_agent_api::message;
use warp_multi_agent_api::request::input;

use crate::tools::ToolDefinition;
use crate::AIApiError;

pub static READ_FILES: ReadFilesTool = ReadFilesTool;
pub static READ_DOCUMENTS: ReadDocumentsTool = ReadDocumentsTool;
pub static GREP: GrepTool = GrepTool;
pub static FILE_GLOB_V2: FileGlobV2Tool = FileGlobV2Tool;
pub static READ_SHELL_COMMAND_OUTPUT: ReadShellCommandOutputTool = ReadShellCommandOutputTool;
pub static READ_SKILL: ReadSkillTool = ReadSkillTool;
pub static READ_MCP_RESOURCE: ReadMcpResourceTool = ReadMcpResourceTool;

// ---------------------------------------------------------------------------
// 7a. ReadFiles
// ---------------------------------------------------------------------------

pub struct ReadFilesTool;

impl ToolDefinition for ReadFilesTool {
    fn name(&self) -> &'static str {
        "read_files"
    }

    fn description(&self) -> &'static str {
        "Read the contents of one or more files."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "files": {
                    "type": "array",
                    "description": "List of files to read.",
                    "items": {
                        "type": "object",
                        "properties": {
                            "name": {
                                "type": "string",
                                "description": "Path to the file."
                            },
                            "line_ranges": {
                                "type": "array",
                                "description": "Optional line ranges to read. If omitted, the entire file is read.",
                                "items": {
                                    "type": "object",
                                    "properties": {
                                        "start": { "type": "integer" },
                                        "end": { "type": "integer" }
                                    },
                                    "required": ["start", "end"]
                                }
                            }
                        },
                        "required": ["name"]
                    }
                }
            },
            "required": ["files"]
        })
    }

    fn decode_call_args(
        &self,
        args: Value,
    ) -> std::result::Result<message::tool_call::Tool, Arc<AIApiError>> {
        let files_val = args.get("files").and_then(|v| v.as_array()).ok_or_else(|| {
            Arc::new(AIApiError::Other(anyhow::anyhow!(
                "read_files: missing required `files` argument"
            )))
        })?;

        let files: Vec<message::tool_call::read_files::File> = files_val
            .iter()
            .filter_map(|f| {
                let name = f.get("name")?.as_str()?.to_string();
                let line_ranges = f
                    .get("line_ranges")
                    .and_then(|v| v.as_array())
                    .map(|arr| {
                        arr.iter()
                            .filter_map(|r| {
                                let start = r.get("start")?.as_u64()? as u32;
                                let end = r.get("end")?.as_u64()? as u32;
                                Some(warp_multi_agent_api::FileContentLineRange { start, end })
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                Some(message::tool_call::read_files::File { name, line_ranges })
            })
            .collect();

        Ok(message::tool_call::Tool::ReadFiles(
            message::tool_call::ReadFiles { files },
        ))
    }

    fn encode_call_args(&self, tool: &message::tool_call::Tool) -> Value {
        if let message::tool_call::Tool::ReadFiles(rf) = tool {
            json!({
                "files": rf.files.iter().map(|f| json!({ "name": f.name })).collect::<Vec<_>>()
            })
        } else {
            Value::Object(Default::default())
        }
    }

    fn encode_result_text(
        &self,
        result: &input::tool_call_result::Result,
    ) -> std::result::Result<String, Arc<AIApiError>> {
        if let input::tool_call_result::Result::ReadFiles(rf_result) = result {
            use warp_multi_agent_api::read_files_result::Result as Inner;
            match rf_result.result.as_ref() {
                Some(Inner::TextFilesSuccess(success)) => {
                    let content = success
                        .files
                        .iter()
                        .map(|f| format!("{}:\n{}", f.file_path, f.content))
                        .collect::<Vec<_>>()
                        .join("\n\n");
                    Ok(content)
                }
                Some(Inner::AnyFilesSuccess(success)) => {
                    // Render the text-content files only; skip binary.
                    use warp_multi_agent_api::any_file_content;
                    let content = success
                        .files
                        .iter()
                        .filter_map(|f| {
                            if let Some(any_file_content::Content::TextContent(tc)) =
                                f.content.as_ref()
                            {
                                Some(format!("{}:\n{}", tc.file_path, tc.content))
                            } else {
                                None
                            }
                        })
                        .collect::<Vec<_>>()
                        .join("\n\n");
                    Ok(content)
                }
                Some(Inner::Error(e)) => Ok(format!("error: {}", e.message)),
                None => Ok("(no result)".to_string()),
            }
        } else {
            Err(Arc::new(AIApiError::Other(anyhow::anyhow!(
                "read_files: result variant mismatch"
            ))))
        }
    }

    fn matches_proto_call(&self, tool: &message::tool_call::Tool) -> bool {
        matches!(tool, message::tool_call::Tool::ReadFiles(_))
    }

    fn matches_proto_result(&self, result: &input::tool_call_result::Result) -> bool {
        matches!(result, input::tool_call_result::Result::ReadFiles(_))
    }
}

// ---------------------------------------------------------------------------
// 7b. ReadDocuments
// ---------------------------------------------------------------------------

pub struct ReadDocumentsTool;

impl ToolDefinition for ReadDocumentsTool {
    fn name(&self) -> &'static str {
        "read_documents"
    }

    fn description(&self) -> &'static str {
        "Read Warp Drive documents by ID."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "documents": {
                    "type": "array",
                    "description": "List of documents to read.",
                    "items": {
                        "type": "object",
                        "properties": {
                            "document_id": {
                                "type": "string",
                                "description": "The unique identifier of the document."
                            },
                            "line_ranges": {
                                "type": "array",
                                "description": "Optional line ranges to read. If omitted, the entire document is read.",
                                "items": {
                                    "type": "object",
                                    "properties": {
                                        "start": { "type": "integer" },
                                        "end": { "type": "integer" }
                                    },
                                    "required": ["start", "end"]
                                }
                            }
                        },
                        "required": ["document_id"]
                    }
                }
            },
            "required": ["documents"]
        })
    }

    fn decode_call_args(
        &self,
        args: Value,
    ) -> std::result::Result<message::tool_call::Tool, Arc<AIApiError>> {
        let docs_val = args
            .get("documents")
            .and_then(|v| v.as_array())
            .ok_or_else(|| {
                Arc::new(AIApiError::Other(anyhow::anyhow!(
                    "read_documents: missing required `documents` argument"
                )))
            })?;

        let documents: Vec<message::tool_call::read_documents::Document> = docs_val
            .iter()
            .filter_map(|d| {
                let document_id = d.get("document_id")?.as_str()?.to_string();
                let line_ranges = d
                    .get("line_ranges")
                    .and_then(|v| v.as_array())
                    .map(|arr| {
                        arr.iter()
                            .filter_map(|r| {
                                let start = r.get("start")?.as_u64()? as u32;
                                let end = r.get("end")?.as_u64()? as u32;
                                Some(warp_multi_agent_api::FileContentLineRange { start, end })
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                Some(message::tool_call::read_documents::Document {
                    document_id,
                    line_ranges,
                })
            })
            .collect();

        Ok(message::tool_call::Tool::ReadDocuments(
            message::tool_call::ReadDocuments { documents },
        ))
    }

    fn encode_call_args(&self, tool: &message::tool_call::Tool) -> Value {
        if let message::tool_call::Tool::ReadDocuments(rd) = tool {
            json!({
                "documents": rd.documents.iter().map(|d| json!({ "document_id": d.document_id })).collect::<Vec<_>>()
            })
        } else {
            Value::Object(Default::default())
        }
    }

    fn encode_result_text(
        &self,
        result: &input::tool_call_result::Result,
    ) -> std::result::Result<String, Arc<AIApiError>> {
        if let input::tool_call_result::Result::ReadDocuments(rd_result) = result {
            use warp_multi_agent_api::read_documents_result::Result as Inner;
            match rd_result.result.as_ref() {
                Some(Inner::Success(success)) => {
                    let content = success
                        .documents
                        .iter()
                        .map(|d| format!("{}:\n{}", d.document_id, d.content))
                        .collect::<Vec<_>>()
                        .join("\n\n");
                    Ok(content)
                }
                Some(Inner::Error(e)) => Ok(format!("error: {}", e.message)),
                None => Ok("(no result)".to_string()),
            }
        } else {
            Err(Arc::new(AIApiError::Other(anyhow::anyhow!(
                "read_documents: result variant mismatch"
            ))))
        }
    }

    fn matches_proto_call(&self, tool: &message::tool_call::Tool) -> bool {
        matches!(tool, message::tool_call::Tool::ReadDocuments(_))
    }

    fn matches_proto_result(&self, result: &input::tool_call_result::Result) -> bool {
        matches!(result, input::tool_call_result::Result::ReadDocuments(_))
    }
}

// ---------------------------------------------------------------------------
// 7c. Grep
// ---------------------------------------------------------------------------

pub struct GrepTool;

impl ToolDefinition for GrepTool {
    fn name(&self) -> &'static str {
        "grep"
    }

    fn description(&self) -> &'static str {
        "Search for patterns in files using regex."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "queries": {
                    "type": "array",
                    "description": "The search terms or patterns to look for.",
                    "items": { "type": "string" }
                },
                "path": {
                    "type": "string",
                    "description": "The relative path to the file or directory to search in."
                }
            },
            "required": ["queries", "path"]
        })
    }

    fn decode_call_args(
        &self,
        args: Value,
    ) -> std::result::Result<message::tool_call::Tool, Arc<AIApiError>> {
        let queries: Vec<String> = args
            .get("queries")
            .and_then(|v| v.as_array())
            .ok_or_else(|| {
                Arc::new(AIApiError::Other(anyhow::anyhow!(
                    "grep: missing required `queries` argument"
                )))
            })?
            .iter()
            .filter_map(|v| v.as_str().map(|s| s.to_string()))
            .collect();

        let path = args
            .get("path")
            .and_then(|v| v.as_str())
            .ok_or_else(|| {
                Arc::new(AIApiError::Other(anyhow::anyhow!(
                    "grep: missing required `path` argument"
                )))
            })?
            .to_string();

        Ok(message::tool_call::Tool::Grep(message::tool_call::Grep {
            queries,
            path,
        }))
    }

    fn encode_call_args(&self, tool: &message::tool_call::Tool) -> Value {
        if let message::tool_call::Tool::Grep(g) = tool {
            json!({ "queries": g.queries, "path": g.path })
        } else {
            Value::Object(Default::default())
        }
    }

    fn encode_result_text(
        &self,
        result: &input::tool_call_result::Result,
    ) -> std::result::Result<String, Arc<AIApiError>> {
        if let input::tool_call_result::Result::Grep(grep_result) = result {
            use warp_multi_agent_api::grep_result::Result as Inner;
            match grep_result.result.as_ref() {
                Some(Inner::Success(success)) => {
                    let lines: Vec<String> = success
                        .matched_files
                        .iter()
                        .flat_map(|file_match| {
                            file_match.matched_lines.iter().map(|line_match| {
                                format!("{}:{}", file_match.file_path, line_match.line_number)
                            })
                        })
                        .collect();
                    Ok(lines.join("\n"))
                }
                Some(Inner::Error(e)) => Ok(format!("error: {}", e.message)),
                None => Ok("(no result)".to_string()),
            }
        } else {
            Err(Arc::new(AIApiError::Other(anyhow::anyhow!(
                "grep: result variant mismatch"
            ))))
        }
    }

    fn matches_proto_call(&self, tool: &message::tool_call::Tool) -> bool {
        matches!(tool, message::tool_call::Tool::Grep(_))
    }

    fn matches_proto_result(&self, result: &input::tool_call_result::Result) -> bool {
        matches!(result, input::tool_call_result::Result::Grep(_))
    }
}

// ---------------------------------------------------------------------------
// 7d. FileGlobV2
// ---------------------------------------------------------------------------

pub struct FileGlobV2Tool;

impl ToolDefinition for FileGlobV2Tool {
    fn name(&self) -> &'static str {
        "file_glob_v2"
    }

    fn description(&self) -> &'static str {
        "Find files matching glob patterns."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "patterns": {
                    "type": "array",
                    "description": "The patterns to match file names against. Supports ?, *, [].",
                    "items": { "type": "string" }
                },
                "search_dir": {
                    "type": "string",
                    "description": "The relative path to the directory to search in."
                },
                "max_matches": {
                    "type": "integer",
                    "description": "The maximum number of matches to return. Zero indicates no limit."
                },
                "max_depth": {
                    "type": "integer",
                    "description": "The maximum depth to search in. Zero indicates no limit."
                },
                "min_depth": {
                    "type": "integer",
                    "description": "The minimum depth to search in. Zero indicates no limit."
                }
            },
            "required": ["patterns", "search_dir"]
        })
    }

    fn decode_call_args(
        &self,
        args: Value,
    ) -> std::result::Result<message::tool_call::Tool, Arc<AIApiError>> {
        let patterns: Vec<String> = args
            .get("patterns")
            .and_then(|v| v.as_array())
            .ok_or_else(|| {
                Arc::new(AIApiError::Other(anyhow::anyhow!(
                    "file_glob_v2: missing required `patterns` argument"
                )))
            })?
            .iter()
            .filter_map(|v| v.as_str().map(|s| s.to_string()))
            .collect();

        let search_dir = args
            .get("search_dir")
            .and_then(|v| v.as_str())
            .ok_or_else(|| {
                Arc::new(AIApiError::Other(anyhow::anyhow!(
                    "file_glob_v2: missing required `search_dir` argument"
                )))
            })?
            .to_string();

        let max_matches = args
            .get("max_matches")
            .and_then(|v| v.as_i64())
            .unwrap_or(0) as i32;
        let max_depth = args
            .get("max_depth")
            .and_then(|v| v.as_i64())
            .unwrap_or(0) as i32;
        let min_depth = args
            .get("min_depth")
            .and_then(|v| v.as_i64())
            .unwrap_or(0) as i32;

        Ok(message::tool_call::Tool::FileGlobV2(
            message::tool_call::FileGlobV2 {
                patterns,
                search_dir,
                max_matches,
                max_depth,
                min_depth,
            },
        ))
    }

    fn encode_call_args(&self, tool: &message::tool_call::Tool) -> Value {
        if let message::tool_call::Tool::FileGlobV2(fg) = tool {
            json!({ "patterns": fg.patterns, "search_dir": fg.search_dir })
        } else {
            Value::Object(Default::default())
        }
    }

    fn encode_result_text(
        &self,
        result: &input::tool_call_result::Result,
    ) -> std::result::Result<String, Arc<AIApiError>> {
        if let input::tool_call_result::Result::FileGlobV2(fg_result) = result {
            use warp_multi_agent_api::file_glob_v2_result::Result as Inner;
            match fg_result.result.as_ref() {
                Some(Inner::Success(success)) => {
                    let paths: Vec<&str> = success
                        .matched_files
                        .iter()
                        .map(|m| m.file_path.as_str())
                        .collect();
                    Ok(paths.join("\n"))
                }
                Some(Inner::Error(e)) => Ok(format!("error: {}", e.message)),
                None => Ok("(no result)".to_string()),
            }
        } else {
            Err(Arc::new(AIApiError::Other(anyhow::anyhow!(
                "file_glob_v2: result variant mismatch"
            ))))
        }
    }

    fn matches_proto_call(&self, tool: &message::tool_call::Tool) -> bool {
        matches!(tool, message::tool_call::Tool::FileGlobV2(_))
    }

    fn matches_proto_result(&self, result: &input::tool_call_result::Result) -> bool {
        matches!(result, input::tool_call_result::Result::FileGlobV2(_))
    }
}

// ---------------------------------------------------------------------------
// 7e. ReadShellCommandOutput
// ---------------------------------------------------------------------------

pub struct ReadShellCommandOutputTool;

impl ToolDefinition for ReadShellCommandOutputTool {
    fn name(&self) -> &'static str {
        "read_shell_command_output"
    }

    fn description(&self) -> &'static str {
        "Read output from a long-running shell command."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "command_id": {
                    "type": "string",
                    "description": "The ID of the long-running command whose output to read."
                },
                "delay_ms": {
                    "type": "integer",
                    "description": "Optional delay in milliseconds before returning the output."
                }
            },
            "required": ["command_id"]
        })
    }

    fn decode_call_args(
        &self,
        args: Value,
    ) -> std::result::Result<message::tool_call::Tool, Arc<AIApiError>> {
        let command_id = args
            .get("command_id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| {
                Arc::new(AIApiError::Other(anyhow::anyhow!(
                    "read_shell_command_output: missing required `command_id` argument"
                )))
            })?
            .to_string();

        let delay = args.get("delay_ms").and_then(|v| v.as_i64()).map(|ms| {
            message::tool_call::read_shell_command_output::Delay::Duration(
                prost_types::Duration {
                    seconds: ms / 1000,
                    nanos: ((ms % 1000) * 1_000_000) as i32,
                },
            )
        });

        Ok(message::tool_call::Tool::ReadShellCommandOutput(
            message::tool_call::ReadShellCommandOutput {
                command_id,
                delay,
            },
        ))
    }

    fn encode_call_args(&self, tool: &message::tool_call::Tool) -> Value {
        if let message::tool_call::Tool::ReadShellCommandOutput(rsco) = tool {
            json!({ "command_id": rsco.command_id })
        } else {
            Value::Object(Default::default())
        }
    }

    fn encode_result_text(
        &self,
        result: &input::tool_call_result::Result,
    ) -> std::result::Result<String, Arc<AIApiError>> {
        if let input::tool_call_result::Result::ReadShellCommandOutput(rsco_result) = result {
            use warp_multi_agent_api::read_shell_command_output_result::Result as Inner;
            match rsco_result.result.as_ref() {
                Some(Inner::CommandFinished(finished)) => Ok(format!(
                    "exit_code: {}\noutput:\n{}",
                    finished.exit_code, finished.output
                )),
                Some(Inner::LongRunningCommandSnapshot(snap)) => Ok(format!(
                    "(long-running snapshot, command_id: {})\n{}",
                    snap.command_id, snap.output
                )),
                Some(Inner::Error(_)) => {
                    Ok("error: command not found".to_string())
                }
                None => Ok("(no result)".to_string()),
            }
        } else {
            Err(Arc::new(AIApiError::Other(anyhow::anyhow!(
                "read_shell_command_output: result variant mismatch"
            ))))
        }
    }

    fn matches_proto_call(&self, tool: &message::tool_call::Tool) -> bool {
        matches!(tool, message::tool_call::Tool::ReadShellCommandOutput(_))
    }

    fn matches_proto_result(&self, result: &input::tool_call_result::Result) -> bool {
        matches!(
            result,
            input::tool_call_result::Result::ReadShellCommandOutput(_)
        )
    }
}

// ---------------------------------------------------------------------------
// 7f. ReadSkill
// ---------------------------------------------------------------------------

pub struct ReadSkillTool;

impl ToolDefinition for ReadSkillTool {
    fn name(&self) -> &'static str {
        "read_skill"
    }

    fn description(&self) -> &'static str {
        "Read a Warp Skill (instructions/prompt template)."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "name": {
                    "type": "string",
                    "description": "The name of the skill."
                },
                "skill_path": {
                    "type": "string",
                    "description": "Optional path to the SKILL.md file to read."
                },
                "bundled_skill_id": {
                    "type": "string",
                    "description": "Optional unique identifier for a skill bundled with the client."
                }
            },
            "required": ["name"]
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
                    "read_skill: missing required `name` argument"
                )))
            })?
            .to_string();

        let skill_reference = args
            .get("skill_path")
            .and_then(|v| v.as_str())
            .map(|path| message::tool_call::read_skill::SkillReference::SkillPath(
                path.to_string(),
            ))
            .or_else(|| {
                args.get("bundled_skill_id")
                    .and_then(|v| v.as_str())
                    .map(|id| {
                        message::tool_call::read_skill::SkillReference::BundledSkillId(
                            id.to_string(),
                        )
                    })
            });

        Ok(message::tool_call::Tool::ReadSkill(
            message::tool_call::ReadSkill {
                name,
                skill_reference,
            },
        ))
    }

    fn encode_call_args(&self, tool: &message::tool_call::Tool) -> Value {
        if let message::tool_call::Tool::ReadSkill(rs) = tool {
            json!({ "name": rs.name })
        } else {
            Value::Object(Default::default())
        }
    }

    fn encode_result_text(
        &self,
        result: &input::tool_call_result::Result,
    ) -> std::result::Result<String, Arc<AIApiError>> {
        if let input::tool_call_result::Result::ReadSkill(rs_result) = result {
            use warp_multi_agent_api::read_skill_result::Result as Inner;
            match rs_result.result.as_ref() {
                Some(Inner::Success(success)) => {
                    if let Some(content) = &success.content {
                        Ok(format!("{}:\n{}", content.file_path, content.content))
                    } else {
                        Ok("(no skill content)".to_string())
                    }
                }
                Some(Inner::Error(e)) => Ok(format!("error: {}", e.message)),
                None => Ok("(no result)".to_string()),
            }
        } else {
            Err(Arc::new(AIApiError::Other(anyhow::anyhow!(
                "read_skill: result variant mismatch"
            ))))
        }
    }

    fn matches_proto_call(&self, tool: &message::tool_call::Tool) -> bool {
        matches!(tool, message::tool_call::Tool::ReadSkill(_))
    }

    fn matches_proto_result(&self, result: &input::tool_call_result::Result) -> bool {
        matches!(result, input::tool_call_result::Result::ReadSkill(_))
    }
}

// ---------------------------------------------------------------------------
// 7g. ReadMCPResource
// ---------------------------------------------------------------------------

pub struct ReadMcpResourceTool;

impl ToolDefinition for ReadMcpResourceTool {
    fn name(&self) -> &'static str {
        "read_mcp_resource"
    }

    fn description(&self) -> &'static str {
        "Read a resource from a connected MCP server."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "uri": {
                    "type": "string",
                    "description": "The URI of the MCP resource to read."
                },
                "server_id": {
                    "type": "string",
                    "description": "Optional identifier of the MCP server that provides this resource."
                }
            },
            "required": ["uri"]
        })
    }

    fn decode_call_args(
        &self,
        args: Value,
    ) -> std::result::Result<message::tool_call::Tool, Arc<AIApiError>> {
        let uri = args
            .get("uri")
            .and_then(|v| v.as_str())
            .ok_or_else(|| {
                Arc::new(AIApiError::Other(anyhow::anyhow!(
                    "read_mcp_resource: missing required `uri` argument"
                )))
            })?
            .to_string();

        let server_id = args
            .get("server_id")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        Ok(message::tool_call::Tool::ReadMcpResource(
            message::tool_call::ReadMcpResource { uri, server_id },
        ))
    }

    fn encode_call_args(&self, tool: &message::tool_call::Tool) -> Value {
        if let message::tool_call::Tool::ReadMcpResource(rmr) = tool {
            json!({ "uri": rmr.uri })
        } else {
            Value::Object(Default::default())
        }
    }

    fn encode_result_text(
        &self,
        result: &input::tool_call_result::Result,
    ) -> std::result::Result<String, Arc<AIApiError>> {
        if let input::tool_call_result::Result::ReadMcpResource(rmr_result) = result {
            use warp_multi_agent_api::mcp_resource_content;
            use warp_multi_agent_api::read_mcp_resource_result::Result as Inner;
            match rmr_result.result.as_ref() {
                Some(Inner::Success(success)) => {
                    let parts: Vec<String> = success
                        .contents
                        .iter()
                        .filter_map(|c| match c.content_type.as_ref()? {
                            mcp_resource_content::ContentType::Text(t) => {
                                Some(t.content.clone())
                            }
                            mcp_resource_content::ContentType::Binary(_) => None,
                        })
                        .collect();
                    Ok(parts.join("\n"))
                }
                Some(Inner::Error(e)) => Ok(format!("error: {}", e.message)),
                None => Ok("(no result)".to_string()),
            }
        } else {
            Err(Arc::new(AIApiError::Other(anyhow::anyhow!(
                "read_mcp_resource: result variant mismatch"
            ))))
        }
    }

    fn matches_proto_call(&self, tool: &message::tool_call::Tool) -> bool {
        matches!(tool, message::tool_call::Tool::ReadMcpResource(_))
    }

    fn matches_proto_result(&self, result: &input::tool_call_result::Result) -> bool {
        matches!(result, input::tool_call_result::Result::ReadMcpResource(_))
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decode_read_files_minimal() {
        let args = json!({ "files": [{ "name": "src/main.rs" }] });
        let tool = READ_FILES.decode_call_args(args).expect("decode");
        match tool {
            message::tool_call::Tool::ReadFiles(rf) => {
                assert_eq!(rf.files.len(), 1);
                assert_eq!(rf.files[0].name, "src/main.rs");
            }
            _ => panic!("expected ReadFiles variant"),
        }
    }

    #[test]
    fn decode_read_documents_minimal() {
        let args = json!({ "documents": [{ "document_id": "doc-123" }] });
        let tool = READ_DOCUMENTS.decode_call_args(args).expect("decode");
        match tool {
            message::tool_call::Tool::ReadDocuments(rd) => {
                assert_eq!(rd.documents.len(), 1);
                assert_eq!(rd.documents[0].document_id, "doc-123");
            }
            _ => panic!("expected ReadDocuments variant"),
        }
    }

    #[test]
    fn decode_grep_minimal() {
        let args = json!({ "queries": ["fn main"], "path": "src/" });
        let tool = GREP.decode_call_args(args).expect("decode");
        match tool {
            message::tool_call::Tool::Grep(g) => {
                assert_eq!(g.queries, vec!["fn main"]);
                assert_eq!(g.path, "src/");
            }
            _ => panic!("expected Grep variant"),
        }
    }

    #[test]
    fn decode_file_glob_v2_minimal() {
        let args = json!({ "patterns": ["*.rs"], "search_dir": "src/" });
        let tool = FILE_GLOB_V2.decode_call_args(args).expect("decode");
        match tool {
            message::tool_call::Tool::FileGlobV2(fg) => {
                assert_eq!(fg.patterns, vec!["*.rs"]);
                assert_eq!(fg.search_dir, "src/");
            }
            _ => panic!("expected FileGlobV2 variant"),
        }
    }

    #[test]
    fn decode_read_shell_command_output_minimal() {
        let args = json!({ "command_id": "cmd-abc" });
        let tool = READ_SHELL_COMMAND_OUTPUT
            .decode_call_args(args)
            .expect("decode");
        match tool {
            message::tool_call::Tool::ReadShellCommandOutput(rsco) => {
                assert_eq!(rsco.command_id, "cmd-abc");
                assert!(rsco.delay.is_none());
            }
            _ => panic!("expected ReadShellCommandOutput variant"),
        }
    }

    #[test]
    fn decode_read_skill_minimal() {
        let args = json!({ "name": "my-skill" });
        let tool = READ_SKILL.decode_call_args(args).expect("decode");
        match tool {
            message::tool_call::Tool::ReadSkill(rs) => {
                assert_eq!(rs.name, "my-skill");
                assert!(rs.skill_reference.is_none());
            }
            _ => panic!("expected ReadSkill variant"),
        }
    }

    #[test]
    fn decode_read_mcp_resource_minimal() {
        let args = json!({ "uri": "mcp://server/resource" });
        let tool = READ_MCP_RESOURCE.decode_call_args(args).expect("decode");
        match tool {
            message::tool_call::Tool::ReadMcpResource(rmr) => {
                assert_eq!(rmr.uri, "mcp://server/resource");
            }
            _ => panic!("expected ReadMcpResource variant"),
        }
    }
}

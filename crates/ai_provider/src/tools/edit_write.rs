//! Edit/write tools: ApplyFileDiffs, EditDocuments, CreateDocuments,
//! WriteToLongRunningShellCommand.

use serde_json::{json, Value};
use std::sync::Arc;
use warp_multi_agent_api::message;
use warp_multi_agent_api::request::input;

use crate::tools::ToolDefinition;
use crate::AIApiError;

pub static APPLY_FILE_DIFFS: ApplyFileDiffsTool = ApplyFileDiffsTool;
pub static EDIT_DOCUMENTS: EditDocumentsTool = EditDocumentsTool;
pub static CREATE_DOCUMENTS: CreateDocumentsTool = CreateDocumentsTool;
pub static WRITE_TO_LONG_RUNNING_SHELL_COMMAND: WriteToLongRunningShellCommandTool =
    WriteToLongRunningShellCommandTool;

// ---------------------------------------------------------------------------
// 8a. ApplyFileDiffs
// ---------------------------------------------------------------------------

pub struct ApplyFileDiffsTool;

impl ToolDefinition for ApplyFileDiffsTool {
    fn name(&self) -> &'static str {
        "apply_file_diffs"
    }

    fn description(&self) -> &'static str {
        "Edit, create, or delete files via search/replace diffs."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "summary": {
                    "type": "string",
                    "description": "A short description of all the changes being made."
                },
                "diffs": {
                    "type": "array",
                    "description": "Search/replace diffs to apply to existing files.",
                    "items": {
                        "type": "object",
                        "properties": {
                            "file_path": {
                                "type": "string",
                                "description": "Path of the file to edit."
                            },
                            "search": {
                                "type": "string",
                                "description": "The exact content to find and replace."
                            },
                            "replace": {
                                "type": "string",
                                "description": "The content that replaces the search string."
                            }
                        },
                        "required": ["file_path", "search", "replace"]
                    }
                },
                "new_files": {
                    "type": "array",
                    "description": "New files to create.",
                    "items": {
                        "type": "object",
                        "properties": {
                            "file_path": {
                                "type": "string",
                                "description": "Path of the new file."
                            },
                            "content": {
                                "type": "string",
                                "description": "Contents of the new file."
                            }
                        },
                        "required": ["file_path", "content"]
                    }
                },
                "deleted_files": {
                    "type": "array",
                    "description": "Files to delete.",
                    "items": {
                        "type": "object",
                        "properties": {
                            "file_path": {
                                "type": "string",
                                "description": "Path of the file to delete."
                            }
                        },
                        "required": ["file_path"]
                    }
                }
            },
            "required": ["summary"]
        })
    }

    fn decode_call_args(
        &self,
        args: Value,
    ) -> std::result::Result<message::tool_call::Tool, Arc<AIApiError>> {
        let summary = args
            .get("summary")
            .and_then(|v| v.as_str())
            .ok_or_else(|| {
                Arc::new(AIApiError::Other(anyhow::anyhow!(
                    "apply_file_diffs: missing required `summary` argument"
                )))
            })?
            .to_string();

        let diffs: Vec<message::tool_call::apply_file_diffs::FileDiff> = args
            .get("diffs")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|d| {
                        let file_path = d.get("file_path")?.as_str()?.to_string();
                        let search = d.get("search")?.as_str()?.to_string();
                        let replace = d.get("replace")?.as_str()?.to_string();
                        Some(message::tool_call::apply_file_diffs::FileDiff {
                            file_path,
                            search,
                            replace,
                        })
                    })
                    .collect()
            })
            .unwrap_or_default();

        let new_files: Vec<message::tool_call::apply_file_diffs::NewFile> = args
            .get("new_files")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|f| {
                        let file_path = f.get("file_path")?.as_str()?.to_string();
                        let content = f.get("content")?.as_str()?.to_string();
                        Some(message::tool_call::apply_file_diffs::NewFile {
                            file_path,
                            content,
                        })
                    })
                    .collect()
            })
            .unwrap_or_default();

        let deleted_files: Vec<message::tool_call::apply_file_diffs::DeleteFile> = args
            .get("deleted_files")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|f| {
                        let file_path = f.get("file_path")?.as_str()?.to_string();
                        Some(message::tool_call::apply_file_diffs::DeleteFile { file_path })
                    })
                    .collect()
            })
            .unwrap_or_default();

        Ok(message::tool_call::Tool::ApplyFileDiffs(
            message::tool_call::ApplyFileDiffs {
                summary,
                diffs,
                new_files,
                deleted_files,
                ..Default::default()
            },
        ))
    }

    fn encode_call_args(&self, tool: &message::tool_call::Tool) -> Value {
        if let message::tool_call::Tool::ApplyFileDiffs(afd) = tool {
            json!({
                "summary": afd.summary,
                "diffs": afd.diffs.iter().map(|d| json!({
                    "file_path": d.file_path,
                    "search": d.search,
                    "replace": d.replace,
                })).collect::<Vec<_>>(),
            })
        } else {
            Value::Object(Default::default())
        }
    }

    fn encode_result_text(
        &self,
        result: &input::tool_call_result::Result,
    ) -> std::result::Result<String, Arc<AIApiError>> {
        if let input::tool_call_result::Result::ApplyFileDiffs(afd_result) = result {
            use warp_multi_agent_api::apply_file_diffs_result::Result as Inner;
            match afd_result.result.as_ref() {
                Some(Inner::Success(success)) => {
                    let updated: Vec<String> = success
                        .updated_files_v2
                        .iter()
                        .filter_map(|u| u.file.as_ref().map(|f| f.file_path.clone()))
                        .collect();
                    let deleted: Vec<&str> = success
                        .deleted_files
                        .iter()
                        .map(|d| d.file_path.as_str())
                        .collect();
                    Ok(format!(
                        "updated_files: {}\ndeleted_files: {}",
                        updated.join(", "),
                        deleted.join(", ")
                    ))
                }
                Some(Inner::Error(e)) => Ok(format!("error: {}", e.message)),
                None => Ok("(no result)".to_string()),
            }
        } else {
            Err(Arc::new(AIApiError::Other(anyhow::anyhow!(
                "apply_file_diffs: result variant mismatch"
            ))))
        }
    }

    fn matches_proto_call(&self, tool: &message::tool_call::Tool) -> bool {
        matches!(tool, message::tool_call::Tool::ApplyFileDiffs(_))
    }

    fn matches_proto_result(&self, result: &input::tool_call_result::Result) -> bool {
        matches!(result, input::tool_call_result::Result::ApplyFileDiffs(_))
    }
}

// ---------------------------------------------------------------------------
// 8b. EditDocuments
// ---------------------------------------------------------------------------

pub struct EditDocumentsTool;

impl ToolDefinition for EditDocumentsTool {
    fn name(&self) -> &'static str {
        "edit_documents"
    }

    fn description(&self) -> &'static str {
        "Edit Warp Drive documents via search/replace."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "diffs": {
                    "type": "array",
                    "description": "Search/replace diffs to apply to Warp Drive documents.",
                    "items": {
                        "type": "object",
                        "properties": {
                            "document_id": {
                                "type": "string",
                                "description": "The unique identifier of the document to edit."
                            },
                            "search": {
                                "type": "string",
                                "description": "The exact content to find and replace."
                            },
                            "replace": {
                                "type": "string",
                                "description": "The content that replaces the search string."
                            }
                        },
                        "required": ["document_id", "search", "replace"]
                    }
                }
            },
            "required": ["diffs"]
        })
    }

    fn decode_call_args(
        &self,
        args: Value,
    ) -> std::result::Result<message::tool_call::Tool, Arc<AIApiError>> {
        let diffs_val = args
            .get("diffs")
            .and_then(|v| v.as_array())
            .ok_or_else(|| {
                Arc::new(AIApiError::Other(anyhow::anyhow!(
                    "edit_documents: missing required `diffs` argument"
                )))
            })?;

        let diffs: Vec<message::tool_call::edit_documents::DocumentDiff> = diffs_val
            .iter()
            .filter_map(|d| {
                let document_id = d.get("document_id")?.as_str()?.to_string();
                let search = d.get("search")?.as_str()?.to_string();
                let replace = d.get("replace")?.as_str()?.to_string();
                Some(message::tool_call::edit_documents::DocumentDiff {
                    document_id,
                    search,
                    replace,
                })
            })
            .collect();

        Ok(message::tool_call::Tool::EditDocuments(
            message::tool_call::EditDocuments { diffs },
        ))
    }

    fn encode_call_args(&self, tool: &message::tool_call::Tool) -> Value {
        if let message::tool_call::Tool::EditDocuments(ed) = tool {
            json!({
                "diffs": ed.diffs.iter().map(|d| json!({
                    "document_id": d.document_id,
                    "search": d.search,
                    "replace": d.replace,
                })).collect::<Vec<_>>(),
            })
        } else {
            Value::Object(Default::default())
        }
    }

    fn encode_result_text(
        &self,
        result: &input::tool_call_result::Result,
    ) -> std::result::Result<String, Arc<AIApiError>> {
        if let input::tool_call_result::Result::EditDocuments(ed_result) = result {
            use warp_multi_agent_api::edit_documents_result::Result as Inner;
            match ed_result.result.as_ref() {
                Some(Inner::Success(success)) => {
                    let ids: Vec<&str> = success
                        .updated_documents
                        .iter()
                        .map(|d| d.document_id.as_str())
                        .collect();
                    Ok(format!("updated_documents: {}", ids.join(", ")))
                }
                Some(Inner::Error(e)) => Ok(format!("error: {}", e.message)),
                None => Ok("(no result)".to_string()),
            }
        } else {
            Err(Arc::new(AIApiError::Other(anyhow::anyhow!(
                "edit_documents: result variant mismatch"
            ))))
        }
    }

    fn matches_proto_call(&self, tool: &message::tool_call::Tool) -> bool {
        matches!(tool, message::tool_call::Tool::EditDocuments(_))
    }

    fn matches_proto_result(&self, result: &input::tool_call_result::Result) -> bool {
        matches!(result, input::tool_call_result::Result::EditDocuments(_))
    }
}

// ---------------------------------------------------------------------------
// 8c. CreateDocuments
// ---------------------------------------------------------------------------

pub struct CreateDocumentsTool;

impl ToolDefinition for CreateDocumentsTool {
    fn name(&self) -> &'static str {
        "create_documents"
    }

    fn description(&self) -> &'static str {
        "Create new Warp Drive documents."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "new_documents": {
                    "type": "array",
                    "description": "Documents to create.",
                    "items": {
                        "type": "object",
                        "properties": {
                            "title": {
                                "type": "string",
                                "description": "The title of the new document."
                            },
                            "content": {
                                "type": "string",
                                "description": "The content of the new document."
                            }
                        },
                        "required": ["title", "content"]
                    }
                }
            },
            "required": ["new_documents"]
        })
    }

    fn decode_call_args(
        &self,
        args: Value,
    ) -> std::result::Result<message::tool_call::Tool, Arc<AIApiError>> {
        let docs_val = args
            .get("new_documents")
            .and_then(|v| v.as_array())
            .ok_or_else(|| {
                Arc::new(AIApiError::Other(anyhow::anyhow!(
                    "create_documents: missing required `new_documents` argument"
                )))
            })?;

        let new_documents: Vec<message::tool_call::create_documents::NewDocument> = docs_val
            .iter()
            .filter_map(|d| {
                let title = d.get("title")?.as_str()?.to_string();
                let content = d.get("content")?.as_str()?.to_string();
                Some(message::tool_call::create_documents::NewDocument {
                    title,
                    content,
                })
            })
            .collect();

        Ok(message::tool_call::Tool::CreateDocuments(
            message::tool_call::CreateDocuments { new_documents },
        ))
    }

    fn encode_call_args(&self, tool: &message::tool_call::Tool) -> Value {
        if let message::tool_call::Tool::CreateDocuments(cd) = tool {
            json!({
                "new_documents": cd.new_documents.iter().map(|d| json!({
                    "title": d.title,
                    "content": d.content,
                })).collect::<Vec<_>>(),
            })
        } else {
            Value::Object(Default::default())
        }
    }

    fn encode_result_text(
        &self,
        result: &input::tool_call_result::Result,
    ) -> std::result::Result<String, Arc<AIApiError>> {
        if let input::tool_call_result::Result::CreateDocuments(cd_result) = result {
            use warp_multi_agent_api::create_documents_result::Result as Inner;
            match cd_result.result.as_ref() {
                Some(Inner::Success(success)) => {
                    let ids: Vec<&str> = success
                        .created_documents
                        .iter()
                        .map(|d| d.document_id.as_str())
                        .collect();
                    Ok(format!("created_documents: {}", ids.join(", ")))
                }
                Some(Inner::Error(e)) => Ok(format!("error: {}", e.message)),
                None => Ok("(no result)".to_string()),
            }
        } else {
            Err(Arc::new(AIApiError::Other(anyhow::anyhow!(
                "create_documents: result variant mismatch"
            ))))
        }
    }

    fn matches_proto_call(&self, tool: &message::tool_call::Tool) -> bool {
        matches!(tool, message::tool_call::Tool::CreateDocuments(_))
    }

    fn matches_proto_result(&self, result: &input::tool_call_result::Result) -> bool {
        matches!(result, input::tool_call_result::Result::CreateDocuments(_))
    }
}

// ---------------------------------------------------------------------------
// 8d. WriteToLongRunningShellCommand
// ---------------------------------------------------------------------------

pub struct WriteToLongRunningShellCommandTool;

impl ToolDefinition for WriteToLongRunningShellCommandTool {
    fn name(&self) -> &'static str {
        "write_to_long_running_shell_command"
    }

    fn description(&self) -> &'static str {
        "Send input to a long-running shell command (REPL or interactive process)."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "command_id": {
                    "type": "string",
                    "description": "The ID of the long-running command to write to."
                },
                "input": {
                    "type": "string",
                    "description": "The text to send to the running process."
                },
                "mode": {
                    "type": "string",
                    "enum": ["raw", "line", "block"],
                    "description": "How to write the input: raw bytes, a single line (with enter), or a block (with bracketed paste)."
                }
            },
            "required": ["command_id", "input"]
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
                    "write_to_long_running_shell_command: missing required `command_id` argument"
                )))
            })?
            .to_string();

        let input_str = args
            .get("input")
            .and_then(|v| v.as_str())
            .ok_or_else(|| {
                Arc::new(AIApiError::Other(anyhow::anyhow!(
                    "write_to_long_running_shell_command: missing required `input` argument"
                )))
            })?;

        let input: Vec<u8> = input_str.as_bytes().to_vec();

        use message::tool_call::write_to_long_running_shell_command::Mode;
        use message::tool_call::write_to_long_running_shell_command::mode::Mode as ModeVariant;

        let mode = match args.get("mode").and_then(|v| v.as_str()) {
            Some("raw") => Some(Mode {
                mode: Some(ModeVariant::Raw(())),
            }),
            Some("line") => Some(Mode {
                mode: Some(ModeVariant::Line(())),
            }),
            Some("block") => Some(Mode {
                mode: Some(ModeVariant::Block(())),
            }),
            _ => None,
        };

        Ok(message::tool_call::Tool::WriteToLongRunningShellCommand(
            message::tool_call::WriteToLongRunningShellCommand {
                command_id,
                input,
                mode,
            },
        ))
    }

    fn encode_call_args(&self, tool: &message::tool_call::Tool) -> Value {
        if let message::tool_call::Tool::WriteToLongRunningShellCommand(w) = tool {
            json!({
                "command_id": w.command_id,
                "input": String::from_utf8_lossy(&w.input),
            })
        } else {
            Value::Object(Default::default())
        }
    }

    fn encode_result_text(
        &self,
        result: &input::tool_call_result::Result,
    ) -> std::result::Result<String, Arc<AIApiError>> {
        if let input::tool_call_result::Result::WriteToLongRunningShellCommand(w_result) = result {
            use warp_multi_agent_api::write_to_long_running_shell_command_result::Result as Inner;
            match w_result.result.as_ref() {
                Some(Inner::CommandFinished(finished)) => Ok(format!(
                    "exit_code: {}\noutput:\n{}",
                    finished.exit_code, finished.output
                )),
                Some(Inner::LongRunningCommandSnapshot(snap)) => Ok(format!(
                    "(long-running snapshot, command_id: {})\n{}",
                    snap.command_id, snap.output
                )),
                Some(Inner::Error(_)) => Ok("error: command not found".to_string()),
                None => Ok("(no result)".to_string()),
            }
        } else {
            Err(Arc::new(AIApiError::Other(anyhow::anyhow!(
                "write_to_long_running_shell_command: result variant mismatch"
            ))))
        }
    }

    fn matches_proto_call(&self, tool: &message::tool_call::Tool) -> bool {
        matches!(
            tool,
            message::tool_call::Tool::WriteToLongRunningShellCommand(_)
        )
    }

    fn matches_proto_result(&self, result: &input::tool_call_result::Result) -> bool {
        matches!(
            result,
            input::tool_call_result::Result::WriteToLongRunningShellCommand(_)
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
    fn decode_apply_file_diffs_minimal() {
        let args = json!({
            "summary": "Fix a bug in main.rs",
            "diffs": [
                { "file_path": "src/main.rs", "search": "old text", "replace": "new text" }
            ]
        });
        let tool = APPLY_FILE_DIFFS.decode_call_args(args).expect("decode");
        match tool {
            message::tool_call::Tool::ApplyFileDiffs(afd) => {
                assert_eq!(afd.summary, "Fix a bug in main.rs");
                assert_eq!(afd.diffs.len(), 1);
                assert_eq!(afd.diffs[0].file_path, "src/main.rs");
                assert_eq!(afd.diffs[0].search, "old text");
                assert_eq!(afd.diffs[0].replace, "new text");
            }
            _ => panic!("expected ApplyFileDiffs variant"),
        }
    }

    #[test]
    fn decode_edit_documents_minimal() {
        let args = json!({
            "diffs": [
                { "document_id": "doc-456", "search": "old", "replace": "new" }
            ]
        });
        let tool = EDIT_DOCUMENTS.decode_call_args(args).expect("decode");
        match tool {
            message::tool_call::Tool::EditDocuments(ed) => {
                assert_eq!(ed.diffs.len(), 1);
                assert_eq!(ed.diffs[0].document_id, "doc-456");
                assert_eq!(ed.diffs[0].search, "old");
                assert_eq!(ed.diffs[0].replace, "new");
            }
            _ => panic!("expected EditDocuments variant"),
        }
    }

    #[test]
    fn decode_create_documents_minimal() {
        let args = json!({
            "new_documents": [
                { "title": "My Doc", "content": "Hello world" }
            ]
        });
        let tool = CREATE_DOCUMENTS.decode_call_args(args).expect("decode");
        match tool {
            message::tool_call::Tool::CreateDocuments(cd) => {
                assert_eq!(cd.new_documents.len(), 1);
                assert_eq!(cd.new_documents[0].title, "My Doc");
                assert_eq!(cd.new_documents[0].content, "Hello world");
            }
            _ => panic!("expected CreateDocuments variant"),
        }
    }

    #[test]
    fn decode_write_to_long_running_shell_command_minimal() {
        let args = json!({
            "command_id": "cmd-xyz",
            "input": "ls -la\n",
            "mode": "line"
        });
        let tool = WRITE_TO_LONG_RUNNING_SHELL_COMMAND
            .decode_call_args(args)
            .expect("decode");
        match tool {
            message::tool_call::Tool::WriteToLongRunningShellCommand(w) => {
                assert_eq!(w.command_id, "cmd-xyz");
                assert_eq!(w.input, b"ls -la\n");
                assert!(w.mode.is_some());
            }
            _ => panic!("expected WriteToLongRunningShellCommand variant"),
        }
    }
}

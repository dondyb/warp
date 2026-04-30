//! RunShellCommand tool — execute a shell command on the user's machine.

use serde_json::{json, Value};
use std::sync::Arc;
use warp_multi_agent_api::message;
use warp_multi_agent_api::message::tool_call::run_shell_command::WaitUntilCompleteValue;
use warp_multi_agent_api::request::input;
use warp_multi_agent_api::RiskCategory;

use crate::tools::ToolDefinition;
use crate::AIApiError;

pub static TOOL: RunShellCommandTool = RunShellCommandTool;

pub struct RunShellCommandTool;

impl ToolDefinition for RunShellCommandTool {
    fn name(&self) -> &'static str {
        "run_shell_command"
    }

    fn description(&self) -> &'static str {
        "Execute a shell command on the user's machine and return its output. \
         Use for any operation that needs the local shell — running scripts, \
         querying system state, building, testing. Set risk_category to \
         indicate the command's destructiveness (read_only, trivial_local_change, \
         nontrivial_local_change, external_change, risky)."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "command": {
                    "type": "string",
                    "description": "The exact shell command to execute."
                },
                "risk_category": {
                    "type": "string",
                    "enum": [
                        "read_only",
                        "trivial_local_change",
                        "nontrivial_local_change",
                        "external_change",
                        "risky"
                    ],
                    "description": "How destructive the command is. Use read_only \
                                    for queries (ls, cat, grep), trivial_local_change \
                                    for safe edits, risky for irreversible operations."
                },
                "uses_pager": {
                    "type": "boolean",
                    "description": "True if the command uses an interactive pager (less, more)."
                },
                "wait_until_complete": {
                    "type": "boolean",
                    "description": "True (default) for short-running commands. False for \
                                    long-running commands where the user wants live output."
                }
            },
            "required": ["command", "risk_category"]
        })
    }

    fn decode_call_args(
        &self,
        args: Value,
    ) -> std::result::Result<message::tool_call::Tool, Arc<AIApiError>> {
        let command = args
            .get("command")
            .and_then(|v| v.as_str())
            .ok_or_else(|| {
                Arc::new(AIApiError::Other(anyhow::anyhow!(
                    "run_shell_command: missing required `command` argument"
                )))
            })?
            .to_string();
        let risk_category = match args.get("risk_category").and_then(|v| v.as_str()) {
            Some("read_only") => RiskCategory::ReadOnly,
            Some("trivial_local_change") => RiskCategory::TrivialLocalChange,
            Some("nontrivial_local_change") => RiskCategory::NontrivialLocalChange,
            Some("external_change") => RiskCategory::ExternalChange,
            Some("risky") => RiskCategory::Risky,
            _ => RiskCategory::Unspecified,
        };
        let uses_pager = args
            .get("uses_pager")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let wait_until_complete = args
            .get("wait_until_complete")
            .and_then(|v| v.as_bool())
            .unwrap_or(true);

        Ok(message::tool_call::Tool::RunShellCommand(
            message::tool_call::RunShellCommand {
                command,
                risk_category: risk_category.into(),
                uses_pager,
                wait_until_complete_value: Some(
                    WaitUntilCompleteValue::WaitUntilComplete(wait_until_complete),
                ),
                ..Default::default()
            },
        ))
    }

    fn encode_call_args(&self, tool: &message::tool_call::Tool) -> Value {
        if let message::tool_call::Tool::RunShellCommand(rsc) = tool {
            json!({
                "command": rsc.command,
                "risk_category": match RiskCategory::try_from(rsc.risk_category)
                    .unwrap_or(RiskCategory::Unspecified)
                {
                    RiskCategory::ReadOnly => "read_only",
                    RiskCategory::TrivialLocalChange => "trivial_local_change",
                    RiskCategory::NontrivialLocalChange => "nontrivial_local_change",
                    RiskCategory::ExternalChange => "external_change",
                    RiskCategory::Risky => "risky",
                    _ => "read_only",
                },
                "uses_pager": rsc.uses_pager,
            })
        } else {
            Value::Object(Default::default())
        }
    }

    fn encode_result_text(
        &self,
        result: &input::tool_call_result::Result,
    ) -> std::result::Result<String, Arc<AIApiError>> {
        if let input::tool_call_result::Result::RunShellCommand(rsc_result) = result {
            // RunShellCommandResult has oneof result with variants:
            //   LongRunningCommandSnapshot, CommandFinished, PermissionDenied
            use warp_multi_agent_api::run_shell_command_result::Result as Inner;
            match rsc_result.result.as_ref() {
                Some(Inner::CommandFinished(finished)) => Ok(format!(
                    "exit_code: {}\noutput:\n{}",
                    finished.exit_code, finished.output
                )),
                Some(Inner::LongRunningCommandSnapshot(snap)) => Ok(format!(
                    "(long-running snapshot, command_id: {})\n{}",
                    snap.command_id, snap.output
                )),
                Some(Inner::PermissionDenied(_)) => {
                    Ok("permission_denied: user did not approve this command".to_string())
                }
                None => Ok("(no result)".to_string()),
            }
        } else {
            Err(Arc::new(AIApiError::Other(anyhow::anyhow!(
                "run_shell_command: result variant mismatch"
            ))))
        }
    }

    fn matches_proto_call(&self, tool: &message::tool_call::Tool) -> bool {
        matches!(tool, message::tool_call::Tool::RunShellCommand(_))
    }

    fn matches_proto_result(&self, result: &input::tool_call_result::Result) -> bool {
        matches!(result, input::tool_call_result::Result::RunShellCommand(_))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schema_has_required_fields() {
        let schema = TOOL.parameters_schema();
        let required = schema.get("required").and_then(|v| v.as_array()).unwrap();
        assert!(required.iter().any(|v| v.as_str() == Some("command")));
        assert!(required.iter().any(|v| v.as_str() == Some("risk_category")));
    }

    #[test]
    fn decode_minimal_call() {
        let args = json!({
            "command": "ls",
            "risk_category": "read_only"
        });
        let tool = TOOL.decode_call_args(args).expect("decode");
        match tool {
            message::tool_call::Tool::RunShellCommand(rsc) => {
                assert_eq!(rsc.command, "ls");
                assert_eq!(rsc.risk_category, RiskCategory::ReadOnly as i32);
            }
            _ => panic!("expected RunShellCommand variant"),
        }
    }

    #[test]
    fn decode_missing_command_errors() {
        let args = json!({"risk_category": "read_only"});
        let err = TOOL.decode_call_args(args).expect_err("err");
        assert!(format!("{err:#}").contains("missing required `command`"));
    }
}

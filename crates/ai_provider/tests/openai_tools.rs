//! Integration tests for OpenAI tool-calling via mockito.
//!
//! Stage 1: Adapter receives a plain chat request. Mockito returns SSE chunks
//!   with a `tool_calls` delta for `run_shell_command`. The adapter must emit
//!   a `ResponseEvent::ClientActions` containing `AddMessagesToTask` with a
//!   `Message::ToolCall { tool_call_id, tool: Some(RunShellCommand { ... }) }`.
//!
//! Stage 2: Adapter receives a follow-up request that includes a
//!   `ToolCallResult` for the same `tool_call_id`. Mockito returns text deltas.
//!   The adapter must (a) have sent an OpenAI `role: tool` message with the
//!   correct `tool_call_id` and `content: "exit_code: 0\noutput:\nfile_a\nfile_b"`
//!   in the request body, and (b) stream the assistant's text response back.

use ai_provider::{AiProvider, OpenAiAdapter, OpenAiConfig};
use futures::StreamExt;
use std::sync::{Arc, Mutex};
use warp_multi_agent_api::{
    request as req, Request, ResponseEvent, ShellCommandFinished,
    RunShellCommandResult,
    client_action, message, response_event,
};

// ── helpers ──────────────────────────────────────────────────────────────────

fn sse_chunk(json: &str) -> String {
    format!("data: {}\n\n", json)
}

/// Build a simple chat request with a single `UserQuery`.
fn make_query_request(query: &str) -> Request {
    Request {
        input: Some(req::Input {
            r#type: Some(req::input::Type::UserInputs(req::input::UserInputs {
                inputs: vec![req::input::user_inputs::UserInput {
                    input: Some(
                        req::input::user_inputs::user_input::Input::UserQuery(
                            req::input::UserQuery {
                                query: query.to_string(),
                                ..Default::default()
                            },
                        ),
                    ),
                }],
            })),
            ..Default::default()
        }),
        ..Default::default()
    }
}

/// Build a follow-up request that contains a `ToolCallResult` for
/// `run_shell_command` with a `CommandFinished` result.
fn make_tool_result_request(
    tool_call_id: &str,
    exit_code: i32,
    output: &str,
) -> Request {
    use warp_multi_agent_api::run_shell_command_result;

    let result = RunShellCommandResult {
        result: Some(run_shell_command_result::Result::CommandFinished(
            ShellCommandFinished {
                exit_code,
                output: output.to_string(),
                command_id: String::new(),
            },
        )),
        ..Default::default()
    };

    Request {
        input: Some(req::Input {
            r#type: Some(req::input::Type::UserInputs(req::input::UserInputs {
                inputs: vec![req::input::user_inputs::UserInput {
                    input: Some(
                        req::input::user_inputs::user_input::Input::ToolCallResult(
                            req::input::ToolCallResult {
                                tool_call_id: tool_call_id.to_string(),
                                result: Some(
                                    req::input::tool_call_result::Result::RunShellCommand(result),
                                ),
                            },
                        ),
                    ),
                }],
            })),
            ..Default::default()
        }),
        ..Default::default()
    }
}

/// Drain all events from the stream and return them.
async fn collect_events(
    mut stream: ai_provider::ResponseEventStream,
) -> Vec<ResponseEvent> {
    let mut events = Vec::new();
    while let Some(item) = stream.next().await {
        events.push(item.expect("stream event should not error"));
    }
    events
}

// ── Stage 1 ──────────────────────────────────────────────────────────────────

/// Stage 1: The adapter receives a plain chat request and the mock server
/// responds with SSE chunks that represent a tool call for `run_shell_command`.
/// We assert that the adapter emits a `ClientActions` event containing an
/// `AddMessagesToTask` action whose message is `Message::ToolCall` with a
/// `RunShellCommand` tool variant.
#[tokio::test]
async fn stage1_tool_call_delta_emits_tool_call_message() {
    let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();

    let mut server = mockito::Server::new_async().await;

    // Simulate OpenAI streaming a tool_call response split across two chunks:
    // chunk 1: role + first part of tool_calls (id + name)
    // chunk 2: arguments payload
    // chunk 3: finish_reason == "tool_calls" (signals the call is complete)
    let tool_call_args = r#"{\"command\":\"ls /tmp\",\"risk_category\":\"read_only\"}"#;
    let body = format!(
        "{}{}{}{}",
        sse_chunk(
            r#"{"choices":[{"delta":{"role":"assistant","tool_calls":[{"index":0,"id":"call_abc123","type":"function","function":{"name":"run_shell_command","arguments":""}}]},"finish_reason":null}]}"#
        ),
        sse_chunk(&format!(
            r#"{{"choices":[{{"delta":{{"tool_calls":[{{"index":0,"function":{{"arguments":"{tool_call_args}"}}}}]}},"finish_reason":null}}]}}"#
        )),
        sse_chunk(
            r#"{"choices":[{"delta":{},"finish_reason":"tool_calls"}]}"#
        ),
        "data: [DONE]\n\n",
    );

    let _m = server
        .mock("POST", "/chat/completions")
        .match_header("authorization", "Bearer sk-test")
        .with_status(200)
        .with_header("content-type", "text/event-stream")
        .with_body(body)
        .create_async()
        .await;

    let adapter = OpenAiAdapter::new(OpenAiConfig {
        endpoint: server.url(),
        api_key: "sk-test".into(),
        model: "gpt-4o-mini".into(),
    });

    let events = collect_events(
        adapter
            .chat_stream(&make_query_request("list files in /tmp"))
            .await
            .expect("chat_stream"),
    )
    .await;

    // Find a ClientActions event that contains AddMessagesToTask with a ToolCall.
    let tool_call_event = events.iter().find(|e| {
        if let Some(response_event::Type::ClientActions(ca)) = e.r#type.as_ref() {
            ca.actions.iter().any(|action| {
                if let Some(client_action::Action::AddMessagesToTask(add)) =
                    action.action.as_ref()
                {
                    add.messages.iter().any(|msg| {
                        matches!(
                            msg.message.as_ref(),
                            Some(message::Message::ToolCall(_))
                        )
                    })
                } else {
                    false
                }
            })
        } else {
            false
        }
    });

    assert!(
        tool_call_event.is_some(),
        "expected a ClientActions event with AddMessagesToTask(ToolCall), events: {events:#?}"
    );

    // Drill into the event and assert the specifics.
    let ca = if let Some(response_event::Type::ClientActions(ca)) =
        tool_call_event.unwrap().r#type.as_ref()
    {
        ca
    } else {
        panic!("expected ClientActions");
    };

    let add_action = ca
        .actions
        .iter()
        .find_map(|a| {
            if let Some(client_action::Action::AddMessagesToTask(add)) = a.action.as_ref() {
                Some(add)
            } else {
                None
            }
        })
        .expect("AddMessagesToTask action");

    let tool_call_msg = add_action
        .messages
        .iter()
        .find_map(|m| {
            if let Some(message::Message::ToolCall(tc)) = m.message.as_ref() {
                Some(tc)
            } else {
                None
            }
        })
        .expect("ToolCall message");

    assert_eq!(
        tool_call_msg.tool_call_id, "call_abc123",
        "tool_call_id should be 'call_abc123'"
    );

    assert!(
        tool_call_msg.tool.is_some(),
        "ToolCall.tool should be Some(RunShellCommand) — registry decoded it"
    );

    match tool_call_msg.tool.as_ref().unwrap() {
        message::tool_call::Tool::RunShellCommand(rsc) => {
            assert_eq!(rsc.command, "ls /tmp");
        }
        other => panic!("expected RunShellCommand, got {other:?}"),
    }
}

// ── Stage 2 ──────────────────────────────────────────────────────────────────

/// Stage 2: The adapter receives a follow-up request that includes a
/// `ToolCallResult`. The mock server captures the outgoing request body so we
/// can assert that it contains a `role: tool` message with the right
/// `tool_call_id` and `content`. The mock server then returns a text-delta SSE
/// stream and we assert the text makes it back.
#[tokio::test]
async fn stage2_tool_result_sent_as_role_tool_and_response_streamed() {
    let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();

    let mut server = mockito::Server::new_async().await;

    // Capture the request body for post-hoc assertion.
    let captured_body: Arc<Mutex<Option<serde_json::Value>>> = Arc::new(Mutex::new(None));
    let captured_body_clone = captured_body.clone();

    let response_body = format!(
        "{}{}{}",
        sse_chunk(r#"{"choices":[{"delta":{"role":"assistant"},"finish_reason":null}]}"#),
        sse_chunk(r#"{"choices":[{"delta":{"content":"Done! Found file_a and file_b."},"finish_reason":"stop"}]}"#),
        "data: [DONE]\n\n",
    );

    let _m = server
        .mock("POST", "/chat/completions")
        .match_header("authorization", "Bearer sk-test")
        .match_request(move |req| {
            // Capture the body for assertion after the stream is consumed.
            if let Ok(body_str) = req.utf8_lossy_body() {
                if let Ok(json) = serde_json::from_str::<serde_json::Value>(&body_str) {
                    *captured_body_clone.lock().unwrap() = Some(json);
                }
            }
            true // Always match — we assert the body separately.
        })
        .with_status(200)
        .with_header("content-type", "text/event-stream")
        .with_body(response_body)
        .create_async()
        .await;

    let adapter = OpenAiAdapter::new(OpenAiConfig {
        endpoint: server.url(),
        api_key: "sk-test".into(),
        model: "gpt-4o-mini".into(),
    });

    // Build a follow-up request: the tool_call_id "call_abc123" has finished
    // with exit_code 0 and output "file_a\nfile_b".
    let follow_up = make_tool_result_request("call_abc123", 0, "file_a\nfile_b");

    let events = collect_events(
        adapter
            .chat_stream(&follow_up)
            .await
            .expect("chat_stream"),
    )
    .await;

    // ── (a) Verify the outgoing request body had a `role: tool` message ──────

    let body_json = captured_body
        .lock()
        .unwrap()
        .clone()
        .expect("request body should have been captured");

    let messages = body_json["messages"]
        .as_array()
        .expect("messages array in request body");

    let tool_msg = messages
        .iter()
        .find(|m| m["role"] == "tool")
        .expect("expected a role:tool message in the outgoing request");

    assert_eq!(
        tool_msg["tool_call_id"], "call_abc123",
        "tool_call_id should match"
    );
    assert_eq!(
        tool_msg["content"], "exit_code: 0\noutput:\nfile_a\nfile_b",
        "content should be the encoded RunShellCommand result"
    );

    // ── (b) Verify the assistant text response was streamed back ─────────────

    let text_events: Vec<_> = events
        .iter()
        .filter_map(|e| {
            if let Some(response_event::Type::ClientActions(ca)) = e.r#type.as_ref() {
                let texts: Vec<String> = ca
                    .actions
                    .iter()
                    .filter_map(|a| {
                        if let Some(client_action::Action::AppendToMessageContent(ap)) =
                            a.action.as_ref()
                        {
                            if let Some(msg) = ap.message.as_ref() {
                                if let Some(message::Message::AgentOutput(out)) =
                                    msg.message.as_ref()
                                {
                                    return Some(out.text.clone());
                                }
                            }
                        }
                        None
                    })
                    .collect();
                if texts.is_empty() { None } else { Some(texts) }
            } else {
                None
            }
        })
        .flatten()
        .collect();

    assert!(
        !text_events.is_empty(),
        "expected at least one text-delta event from the mock text response"
    );

    let full_text = text_events.join("");
    assert!(
        full_text.contains("file_a") || full_text.contains("Done"),
        "response text should contain the assistant reply, got: {full_text:?}"
    );
}

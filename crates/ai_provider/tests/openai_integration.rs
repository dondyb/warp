//! Integration tests for `OpenAiAdapter` using `mockito` to fake the
//! OpenAI Chat Completions endpoint.

use ai_provider::{AiProvider, OpenAiAdapter, OpenAiConfig};
use futures::StreamExt;
use warp_multi_agent_api::{request as req, Request, ResponseEvent, response_event, client_action, message};

fn make_request(query: &str) -> Request {
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
                ..Default::default()
            })),
            ..Default::default()
        }),
        ..Default::default()
    }
}

fn sse_chunk(json: &str) -> String {
    format!("data: {}\n\n", json)
}

#[tokio::test]
async fn happy_path_emits_canonical_transaction_sequence() {
    // Initialize the rustls crypto provider (required by reqwest even for HTTP
    // connections in the test binary; the error is "No provider set").
    let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();

    let mut server = mockito::Server::new_async().await;
    let body = format!(
        "{}{}{}{}",
        sse_chunk(r#"{"choices":[{"delta":{"role":"assistant"},"finish_reason":null}]}"#),
        sse_chunk(r#"{"choices":[{"delta":{"content":"Hello"},"finish_reason":null}]}"#),
        sse_chunk(r#"{"choices":[{"delta":{"content":" world"},"finish_reason":"stop"}]}"#),
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
    let mut stream = adapter
        .chat_stream(&make_request("hi"))
        .await
        .expect("stream");

    // Collect all events.
    let mut events: Vec<ResponseEvent> = Vec::new();
    while let Some(item) = stream.next().await {
        events.push(item.expect("event"));
    }

    // Expected sequence:
    // 0: StreamInit
    // 1: ClientActions [BeginTransaction, CreateTask, AddMessagesToTask]
    // 2: ClientActions [AppendToMessageContent("Hello")]
    // 3: ClientActions [AppendToMessageContent(" world")]
    // 4: ClientActions [CommitTransaction]
    // 5: StreamFinished{Done}
    assert_eq!(events.len(), 6, "events: {events:#?}");

    // Spot-check the structure.
    assert!(matches!(
        events[0].r#type.as_ref().unwrap(),
        response_event::Type::Init(_)
    ));
    assert!(matches!(
        events[5].r#type.as_ref().unwrap(),
        response_event::Type::Finished(_)
    ));

    // The two delta events carry the right text.
    let extract_append_text = |e: &ResponseEvent| -> Option<String> {
        if let response_event::Type::ClientActions(a) = e.r#type.as_ref().unwrap() {
            if let Some(action) = a.actions.first() {
                if let client_action::Action::AppendToMessageContent(ap) =
                    action.action.as_ref().unwrap()
                {
                    if let Some(msg) = ap.message.as_ref() {
                        if let message::Message::AgentOutput(out) =
                            msg.message.as_ref().unwrap()
                        {
                            return Some(out.text.clone());
                        }
                    }
                }
            }
        }
        None
    };
    assert_eq!(extract_append_text(&events[2]), Some("Hello".into()));
    assert_eq!(extract_append_text(&events[3]), Some(" world".into()));
}

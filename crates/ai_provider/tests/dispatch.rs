use ai_provider::{AiProvider, ResponseEventStream};

struct StubProvider;

#[async_trait::async_trait]
impl AiProvider for StubProvider {
    async fn chat_stream(
        &self,
        _request: &warp_multi_agent_api::Request,
    ) -> std::result::Result<ResponseEventStream, std::sync::Arc<ai_provider::AIApiError>> {
        Ok(Box::pin(futures::stream::iter(std::iter::empty())))
    }
}

#[tokio::test]
async fn stub_provider_returns_empty_stream() {
    let provider: Box<dyn AiProvider> = Box::new(StubProvider);
    let req = warp_multi_agent_api::Request::default();
    let mut stream = provider.chat_stream(&req).await.expect("stream");
    use futures::StreamExt;
    assert!(stream.next().await.is_none());
}

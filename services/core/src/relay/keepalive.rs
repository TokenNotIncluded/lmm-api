//! Anthropic Messages streaming keepalive.
//!
//! Idle proxies drop a Claude stream that stays silent during routing, billing
//! reservation, connect retries, or a slow upstream read. This task emits the
//! SSE comment `: PING` (the same comment frame used by the upstream new-api
//! keepalive) after each full interval. It never writes a partial comment, and
//! callers join it before any business byte so the two writers cannot interleave.
//!
//! The first comment waits one interval. Committing `200 text/event-stream` is
//! deferred until that comment, so a fast stream keeps a normal status line and
//! a later failure can still use an HTTP error. After the comment has been
//! committed, failures stay on this stream as a Claude `error` event and the
//! settlement report remains a failure.

use super::{Cancellation, Phase, RelayError, ResponseHead};
use axum::body::Bytes;
use std::{
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::{
    sync::{mpsc, oneshot, watch},
    task::JoinHandle,
    time::timeout,
};

pub(super) const FRAME: &[u8] = b": PING\n\n";
pub(super) const STREAM_CONTENT_TYPE: &str = "text/event-stream; charset=utf-8";

/// Downstream response head. The keepalive task and the worker race to take it;
/// the first sender commits the HTTP status.
pub(super) type ResponseHeadSlot =
    Arc<Mutex<Option<oneshot::Sender<Result<ResponseHead, RelayError>>>>>;

pub(super) struct Done {
    pub error: Option<RelayError>,
    pub emitted_bytes: usize,
}

pub(super) struct KeepAlive {
    stop: watch::Sender<bool>,
    task: Option<JoinHandle<Done>>,
}

impl Drop for KeepAlive {
    fn drop(&mut self) {
        let _ = self.stop.send(true);
        if let Some(task) = self.task.take() {
            task.abort();
        }
    }
}

impl KeepAlive {
    pub(super) fn spawn(
        interval: Duration,
        write_timeout: Duration,
        chunk_bytes: usize,
        cancel: Cancellation,
        head: ResponseHeadSlot,
        sender: mpsc::Sender<Result<Bytes, RelayError>>,
    ) -> Self {
        let (stop, mut stopped) = watch::channel(false);
        let task = Some(tokio::spawn(async move {
            let mut emitted_bytes = 0usize;
            loop {
                tokio::select! {
                    biased;
                    result = stopped.changed() => {
                        if result.is_err() || *stopped.borrow() {
                            return Done { error: None, emitted_bytes };
                        }
                    }
                    _ = cancel.cancelled() => {
                        return Done { error: Some(RelayError::Cancelled), emitted_bytes };
                    }
                    _ = tokio::time::sleep(interval) => {
                        if *stopped.borrow() || cancel.is_cancelled() {
                            let error = cancel.is_cancelled().then_some(RelayError::Cancelled);
                            return Done { error, emitted_bytes };
                        }
                        if let Err(error) = commit_stream_head(&head) {
                            return Done { error: Some(error), emitted_bytes };
                        }
                        let (sent, error) = send_frame(
                            &sender,
                            &cancel,
                            write_timeout,
                            chunk_bytes,
                        ).await;
                        emitted_bytes = emitted_bytes.saturating_add(sent);
                        if let Some(error) = error {
                            return Done { error: Some(error), emitted_bytes };
                        }
                    }
                }
            }
        }));
        Self { stop, task }
    }

    pub(super) async fn finish(mut self) -> Done {
        let _ = self.stop.send(true);
        let Some(task) = self.task.take() else {
            return Done {
                error: None,
                emitted_bytes: 0,
            };
        };
        match task.await {
            Ok(done) => done,
            Err(_) => Done {
                error: Some(RelayError::Protocol("keepalive failed")),
                emitted_bytes: 0,
            },
        }
    }
}

pub(super) fn commit_stream_head(
    slot: &Mutex<Option<oneshot::Sender<Result<ResponseHead, RelayError>>>>,
) -> Result<(), RelayError> {
    let mut guard = slot
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if let Some(head) = guard.take() {
        head.send(Ok(ResponseHead {
            status: 200,
            content_type: STREAM_CONTENT_TYPE,
        }))
        .map_err(|_| RelayError::Cancelled)?;
    }
    Ok(())
}

/// Write one keepalive comment in `chunk_bytes` pieces. Callers join this task
/// before business writes, and stop is observed only between comments, so a
/// comment is not mixed into a model frame. Cancellation or a write timeout can
/// still end a comment early; that failure is returned instead of model bytes.
async fn send_frame(
    sender: &mpsc::Sender<Result<Bytes, RelayError>>,
    cancel: &Cancellation,
    write_timeout: Duration,
    chunk_bytes: usize,
) -> (usize, Option<RelayError>) {
    let mut sent = 0usize;
    for chunk in FRAME.chunks(chunk_bytes.max(1)) {
        let write = timeout(write_timeout, async {
            tokio::select! {
                biased;
                _ = cancel.cancelled() => Err(RelayError::Cancelled),
                result = sender.send(Ok(Bytes::copy_from_slice(chunk))) => {
                    result.map_err(|_| RelayError::Cancelled)
                }
            }
        });
        match write.await {
            Ok(Ok(())) => sent = sent.saturating_add(chunk.len()),
            Ok(Err(error)) => return (sent, Some(error)),
            Err(_) => return (sent, Some(RelayError::Timeout(Phase::Write))),
        }
    }
    (sent, None)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::relay::{
        Billing, ClientProtocol, Limits, Outcome, OutputCheckpoint, Relay, RelayFuture, Report,
        Request, RequestContext, Reservation, Route, RouteProvider, Settlement, UpstreamProtocol,
        sse,
    };
    use axum::{
        Router,
        body::Body,
        http::{StatusCode, header},
        routing::post,
    };
    use serde_json::json;
    use std::{
        sync::{
            Arc, Mutex,
            atomic::{AtomicUsize, Ordering},
        },
        time::Duration,
    };
    use tokio::{
        net::TcpListener,
        sync::{oneshot, watch},
        time::timeout,
    };
    use tokio_stream::wrappers::ReceiverStream;

    fn frames(raw: &str) -> Vec<(Option<String>, String)> {
        let mut parser = sse::Parser::new(1024 * 1024);
        let mut out = Vec::new();
        for byte in raw.as_bytes() {
            if let Some(frame) = parser.feed(*byte).unwrap() {
                out.push((frame.event, frame.data));
            }
        }
        parser.finish().unwrap();
        out
    }

    #[test]
    fn ping_comments_between_events_keep_the_same_frames() {
        let business = "event: message_start\ndata: {\"type\":\"message_start\"}\n\nevent: content_block_delta\ndata: {\"text\":\"鲸鱼\"}\n\n";
        let wrapped = format!("{FRAME_TEXT}{business}{FRAME_TEXT}");
        assert_eq!(frames(business), frames(&wrapped));
        let between = business.replacen("\n\n", "\n\n: PING\n\n", 1);
        assert_eq!(frames(business), frames(&between));
        assert_eq!(frames(&between).len(), 2);
    }

    const FRAME_TEXT: &str = ": PING\n\n";

    #[test]
    fn sub_millisecond_ping_interval_is_rejected() {
        let limits = Limits {
            ping_interval: Duration::from_micros(500),
            ..Limits::default()
        };
        assert!(limits.validate().is_err());
        assert!(Limits::default().validate().is_ok());
        assert_eq!(Limits::default().ping_interval, Duration::from_secs(10));
    }

    #[tokio::test]
    async fn first_ping_waits_one_interval_and_stop_prevents_a_partial_mix() {
        let interval = Duration::from_millis(80);
        let (head_tx, mut head_rx) = oneshot::channel();
        let head = Arc::new(Mutex::new(Some(head_tx)));
        let (sender, mut receiver) = mpsc::channel(8);
        let cancel = Cancellation::default();
        let keepalive = KeepAlive::spawn(
            interval,
            Duration::from_secs(1),
            3,
            cancel,
            Arc::clone(&head),
            sender.clone(),
        );
        assert!(
            timeout(Duration::from_millis(30), &mut head_rx)
                .await
                .is_err(),
            "headers must stay uncommitted until the first interval"
        );
        let head = timeout(Duration::from_secs(1), &mut head_rx)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        assert_eq!(head.status, 200);
        assert_eq!(head.content_type, STREAM_CONTENT_TYPE);
        let mut bytes = Vec::new();
        while bytes.len() < FRAME.len() {
            let chunk = timeout(Duration::from_secs(1), receiver.recv())
                .await
                .unwrap()
                .unwrap()
                .unwrap();
            assert!(chunk.len() <= 3);
            bytes.extend_from_slice(&chunk);
        }
        assert_eq!(bytes, FRAME);
        let done = keepalive.finish().await;
        assert_eq!(done.error, None);
        assert_eq!(done.emitted_bytes, FRAME.len());
        sender
            .send(Ok(Bytes::from_static(b"event: message_start\n\n")))
            .await
            .unwrap();
        let mut rest = Vec::new();
        while let Ok(Some(chunk)) = timeout(Duration::from_millis(40), receiver.recv()).await {
            rest.extend_from_slice(&chunk.unwrap());
        }
        assert_eq!(rest, b"event: message_start\n\n");
    }

    #[tokio::test]
    async fn stop_before_the_interval_emits_nothing() {
        let (head_tx, head_rx) = oneshot::channel();
        let head = Arc::new(Mutex::new(Some(head_tx)));
        let (sender, mut receiver) = mpsc::channel(2);
        let keepalive = KeepAlive::spawn(
            Duration::from_secs(30),
            Duration::from_secs(1),
            16,
            Cancellation::default(),
            head,
            sender,
        );
        let done = keepalive.finish().await;
        assert_eq!(done.error, None);
        assert_eq!(done.emitted_bytes, 0);
        assert!(
            head_rx.await.is_err(),
            "stopped keepalive must not commit headers"
        );
        assert!(receiver.recv().await.is_none());
    }

    fn context() -> RequestContext {
        static IDS: AtomicUsize = AtomicUsize::new(1);
        RequestContext {
            request_id: format!("ka{}", IDS.fetch_add(1, Ordering::Relaxed)),
            actor_user_id: 7,
            key_id: Some(9),
            oauth_source: None,
        }
    }

    fn parsed(client: ClientProtocol, limits: &Limits) -> Request {
        let body = match client {
            ClientProtocol::Anthropic => json!({
                "model": "claude",
                "max_tokens": 32,
                "stream": true,
                "messages": [{"role": "user", "content": "hello"}]
            }),
            ClientProtocol::Chat => json!({
                "model": "claude",
                "stream": true,
                "messages": [{"role": "user", "content": "hello"}]
            }),
            _ => unreachable!(),
        };
        Request::parse(client, &serde_json::to_vec(&body).unwrap(), limits).unwrap()
    }

    fn anthropic_sse() -> Vec<u8> {
        let events = [
            json!({"type":"message_start","message":{"id":"upstream-ant","type":"message","role":"assistant","content":[],"usage":{"input_tokens":2,"output_tokens":0,"cache_read_input_tokens":0,"cache_creation_input_tokens":0}}}),
            json!({"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}}),
            json!({"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"upstream-answer"}}),
            json!({"type":"content_block_stop","index":0}),
            json!({"type":"message_delta","delta":{"stop_reason":"end_turn"},"usage":{"output_tokens":3,"cache_read_input_tokens":0,"cache_creation_input_tokens":0}}),
            json!({"type":"message_stop"}),
        ];
        let mut data = String::new();
        for value in events {
            data.push_str(&format!(
                "event: {}\ndata: {value}\n\n",
                value["type"].as_str().unwrap()
            ));
        }
        data.into_bytes()
    }

    fn chat_sse() -> Vec<u8> {
        let events = [
            json!({"id":"upstream-chat","choices":[{"index":0,"delta":{"role":"assistant","content":"upstream-answer"},"finish_reason":null}]}),
            json!({"id":"upstream-chat","choices":[{"index":0,"delta":{},"finish_reason":"stop"}],"usage":{"prompt_tokens":2,"completion_tokens":3}}),
        ];
        let mut data = String::new();
        for value in events {
            data.push_str(&format!("data: {value}\n\n"));
        }
        data.push_str("data: [DONE]\n\n");
        data.into_bytes()
    }

    struct FixedRoute(Route);
    impl RouteProvider for FixedRoute {
        fn select<'a>(&'a self, _: &'a RequestContext, _: &'a Request) -> RelayFuture<'a, Route> {
            let route = self.0.clone();
            Box::pin(async move { Ok(route) })
        }
    }

    struct Hold {
        release: Mutex<Option<oneshot::Receiver<()>>>,
        reports: Arc<Mutex<Vec<Report>>>,
    }
    impl Billing for Hold {
        fn reserve<'a>(
            &'a self,
            _: &'a RequestContext,
            _: &'a Route,
            _: &'a Request,
        ) -> RelayFuture<'a, Reservation> {
            let release = self
                .release
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .take();
            Box::pin(async move {
                if let Some(release) = release {
                    release.await.map_err(|_| RelayError::BillingUnavailable)?;
                }
                Ok(Reservation { id: "hold".into() })
            })
        }
        fn checkpoint<'a>(
            &'a self,
            _: &'a Reservation,
            _: &'a OutputCheckpoint,
        ) -> RelayFuture<'a, ()> {
            Box::pin(async { Ok(()) })
        }
        fn finalize<'a>(&'a self, _: &'a Reservation, report: &'a Report) -> RelayFuture<'a, ()> {
            let reports = Arc::clone(&self.reports);
            let report = report.clone();
            Box::pin(async move {
                reports
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .push(report);
                Ok(())
            })
        }
    }

    async fn serve(body: Vec<u8>, release: Option<watch::Receiver<bool>>) -> String {
        let app = Router::new().route(
            "/v1/messages",
            post(move || {
                let body = body.clone();
                let release = release.clone();
                async move {
                    let (sender, receiver) = tokio::sync::mpsc::channel(1);
                    tokio::spawn(async move {
                        if let Some(mut release) = release {
                            let _ = release.wait_for(|ready| *ready).await;
                        }
                        let _ = sender
                            .send(Ok::<_, std::io::Error>(Bytes::from(body)))
                            .await;
                    });
                    (
                        StatusCode::OK,
                        [(header::CONTENT_TYPE, "text/event-stream")],
                        Body::from_stream(ReceiverStream::new(receiver)),
                    )
                }
            }),
        );
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        format!("http://{address}/v1/messages")
    }

    fn route(url: &str, protocol: UpstreamProtocol) -> Route {
        Route::new(
            "route-1".into(),
            protocol,
            url,
            "claude".into(),
            "synthetic-only",
            1,
        )
        .unwrap()
    }

    fn relay(
        url: &str,
        protocol: UpstreamProtocol,
        limits: Limits,
        release: Option<oneshot::Receiver<()>>,
    ) -> (Relay, Arc<Mutex<Vec<Report>>>) {
        let reports = Arc::new(Mutex::new(Vec::new()));
        let billing = Hold {
            release: Mutex::new(release),
            reports: Arc::clone(&reports),
        };
        let relay = Relay::new(
            Arc::new(FixedRoute(route(url, protocol))),
            Arc::new(billing),
            limits,
        )
        .unwrap();
        (relay, reports)
    }

    async fn read_until(body: &mut crate::relay::RelayBody, bytes: &mut Vec<u8>, needle: &[u8]) {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
        while !bytes.windows(needle.len()).any(|window| window == needle) {
            let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
            let chunk = timeout(remaining, body.next_chunk())
                .await
                .expect("timed out waiting for keepalive bytes")
                .expect("stream ended before keepalive")
                .unwrap();
            bytes.extend_from_slice(&chunk);
        }
    }

    async fn finish_body(body: &mut crate::relay::RelayBody, bytes: &mut Vec<u8>) {
        loop {
            match timeout(Duration::from_secs(2), body.next_chunk()).await {
                Ok(Some(chunk)) => bytes.extend_from_slice(&chunk.unwrap()),
                Ok(None) => return,
                Err(_) => panic!("response body did not finish"),
            }
        }
    }

    #[tokio::test]
    async fn reserve_wait_emits_ping_before_business_frames_and_settles_completed() {
        let (release_tx, release_rx) = oneshot::channel();
        let url = serve(anthropic_sse(), None).await;
        let limits = Limits {
            ping_interval: Duration::from_millis(50),
            hook_timeout: Duration::from_secs(3),
            ..Limits::default()
        };
        let (relay, reports) = relay(
            &url,
            UpstreamProtocol::Anthropic,
            limits.clone(),
            Some(release_rx),
        );
        let mut session = relay
            .start(parsed(ClientProtocol::Anthropic, &limits), context())
            .unwrap();
        let head = timeout(Duration::from_secs(2), &mut session.head)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        assert_eq!(head.status, 200);
        assert_eq!(head.content_type, STREAM_CONTENT_TYPE);
        let mut bytes = Vec::new();
        read_until(&mut session.body, &mut bytes, FRAME).await;
        assert!(bytes.starts_with(FRAME));
        release_tx.send(()).unwrap();
        finish_body(&mut session.body, &mut bytes).await;
        let done = timeout(Duration::from_secs(2), session.completion)
            .await
            .unwrap()
            .unwrap();
        let text = String::from_utf8(bytes).unwrap();
        let stripped = text.replace(FRAME_TEXT, "");
        assert_eq!(frames(&text), frames(&stripped));
        assert!(stripped.contains("upstream-answer"));
        assert!(!stripped.contains(FRAME_TEXT));
        assert_eq!(done.error, None);
        assert_eq!(done.report.outcome, Outcome::Completed);
        assert_eq!(done.settlement, Settlement::Confirmed);
        assert!(done.report.emitted_bytes >= FRAME.len());
        assert_eq!(reports.lock().unwrap().len(), 1);
        assert_eq!(reports.lock().unwrap()[0].outcome, Outcome::Completed);
    }

    #[tokio::test]
    async fn upstream_idle_emits_ping_without_changing_event_frames() {
        let (release_tx, release_rx) = watch::channel(false);
        let url = serve(anthropic_sse(), Some(release_rx)).await;
        let limits = Limits {
            ping_interval: Duration::from_millis(50),
            ..Limits::default()
        };
        let (relay, _) = relay(&url, UpstreamProtocol::Anthropic, limits.clone(), None);
        let mut session = relay
            .start(parsed(ClientProtocol::Anthropic, &limits), context())
            .unwrap();
        let head = timeout(Duration::from_secs(2), &mut session.head)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        assert_eq!(head.status, 200);
        let mut bytes = Vec::new();
        read_until(&mut session.body, &mut bytes, FRAME).await;
        assert!(
            bytes.starts_with(FRAME),
            "idle upstream must not deliver business bytes before the comment: {bytes:?}"
        );
        release_tx.send(true).unwrap();
        finish_body(&mut session.body, &mut bytes).await;
        let done = timeout(Duration::from_secs(2), session.completion)
            .await
            .unwrap()
            .unwrap();
        let text = String::from_utf8(bytes).unwrap();
        assert_eq!(frames(&text), frames(&text.replace(FRAME_TEXT, "")));
        assert!(text.contains("upstream-answer"));
        assert_eq!(done.error, None);
        assert_eq!(done.report.outcome, Outcome::Completed);
        assert_eq!(done.settlement, Settlement::Confirmed);
    }

    #[tokio::test]
    async fn immediate_anthropic_stream_has_no_leading_ping() {
        let url = serve(anthropic_sse(), None).await;
        let limits = Limits {
            ping_interval: Duration::from_secs(30),
            ..Limits::default()
        };
        let (relay, _) = relay(&url, UpstreamProtocol::Anthropic, limits.clone(), None);
        let mut session = relay
            .start(parsed(ClientProtocol::Anthropic, &limits), context())
            .unwrap();
        let head = timeout(Duration::from_secs(2), &mut session.head)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        assert_eq!(head.status, 200);
        let mut bytes = Vec::new();
        finish_body(&mut session.body, &mut bytes).await;
        let done = timeout(Duration::from_secs(2), session.completion)
            .await
            .unwrap()
            .unwrap();
        let text = String::from_utf8(bytes).unwrap();
        assert!(!text.contains(": PING"), "{text}");
        assert!(text.contains("upstream-answer"));
        frames(&text);
        assert_eq!(done.error, None);
        assert_eq!(done.report.outcome, Outcome::Completed);
        assert_eq!(done.settlement, Settlement::Confirmed);
    }

    #[tokio::test]
    async fn chat_streams_do_not_commit_keepalive_during_reserve() {
        let (release_tx, release_rx) = oneshot::channel();
        let url = serve(chat_sse(), None).await;
        let limits = Limits {
            ping_interval: Duration::from_millis(40),
            hook_timeout: Duration::from_secs(3),
            ..Limits::default()
        };
        let (relay, _) = relay(
            &url,
            UpstreamProtocol::Chat,
            limits.clone(),
            Some(release_rx),
        );
        let mut session = relay
            .start(parsed(ClientProtocol::Chat, &limits), context())
            .unwrap();
        assert!(
            timeout(Duration::from_millis(120), &mut session.head)
                .await
                .is_err()
        );
        release_tx.send(()).unwrap();
        let head = timeout(Duration::from_secs(2), &mut session.head)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        assert_eq!(head.status, 200);
        let mut bytes = Vec::new();
        finish_body(&mut session.body, &mut bytes).await;
        let done = timeout(Duration::from_secs(2), session.completion)
            .await
            .unwrap()
            .unwrap();
        let text = String::from_utf8(bytes).unwrap();
        assert!(!text.contains(": PING"), "{text}");
        assert!(text.contains("upstream-answer"));
        assert_eq!(done.error, None);
        assert_eq!(done.report.outcome, Outcome::Completed);
    }

    #[tokio::test]
    async fn connect_retries_keep_the_stream_alive_and_record_failure() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        drop(listener);
        let url = format!("http://{address}/v1/messages");
        let limits = Limits {
            ping_interval: Duration::from_millis(40),
            connect_retries: 2,
            retry_backoff: Duration::from_millis(150),
            header_timeout: Duration::from_secs(2),
            connect_timeout: Duration::from_secs(1),
            hook_timeout: Duration::from_secs(2),
            total_timeout: Duration::from_secs(8),
            ..Limits::default()
        };
        let (relay, reports) = relay(&url, UpstreamProtocol::Anthropic, limits.clone(), None);
        let mut session = relay
            .start(parsed(ClientProtocol::Anthropic, &limits), context())
            .unwrap();
        let head = timeout(Duration::from_secs(2), &mut session.head)
            .await
            .expect("retry keepalive did not commit the stream")
            .unwrap()
            .unwrap();
        assert_eq!(head.status, 200);
        assert_eq!(head.content_type, STREAM_CONTENT_TYPE);
        let mut bytes = Vec::new();
        finish_body(&mut session.body, &mut bytes).await;
        let done = timeout(Duration::from_secs(3), session.completion)
            .await
            .unwrap()
            .unwrap();
        let text = String::from_utf8(bytes).unwrap();
        assert!(text.contains(FRAME_TEXT), "{text}");
        assert!(text.contains("event: error\n"), "{text}");
        assert!(text.contains("\"type\":\"error\""), "{text}");
        assert_eq!(done.report.outcome, Outcome::Failed);
        assert_ne!(done.report.outcome, Outcome::Completed);
        assert!(!done.report.provider_complete);
        assert!(done.report.upstream_attempted);
        assert!(
            done.report.attempts >= 2,
            "attempts={}",
            done.report.attempts
        );
        assert_eq!(done.settlement, Settlement::Confirmed);
        assert!(done.error.is_some());
        let saved = reports.lock().unwrap();
        assert_eq!(saved.len(), 1);
        assert_eq!(saved[0].outcome, Outcome::Failed);
    }
}

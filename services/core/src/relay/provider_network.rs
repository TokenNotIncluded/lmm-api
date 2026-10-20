//! Paid model traffic reuses the origin-bound transport used by moderation.
//! No second resolver, redirect, system proxy or relaxed TLS is introduced here.
use super::{
    Limits, RelayError,
    honeypot_network::{self, NetworkTarget, PreparedConnection},
};
use reqwest::{Client, Url};
use std::{net::SocketAddr, sync::Arc};
use tokio::sync::Semaphore;

#[derive(Clone)]
pub(crate) struct ProviderTransport {
    dns_slots: Arc<Semaphore>,
}

pub(crate) struct ProviderConnection(PreparedConnection);
impl ProviderConnection {
    pub(crate) fn client(&self) -> &Client {
        &self.0.client
    }
    pub(crate) fn matches_peer(&self, peer: Option<SocketAddr>) -> bool {
        self.0.matches_peer(peer)
    }
}

impl ProviderTransport {
    pub(crate) fn new(max_in_flight: usize) -> Self {
        Self {
            dns_slots: honeypot_network::dns_slots(max_in_flight),
        }
    }

    /// The URL comes only from an authorized immutable route. DNS resolution
    /// and connection preparation must be inside the caller's total deadline.
    pub(crate) async fn prepare(
        &self,
        endpoint: &Url,
        limits: &Limits,
    ) -> Result<ProviderConnection, RelayError> {
        let mut base = endpoint.clone();
        // Gemini streaming has a server-generated alt=sse query. Reject all
        // other queries even for trusted route constructors.
        if base.query().is_some_and(|query| query != "alt=sse") {
            return Err(RelayError::RouteUnavailable);
        }
        base.set_query(None);
        honeypot_network::validate_base_url(base.as_str(), true)?;
        let target = NetworkTarget {
            additional_root: None,
            local_http: true,
            allowed_origins: [honeypot_network::canonical_origin(&base)].into(),
        };
        target
            .prepare(endpoint, limits, self.dns_slots.clone())
            .await
            .map(ProviderConnection)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{Router, routing::post};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use tokio::net::TcpListener;

    #[tokio::test]
    async fn private_and_query_targets_are_rejected_before_connecting() {
        let transport = ProviderTransport::new(1);
        for raw in [
            "https://10.0.0.1/v1",
            "https://169.254.169.254/v1",
            "http://localhost/v1",
            "https://localhost/v1",
            "https://api.example/v1?key=secret",
        ] {
            assert!(
                transport
                    .prepare(&Url::parse(raw).unwrap(), &Limits::default())
                    .await
                    .is_err(),
                "{raw}"
            );
        }
    }

    #[tokio::test]
    async fn numeric_loopback_peer_is_pinned_and_redirect_never_replayed() {
        let count = Arc::new(AtomicUsize::new(0));
        let counter = count.clone();
        let app = Router::new().route(
            "/v1/responses",
            post(move || {
                counter.fetch_add(1, Ordering::SeqCst);
                async {
                    (
                        axum::http::StatusCode::TEMPORARY_REDIRECT,
                        [("location", "http://127.0.0.1:1/credential-leak")],
                    )
                }
            }),
        );
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let peer = listener.local_addr().unwrap();
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let transport = ProviderTransport::new(1);
        let endpoint = Url::parse(&format!("http://{peer}/v1/responses")).unwrap();
        let connection = transport
            .prepare(&endpoint, &Limits::default())
            .await
            .unwrap();
        let response = connection
            .client()
            .post(endpoint)
            .bearer_auth("synthetic-only")
            .send()
            .await
            .unwrap();
        assert_eq!(response.status().as_u16(), 307);
        assert!(connection.matches_peer(response.remote_addr()));
        assert!(!connection.matches_peer(Some("127.0.0.1:1".parse().unwrap())));
        assert_eq!(count.load(Ordering::SeqCst), 1);
        server.abort();
        let _ = server.await;
    }
}

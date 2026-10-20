//! Same-process private HTTP/1 host port. Reuses RpcServer's safe filesystem
//! binding instead of introducing another socket owner or deployment program.
use super::{ToolMarketStore, http};
use axum::{Router, body::Body};
use hyper::{Request, body::Incoming, server::conn::http1};
use hyper_util::{
    rt::{TokioIo, TokioTimer},
    service::TowerToHyperService,
};
use std::{io, path::Path, sync::Arc, time::Duration};
use tokio::{
    net::UnixListener,
    sync::{Semaphore, watch},
    task::JoinSet,
};
use tower::ServiceExt;

pub struct ToolMarketServer {
    listener: UnixListener,
    _guard: crate::internal_rpc::SocketGuard,
    app: Router,
}
impl ToolMarketServer {
    /// Token is an independent toolmarket host credential, not an identity,
    /// commerce or end-user token. Caller loads its private file explicitly.
    pub fn bind(
        path: &Path,
        token: &[u8],
        store: ToolMarketStore,
        shared_group_id: Option<u32>,
    ) -> io::Result<Self> {
        let app = http::router(store, token).map_err(|_| {
            io::Error::new(io::ErrorKind::InvalidInput, "invalid toolmarket credential")
        })?;
        Self::bind_with_router(path, token, app, shared_group_id)
    }
    /// Compose independently authenticated private projections on this one
    /// bounded listener. Every supplied router must enforce its own host token;
    /// the binder token is validated but does not authorize another route.
    pub fn bind_with_router(
        path: &Path,
        binding_token: &[u8],
        app: Router,
        shared_group_id: Option<u32>,
    ) -> io::Result<Self> {
        let (listener, guard) = crate::internal_rpc::RpcServer::bind_with_group(
            path,
            binding_token,
            None,
            shared_group_id,
        )?
        .into_private_listener();
        Ok(Self {
            listener,
            _guard: guard,
            app,
        })
    }
    /// Each request is finite, not an SSE or model stream: 8 connections,
    /// 2-second header budget, 5-second total request/drain bound. SQL errors or
    /// shutdown loss of ACK remain UNKNOWN to the original-operation client.
    pub async fn serve(self, mut shutdown: watch::Receiver<bool>) -> io::Result<()> {
        let Self {
            listener,
            _guard,
            app,
        } = self;
        let slots = Arc::new(Semaphore::new(8));
        let mut tasks = JoinSet::new();
        loop {
            if *shutdown.borrow() {
                break;
            }
            tokio::select! {
                biased;
                _=shutdown.changed()=>break,
                Some(_)=tasks.join_next(),if !tasks.is_empty()=>{},
                accepted=listener.accept()=>{
                    let (stream,_)=accepted?;
                    let Ok(permit)=slots.clone().try_acquire_owned() else {drop(stream);continue};
                    let service=TowerToHyperService::new(app.clone().map_request(|r:Request<Incoming>|r.map(Body::new)));
                    tasks.spawn(async move {
                        let _permit=permit;
                        let mut builder=http1::Builder::new();
                        builder.keep_alive(false).max_headers(32).max_buf_size(32768).timer(TokioTimer::new()).header_read_timeout(Duration::from_secs(2));
                        let _=tokio::time::timeout(Duration::from_secs(5),builder.serve_connection(TokioIo::new(stream),service)).await;
                    });
                }
            }
        }
        let drain = async { while tasks.join_next().await.is_some() {} };
        if tokio::time::timeout(Duration::from_secs(5), drain)
            .await
            .is_err()
        {
            tasks.abort_all();
            while tasks.join_next().await.is_some() {}
        }
        drop(_guard);
        Ok(())
    }
}

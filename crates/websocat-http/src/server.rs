#![allow(unused)]
use futures::StreamExt;
use websocat_api::{
    anyhow, async_trait::async_trait, bytes, futures::TryStreamExt, http, tokio, NodeId, Result,
};
use websocat_derive::WebsocatNode;
#[derive(Debug, derivative::Derivative, WebsocatNode)]
#[websocat_node(official_name = "http-server", validate)]
#[auto_populate_in_allclasslist]
#[derivative(Clone)]
pub struct HttpServer {
    /// IO bytestream node to use
    inner: NodeId,

    /// Expect and handle upgrades
    #[websocat_prop(default = false)]
    upgrade: bool,
}

impl HttpServer {
    fn validate(&mut self) -> Result<()> {
        Ok(())
    }
}

async fn handle_request(
    rq: hyper::Request<hyper::Body>,
    tx: Option<tokio::sync::oneshot::Sender<(websocat_api::Source, websocat_api::Sink)>>,
) -> Result<hyper::Response<hyper::Body>> {
    tracing::info!("rq: {:?}", rq);
    if let (Some(tx)) = tx {
        let mut incoming_body = rq.into_body();
        let (sender, outgoing_body) = hyper::Body::channel();
        let sink = crate::util::body_sink(sender);

        let (request_tx, request_rx) = tokio::sync::mpsc::channel::<bytes::Bytes>(1);
        while let Some(buf) = incoming_body.next().await {
            request_tx.send(buf?).await?;
        }

        let source = crate::util::body_source(request_rx);

        tx.send((
            websocat_api::Source::Datagrams(Box::pin(source)),
            websocat_api::Sink::Datagrams(Box::pin(sink)),
        ));

        incoming_body;
        Ok(hyper::Response::new(outgoing_body))
    } else {
        anyhow::bail!("Trying to reuse HTTP connection for second request to Websocat, which is not supported in this mode")
    }
}
async fn handle_request_for_upgrade(
    rq: hyper::Request<hyper::Body>,
    tx: Option<tokio::sync::oneshot::Sender<(websocat_api::Source, websocat_api::Sink)>>,
) -> Result<hyper::Response<hyper::Body>> {
    tracing::info!("rq: {:?}", rq);
    if let (Some(tx)) = tx {
        let upg = hyper::upgrade::on(rq);
        tokio::spawn(async {
            match upg.await {
                Ok(upg) => {
                    let (r, w) = tokio::io::split(upg);
                    // TODO: also try downcast somehow, like in the client
                    tx.send((
                        websocat_api::Source::ByteStream(Box::pin(r)),
                        websocat_api::Sink::ByteStream(Box::pin(w)),
                    ));
                }
                Err(e) => {
                    tracing::error!("{}", e);
                    drop(tx);
                }
            }
        });
        let mut resp = hyper::Response::new(hyper::Body::empty());
        resp.headers_mut().append(http::header::CONNECTION, http::HeaderValue::from_static("upgrade"));
        *resp.status_mut() = hyper::StatusCode::SWITCHING_PROTOCOLS;
        Ok(resp)
    } else {
        anyhow::bail!("Trying to reuse HTTP connection for second request to Websocat, which is not supported in this mode")
    }
}

#[async_trait]
impl websocat_api::RunnableNode for HttpServer {
    async fn run(
        self: std::pin::Pin<std::sync::Arc<Self>>,
        ctx: websocat_api::RunContext,
        multiconn: Option<websocat_api::ServerModeContext>,
    ) -> Result<websocat_api::Bipipe> {
        let mut io = None;
        let mut cn = None;

        let io_ = ctx.nodes[self.inner]
            .clone()
            .upgrade()?
            .run(ctx.clone(), multiconn)
            .await?;
        cn = io_.closing_notification;
        io = Some(match (io_.r, io_.w) {
            (websocat_api::Source::ByteStream(r), websocat_api::Sink::ByteStream(w)) => {
                readwrite::ReadWriteTokio::new(r, w)
            }
            _ => {
                anyhow::bail!("HTTP server requires a bytestream-based inner node");
            }
        });

        let (tx, rx) = tokio::sync::oneshot::channel();

        let mut tx = std::sync::Arc::new(std::sync::Mutex::new(Some(tx)));

        let http = hyper::server::conn::Http::new();

        if !self.upgrade {
            let service = hyper::service::service_fn(move |rq| {
                let tx = tx.clone().lock().unwrap().take();
                handle_request(rq, tx)
            });
            let conn = http.serve_connection(io.unwrap(), service);

            use websocat_api::futures::TryFutureExt;
            tokio::spawn(conn.map_err(|e| {
                tracing::error!("hyper server error: {}", e);
                ()
            }));
        } else {
            let service = hyper::service::service_fn(move |rq| {
                let tx = tx.clone().lock().unwrap().take();
                handle_request_for_upgrade(rq, tx)
            });
            let conn = http.serve_connection(io.unwrap(), service);

            let conn = conn.with_upgrades();

            use websocat_api::futures::TryFutureExt;
            tokio::spawn(conn.map_err(|e| {
                tracing::error!("hyper server error: {}", e);
                ()
            }));
        }

        let (r, w) = rx.await?;
    
        Ok(websocat_api::Bipipe {
            r,
            w,
            closing_notification: cn,
        })
    }
}

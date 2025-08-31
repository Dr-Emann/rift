use std::net::SocketAddr;
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper::{Request, Response, StatusCode};
use hyper::body::Incoming;
use http_body_util::Full;
use bytes::Bytes;
use tokio::net::TcpListener;
use tracing::{info, error};

pub struct ProxyServer {
    addr: SocketAddr,
}

impl ProxyServer {
    pub fn new(host: &str, port: u16) -> Self {
        let addr: SocketAddr = format!("{}:{}", host, port).parse().unwrap();
        Self { addr }
    }

    pub async fn run(&self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let listener = TcpListener::bind(self.addr).await?;
        info!("Listening on {}", self.addr);

        loop {
            let (stream, _) = listener.accept().await?;
            let io = hyper_util::rt::TokioIo::new(stream);

            tokio::spawn(async move {
                if let Err(e) = http1::Builder::new()
                    .serve_connection(io, service_fn(handle_request))
                    .await
                {
                    error!("Connection error: {}", e);
                }
            });
        }
    }
}

async fn handle_request(
    req: Request<Incoming>,
) -> Result<Response<Full<Bytes>>, hyper::Error> {
    info!("{} {}", req.method(), req.uri());

    Ok(Response::builder()
        .status(StatusCode::OK)
        .body(Full::new(Bytes::from("OK")))
        .unwrap())
}

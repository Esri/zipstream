// © 2019 3D Robotics. License: Apache-2.0
use aws_config::meta::region::RegionProviderChain;
use aws_sdk_s3 as s3;

use bytes::Bytes;
use http_body_util::{BodyExt, Either};
use hyper::server::conn::http1;
use hyper_util::rt::{TokioIo, TokioExecutor};
use tokio::net::TcpListener;
use zipstream::{
    upstream,
    Config, stream_range::BoxError,
    error::Report,
};

use std::net::SocketAddr;

use clap::Parser;
use hyper::{ Request, Response, StatusCode, body::{self, Body} };
use hyper::service::service_fn;
use hyper_tls::HttpsConnector;
use tracing::{error, info, info_span, warn, Instrument};

#[global_allocator]
static GLOBAL: jemallocator::Jemalloc = jemallocator::Jemalloc;

type HyperClient = hyper_util::client::legacy::Client<HttpsConnector<hyper_util::client::legacy::connect::HttpConnector>, http_body_util::Empty<Bytes>>;

#[derive(Parser, Debug)]
#[command(version, about, long_about = None)]
struct Args {
    /// Upstream server that provides zip file manifests
    #[arg(long, value_name="URL")]
    upstream: String,

    /// Remove a prefix from the URL path before proxying to upstream server
    #[arg(long, value_name="PREFIX", default_value="")]
    pub strip_prefix: String,

    /// Value passed in the X-Via-Zip-Stream header on the request to the upstream server
    #[arg(long, value_name="VAL", default_value="true")]
    pub header_value: String,

    /// IP:port to listen for HTTP connections
    #[arg(long, value_name="IP:PORT", default_value="[::1]:3000")]
    pub listen: SocketAddr,
}


#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    log_panics::init();
    let args = Args::parse();

    let subscriber = tracing_subscriber::fmt()
        .json()
        .flatten_event(true)
        .with_current_span(false)
        .finish();
    tracing::subscriber::set_global_default(subscriber)?;
    
    info!("Startup");

    let region_provider = RegionProviderChain::default_provider();
    let s3_config = aws_config::defaults(aws_config::BehaviorVersion::v2023_11_09()).region(region_provider).load().await;
    let s3_client = s3::Client::new(&s3_config);

    let config = Config {
        upstream: args.upstream,
        strip_prefix: args.strip_prefix,
        via_zip_stream_header_value: args.header_value,
    };

    let client = hyper_util::client::legacy::Client::builder(TokioExecutor::new()).build(HttpsConnector::new());

    let listener = TcpListener::bind(args.listen).await?;

    loop {
        let (stream, addr) = listener.accept().await?;

        let span = info_span!(
            "connection",
            socket_addr = ?addr,
        );

        let client = client.clone();
        let s3_client = s3_client.clone();
        let config = config.clone();

        let io = TokioIo::new(stream);

        tokio::task::spawn(async move {
            if let Err(err) = http1::Builder::new()
                .serve_connection(io, service_fn(|req| { async {
                    let span = info_span!(
                        "request",
                        method = ?req.method(),
                        uri = ?req.uri(),
                    );

                    match handle_request(req, &client, s3_client.clone(), &config).instrument(span).await {
                        Ok(res) => Ok(res.map(Either::Right)),
                        Err((status, msg)) => {
                            Response::builder().status(status).body(Either::Left(http_body_util::Full::new(Bytes::from(msg))))
                        }
                    }
                }}))
                .await
            {
                warn!("Error serving connection: {}", Report(err));
            }
        }.instrument(span));
    }
}

async fn handle_request(
    req: Request<body::Incoming>,
    client: &HyperClient,
    s3_client: s3::Client,
    config: &Config
) -> Result<
        Response<Either<body::Incoming, impl Body<Data=Bytes, Error=BoxError>>>,
        (StatusCode, &'static str)
    > {
    info!("Request: {} {}", req.method(), req.uri());
    let upstream_req = upstream::request(config, &req)?;
    let upstream_res = client.request(upstream_req).await.map_err(|e| {
        error!("Failed to connect upstream: {}", Report(e));
        (StatusCode::SERVICE_UNAVAILABLE, "Upstream connection failed")
    })?;

    if upstream_res.headers().get("X-Zip-Stream").is_some() {
        let body = upstream_res.into_body().collect().await.map_err(|e| {
            error!("Failed to read upstream body: {}", Report(e));
            (StatusCode::SERVICE_UNAVAILABLE, "Upstream request failed")
        })?;

        upstream::response(s3_client, &req, body.to_bytes()).map(|res| res.map(Either::Right))
    } else {
        info!("Request proxied from upstream");
        Ok(upstream_res.map(Either::Left))
    }
}

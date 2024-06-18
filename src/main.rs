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

use std::{net::SocketAddr, time::Duration};

use clap::Parser;
use hyper::{ Request, Response, StatusCode, body::{self, Body} };
use hyper::service::service_fn;
use hyper_tls::HttpsConnector;
use tracing::{error, event, info, info_span, warn, Instrument, Level};

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

    tokio::task::spawn(log_metrics());

    let app = App::new(Config {
        upstream: args.upstream,
        strip_prefix: args.strip_prefix,
        via_zip_stream_header_value: args.header_value,
    }).await;

    let listener = TcpListener::bind(args.listen).await?;

    loop {
        let (stream, _) = listener.accept().await?;
        let io = TokioIo::new(stream);

        let app = app.clone();

        tokio::task::spawn(async move {
            if let Err(err) = http1::Builder::new()
                .serve_connection(io, service_fn(|req| { async {
                    let span = info_span!(
                        "request",
                        id = %uuid::Uuid::now_v7().simple(),
                        path = req.uri().path(),
                    );

                    span.in_scope(|| {
                        info!(
                            http.request.method = ?req.method(),
                            url.path = req.uri().path(),
                            http.request.raw_headers = ?req.headers(),
                            "{:?} {}", req.method(), req.uri(),
                        )
                    });

                    match app.handle_request(req).instrument(span).await {
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
        });
    }
}

#[derive(Clone)]
struct App {
    config: Config,
    upstream_client: HyperClient,
    s3_client: s3::Client,
}

impl App {
    async fn new(config: Config) -> App {
        let upstream_client = hyper_util::client::legacy::Client::builder(TokioExecutor::new()).build(HttpsConnector::new());

        let region_provider = RegionProviderChain::default_provider();
        let s3_config = aws_config::defaults(aws_config::BehaviorVersion::v2023_11_09()).region(region_provider).load().await;
        let s3_client = s3::Client::new(&s3_config);

        App { config, upstream_client, s3_client }
    }

    async fn handle_request(&self, req: Request<body::Incoming>) -> Result<
        Response<Either<body::Incoming, impl Body<Data=Bytes, Error=BoxError>>>,
        (StatusCode, &'static str)
    > {
        let upstream_req = upstream::request(&self.config, &req)?;
        let upstream_res = self.upstream_client.request(upstream_req).await.map_err(|e| {
            error!("Failed to connect upstream: {}", Report(e));
            (StatusCode::SERVICE_UNAVAILABLE, "Upstream connection failed")
        })?;

        if upstream_res.headers().get("X-Zip-Stream").is_some() {
            let body = upstream_res.into_body().collect().await.map_err(|e| {
                error!("Failed to read upstream body: {}", Report(e));
                (StatusCode::SERVICE_UNAVAILABLE, "Upstream request failed")
            })?;

            upstream::response(self.s3_client.clone(), &req, body.to_bytes()).map(|res| res.map(Either::Right))
        } else {
            info!("Response proxied from upstream");
            Ok(upstream_res.map(Either::Left))
        }
    }
}

async fn log_metrics() {
    let mut interval = tokio::time::interval(Duration::from_secs(30));

    loop {
        interval.tick().await;

        jemalloc_ctl::epoch::advance().unwrap();
        let allocated = jemalloc_ctl::stats::allocated::read().unwrap();
        let resident = jemalloc_ctl::stats::resident::read().unwrap();

        let active_downloads = zipstream::serve_range::active_downloads();

        event!(target: "zipstream::metrics", Level::INFO,
            zipstream.active_downloads = active_downloads,
            jemalloc.allocated = allocated,
            jemalloc.resident = resident,
        )
    }
}

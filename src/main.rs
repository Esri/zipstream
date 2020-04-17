// © 2019 3D Robotics. License: Apache-2.0
use hyper;
use rusoto_s3;
use rusoto_core;
use log;
use env_logger;
use log_panics;

mod stream_range;
mod serve_range;
mod zip;
mod upstream;
mod s3url;

use std::sync::Arc;

use futures::{ future, Future, Stream, future::Either };
use clap::{Arg, App};
use hyper::{ Client, Response, Server, StatusCode };
use hyper::service::service_fn;
use hyper_tls::HttpsConnector;

#[derive(Clone)]
pub struct Config {
    upstream: String,
    strip_prefix: String,
    via_zip_stream_header_value: String,
}

fn main() {
    let mut logger = env_logger::Builder::from_default_env();
    logger.filter_level(log::LevelFilter::Info);
    logger.write_style(env_logger::WriteStyle::Never);
    logger.init();
    log_panics::init();
    log::info!("Startup");

    let matches = App::new("zipstream")
        .arg(Arg::with_name("upstream")
            .long("upstream")
            .takes_value(true)
            .help("Upstream server that provides zip file manifests")
            .value_name("URL")
            .required(true))
        .arg(Arg::with_name("strip-prefix")
            .long("strip-prefix")
            .takes_value(true)
            .help("Remove a prefix from the URL path before proxying to upstream server")
            .default_value(""))
        .arg(Arg::with_name("header-value")
            .long("header-value")
            .takes_value(true)
            .help("Value passed in the X-Via-Zip-Stream header on the request to the upstream server")
            .default_value("true"))
        .arg(Arg::with_name("listen")
            .long("listen")
            .takes_value(true)
            .help("IP:port to listen for HTTP connections")
            .default_value("127.0.0.1:3000"))
        .get_matches();

    let region = rusoto_core::Region::default();
    let s3_client = Arc::new(rusoto_s3::S3Client::new(region)) as Arc<dyn rusoto_s3::S3 + Send + Sync>;

    let config = Config {
        upstream: matches.value_of("upstream").unwrap().into(),
        strip_prefix:matches.value_of("strip-prefix").unwrap().into(),
        via_zip_stream_header_value: matches.value_of("header-value").unwrap().into(),
    };

    let https = HttpsConnector::new(4).unwrap();
    let client_main = Client::builder()
        .build::<_, hyper::Body>(https);

    let addr = matches.value_of("listen").unwrap().parse().expect("invalid `listen` value");

    let new_svc = move || {
        let client = client_main.clone();
        let s3_client = s3_client.clone();
        let config = config.clone();

        service_fn(move |req|{
            let s3_client = s3_client.clone();
            log::info!("Request: {} {}", req.method(), req.uri());

            future::result(upstream::request(&config, &req).map(|upstream_req| {
                client.request(upstream_req).map_err(|e| {
                    log::error!("Failed to connect upstream: {}", e);
                    (StatusCode::SERVICE_UNAVAILABLE, "Upstream connection failed")
                })
            })).flatten().and_then(move |upstream_response| {
                if upstream_response.headers().get("X-Zip-Stream").is_some() {
                    Either::A(upstream_response.into_body().concat2().map_err(|e| {
                        log::error!("Failed to read upstream body: {}", e);
                        (StatusCode::SERVICE_UNAVAILABLE, "Upstream request failed")
                    }).and_then(move |body| {
                        upstream::response(&s3_client, &req, &body[..])
                    }))
                } else {
                    log::info!("Request proxied from upstream");
                    Either::B(future::ok(upstream_response))
                }
            }).or_else(|(status, message)| {
                future::ok::<_, hyper::Error>(Response::builder().status(status).body(message.into()).unwrap())
            })
        })
    };

    let server = Server::bind(&addr)
        .serve(new_svc)
        .map_err(|e| log::error!("server error: {}", e));

    hyper::rt::run(server);
}

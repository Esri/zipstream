// © 2019 3D Robotics. License: Apache-2.0
use crate::Config;
use crate::stream_range::{ StreamRange, S3Object };
use crate::serve_range::hyper_response;
use crate::zip::{ ZipEntry, ZipOptions, zip_stream };
use crate::s3url::S3Url;

use aws_sdk_s3 as s3;
use bytes::Bytes;
use hyper::{header, Body, Request, Response, Uri, Method, StatusCode};
use serde_derive::Deserialize;
use std::hash::{ Hash, Hasher };
use chrono::{DateTime, Utc};

#[derive(Deserialize, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
struct ZipFileDescription {
    archive_name: String,
    source: S3Url,
    length: u64,
    crc: u32,
    last_modified: DateTime<Utc>,
}

#[derive(Deserialize, Clone, Debug, Hash)]
struct UpstreamResponse {
    filename: String,
    entries: Vec<ZipFileDescription>,
}

static KEEP_HEADERS: &[header::HeaderName] = &[
    header::AUTHORIZATION,
    header::COOKIE,
    header::USER_AGENT,
    header::REFERER,
];

/// Modify a client request into an upstream request
pub fn request(config: &Config, req: &Request<Body>) -> Result<Request<Body>, (StatusCode, &'static str)> {
    if req.method() != Method::GET {
        return Err((StatusCode::METHOD_NOT_ALLOWED, "Only GET requests allowed"))
    }

    let mut new_req = Request::builder().uri({
        let req_path = req.uri().path_and_query().expect("request URL should have path").as_str();

        if !req_path.starts_with(&config.strip_prefix) {
            return Err((StatusCode::NOT_FOUND, "Not found"))
        }

        format!("{}{}", config.upstream, &req_path[config.strip_prefix.len()..]).parse::<Uri>().unwrap()
    }).header("X-Via-Zip-Stream", config.via_zip_stream_header_value.clone());

    for header in KEEP_HEADERS {
        if let Some(value) = req.headers().get(header) {
            new_req = new_req.header(header, value);
        }
    }
    
    Ok(new_req.body(Body::empty()).unwrap())
}

/// Parse an upstream JSON response and produce a streaming zip file response
pub fn response(client: s3::Client, req: &Request<Body>, response_body: Bytes) -> Result<Response<Body>, (StatusCode, &'static str)> {
    let mut res: UpstreamResponse = serde_json::from_slice(&response_body[..]).map_err(|e| {
        log::error!("Invalid upstream response JSON: {}", e);
        (StatusCode::INTERNAL_SERVER_ERROR, "Failed to parse upstream request")
    })?;
    
    drop(response_body);

    res.entries.sort();

    let etag = {
        //TODO: use a hash function that is stable across releases and architectures
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        res.hash(&mut hasher);
        format!("{:x}", hasher.finish())
    };
    
    let entries: Vec<ZipEntry> = res.entries.into_iter().map(|file| {
        ZipEntry {
            archive_path: file.archive_name,
            crc: file.crc,
            data: Box::new(S3Object { 
                client: client.clone(),
                bucket: file.source.bucket,
                key: file.source.key,
                len: file.length
            }),
            last_modified: file.last_modified,
        }
    }).collect();

    let num_entries = entries.len();

    let stream = zip_stream(entries, ZipOptions::default());

    log::info!("Streaming zip file {}: {} entries, {} bytes", res.filename, num_entries, stream.len());

    Ok(hyper_response(req, "application/zip", &etag, &res.filename, &stream))
}


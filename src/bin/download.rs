use std::{env, sync::Arc};
use anyhow::{Context, Error, anyhow};
use clap::{Arg, App, SubCommand};
use rusoto_core::{HttpClient};
use rusoto_s3::{S3, S3Client, GetObjectRequest};
use rusoto_credential::{StaticProvider, AwsCredentials};
use tokio::io::{ AsyncReadExt, AsyncWriteExt };
use tokio::fs::File;
use futures::stream::StreamExt;

use zipstream::s3url::S3Url;
use zipstream::upstream::UpstreamResponse;
use zipstream::stream_range::{StreamRange, S3Object, Range};
use zipstream::zip::{ZipEntry, zip_stream, ZipOptions};

#[tokio::main]
async fn main() {
    let mut logger = env_logger::Builder::from_default_env();
    logger.filter_level(log::LevelFilter::Info);
    logger.init();
    log_panics::init();


    let region = rusoto_core::Region::default();
    //let s3_client = Arc::new(rusoto_s3::S3Client::new(region));

    let s3_client = Arc::new(S3Client::new_with(
        HttpClient::new().unwrap(),
        StaticProvider::from(AwsCredentials::default()),
        Default::default()
     ));

    let matches = App::new("myapp")
                          .args_from_usage(
                              "-m, --manifest_path=[FILE] 'Path to manifest file'
                              -o, --output_path=[FILE]       'output path'")
                          .get_matches();

    let manifest_s3url = matches.value_of("manifest_path").unwrap().parse::<S3Url>().unwrap();
    let manifest_json = s3_download(&*s3_client, &manifest_s3url).await.unwrap();     
    let mut manifest: UpstreamResponse = serde_json::from_slice(&manifest_json).unwrap();

    manifest.entries.sort();
    
    let entries: Vec<ZipEntry> = manifest.entries.into_iter().map(|file| {
        ZipEntry {
            archive_path: file.archive_name,
            crc: file.crc,
            data: Box::new(S3Object { 
                s3: s3_client.clone(),
                bucket: file.source.bucket,
                key: file.source.key,
                len: file.length
            }),
            last_modified: file.last_modified,
        }
    }).collect();

    let num_entries = entries.len();

    let zip = zip_stream(entries, ZipOptions::default());
    let length = zip.len();

    log::info!("Streaming zip file: {} entries, {} bytes", num_entries, length);

    let mut stream = zip.stream_range(Range{ start: 0, end: length });

    let output_path = matches.value_of("output_path").unwrap();
    let mut file = File::create(output_path).await.unwrap();

    let mut completed: usize = 0;

    while let Some(chunk_res) = stream.next().await {
        let chunk = chunk_res.unwrap();
        file.write_all(&chunk).await.unwrap();
        completed += chunk.len();
        eprintln!("\r{} / {}", completed, length);
    }
}

async fn s3_download(s3_client: &dyn S3, s3url: &S3Url) -> Result<Vec<u8>, Error> {
    let response = s3_client.get_object(GetObjectRequest {
        bucket: s3url.bucket.to_owned(),
        key: s3url.key.to_owned(),
        ..Default::default()
      }).await.context("failed to request file from S3")?;
    
      let mut body = Vec::new();
    
      response.body
        .ok_or_else(|| anyhow!("missing body on s3 response"))?
        .into_async_read()
        .read_to_end(&mut body).await?;

    Ok(body)
}
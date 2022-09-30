/// This demo server is a stand-in for your application. A real application
/// would store the length and CRC of your S3 files in a database, and use the
/// proxied request URL and headers to generate a manifest like the one below.

use std::convert::Infallible;
use hyper::service::{make_service_fn, service_fn};
use hyper::{Body, Request, Response, Server};

async fn handler(req: Request<Body>) -> Result<Response<Body>, Infallible> {
    eprintln!("Demo server got a request for {}", req.uri());

    let res = r#"
    {
        "filename": "test.zip",
        "entries": [
          {
            "archive_name": "test1.txt",
            "length": 6,
            "crc": 2086221595,
            "source": "s3://sitescan-test/zipstream-demo/test1.txt",
            "last_modified": "2022-09-29T22:06:27.884Z"
          },
          {
            "archive_name": "test2.txt",
            "length": 6,
            "crc": 1467245784,
            "source": "s3://sitescan-test/zipstream-demo/test2.txt",
            "last_modified": "2022-09-29T22:06:27.884Z"
          }
        ]
      }
    "#;

    Ok(Response::builder()
      .header("Content-type", "application/json")
      .header("X-Zip-Stream", "true")
      .body(Body::from(res))
      .unwrap()
    )
}

#[tokio::main]
pub async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let make_svc = make_service_fn(|_conn| {
        async { Ok::<_, Infallible>(service_fn(handler)) }
    });

    let addr = ([127, 0, 0, 1], 3001).into();
    Server::bind(&addr).serve(make_svc).await?;

    Ok(())
}
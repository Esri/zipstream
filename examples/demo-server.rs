/// This demo server is a stand-in for your application. A real application
/// would store the length and CRC of your S3 files in a database, and use the
/// proxied request URL and headers to generate a manifest like the one below.

use std::convert::Infallible;
use bytes::Bytes;
use hyper::service::service_fn;
use hyper::{Request, Response, body::Body};
use hyper::server::conn::http1;
use hyper_util::rt::TokioIo;
use tokio::net::TcpListener;
use std::net::SocketAddr;

async fn handler(req: Request<impl Body>) -> Result<Response<impl Body<Data=Bytes, Error=Infallible>>, Infallible> {
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
      .body(http_body_util::Full::new(Bytes::from(res)))
      .unwrap()
    )
}

#[tokio::main]
pub async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let addr = SocketAddr::from(([127, 0, 0, 1], 3001));
    let listener = TcpListener::bind(addr).await?;

    loop {
        let (stream, _) = listener.accept().await?;

        let io = TokioIo::new(stream);

        tokio::task::spawn(async move {
            if let Err(err) = http1::Builder::new()
                .serve_connection(io, service_fn(handler))
                .await
            {
                println!("Error serving connection: {:?}", err);
            }
        });
    }
}

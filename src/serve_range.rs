// © 2019 3D Robotics. License: Apache-2.0

use std::{error::Error, pin::Pin, task::Poll};

use bytes::Bytes;
use futures::{Stream, StreamExt};
use crate::{error::Report, stream_range::BoxBytesStream};
use http_body_util::StreamBody;
use hyper::{Request, Response, body::{Body, Frame}, StatusCode, header};
use crate::stream_range::{ BoxError, Range, StreamRange };
use tracing::{error, info, Span};

/// Parse an HTTP range header to a `Range`
///
/// Returns Ok(Some(Range{..})) for a valid range, Ok(None) for a missing or unsupported range,
/// or Err(msg) if parsing fails.
pub fn parse_range(range_val: &str, total_len: u64) -> Result<Option<Range>, &'static str> {
    if !range_val.starts_with("bytes=") {
        return Err("invalid range unit");
    }

    let range_val = &range_val["bytes=".len()..].trim();

    if range_val.contains(',') {
        return Ok(None); // multiple ranges unsupported, but it's legal to just ignore the header
    }

    if let Some(range_end) = range_val.strip_prefix('-') {
        let s = range_end.parse::<u64>().map_err(|_| "invalid range number")?;
        
        if s >= total_len {
            return Ok(None);
        }

        Ok(Some(Range { start: total_len-s, end: total_len }))
    } else if let Some(range_start) = range_val.strip_suffix('-') {
        let s = range_start.parse::<u64>().map_err(|_| "invalid range number")?;
        
        if s >= total_len {
            return Ok(None);
        }

        Ok(Some(Range { start: s, end: total_len}))
    } else if let Some(h) = range_val.find('-') {
        let s = range_val[..h].parse::<u64>().map_err(|_| "invalid range number")?;
        let e = range_val[h+1..].parse::<u64>().map_err(|_| "invalid range number")?;

        if e >= total_len || s > e {
            return Ok(None);
        }

        Ok(Some(Range { start: s, end: e+1 }))
    } else {
        Err("invalid range")
    }
}

#[test]
fn test_range() {
    assert_eq!(parse_range("lines=0-10", 1000), Err("invalid range unit"));

    assert_eq!(parse_range("bytes=500-", 1000), Ok(Some(Range { start: 500, end: 1000})));
    assert_eq!(parse_range("bytes=2000-", 1000), Ok(None));
    
    assert_eq!(parse_range("bytes=-100", 1000), Ok(Some(Range { start: 900, end: 1000})));
    assert_eq!(parse_range("bytes=-2000", 1000), Ok(None));

    assert_eq!(parse_range("bytes=100-200", 1000), Ok(Some(Range { start: 100, end: 201})));
    assert_eq!(parse_range("bytes=500-999", 1000), Ok(Some(Range { start: 500, end: 1000})));
    assert_eq!(parse_range("bytes=500-1000", 1000), Ok(None));
    assert_eq!(parse_range("bytes=200-100", 1000), Ok(None));
    assert_eq!(parse_range("bytes=1500-2000", 1000), Ok(None));

    assert_eq!(parse_range("bytes=", 1000), Err("invalid range"));
    assert_eq!(parse_range("bytes=a-", 1000), Err("invalid range number"));
    assert_eq!(parse_range("bytes=a-b", 1000), Err("invalid range number"));
    assert_eq!(parse_range("bytes=-b", 1000), Err("invalid range number"));
}

/// Serve a `StreamRange` in response to a `hyper` request.
/// This handles the HTTP Range header and "206 Partial content" and associated headers if required
pub fn hyper_response(req: &Request<impl Body>, content_type: &str, etag: &str, filename: &str, data: &dyn StreamRange) -> Response<impl Body<Data=Bytes, Error=BoxError>> {
    let full_len = data.len();
    let full_range = Range { start: 0, end: full_len };

    let range = req.headers().get(hyper::header::RANGE)
        .filter(|_| req.headers().get(hyper::header::IF_RANGE).map_or(true, |val| val == etag))
        .and_then(|v| v.to_str().ok())
        .and_then(|v| parse_range(v, full_len).ok())
        .and_then(|x| x);

    let mut res = Response::builder()
        .header(header::CONTENT_TYPE, content_type)
        .header(header::ACCEPT_RANGES, "bytes")
        .header(header::ETAG, etag)
        .header(header::CONTENT_DISPOSITION, format!("attachment; filename=\"{}\"", filename));

    if let Some(range) = range {
        res = res.status(StatusCode::PARTIAL_CONTENT)
                 .header(header::CONTENT_RANGE, format!("bytes {}-{}/{}", range.start, range.end - 1, full_len));
        info!("Serving range {:?}", range);
    }

    let range = range.unwrap_or(full_range).limit_end(full_len);

    res = res.header(header::CONTENT_LENGTH, range.len());

    let stream = StreamMonitor::new(data.stream_range(range), range.len());

    res.body(StreamBody::new(stream.map(|chunk| chunk.map(Frame::data)))).unwrap()
}

/// Wraps a `BoxByteStream` with `tracing` instrumentation. The data is passed
/// through unchanged.
/// 
/// * Tracks the progress of the download, and on Drop logs whether the download
/// was completed or cancelled. Hyper drops the body stream after the specified
/// content-length is reached, so we do not see the `Stream::poll_next` return
/// `None` when the stream signals its own end. Ideally, Hyper would offer a
/// better way to follow the status of a download after the `Body` is passed to
/// Hyper: https://github.com/hyperium/hyper/issues/2181
/// 
/// * Stores a `tracing::Span` and enters it when polling the Stream, like
/// `Instrument`. The `Instrument` in `tracing` does not impl `Stream`. The one
/// in `tracing-futures` does, but it hasn't been released recently, and the
/// released version 0.2.5 does not include the change to enter the `Span` on
/// drop, which we need here for the logging in the `Drop` impl.
/// 
/// * Logs any errors returned from the stream, which could be done with
/// `TryStreamExt::instrument_err`, but this is already intercepting `poll_next`
/// so it's simple to do there.
struct StreamMonitor {
    stream: BoxBytesStream,
    span: Span,
    pos: u64,
    len: u64,
    errored: bool,
}

impl StreamMonitor {
    fn new(stream: BoxBytesStream, len: u64) -> Self {

        info!(
            http.response.body.bytes = len,
            "Download started"
        );

        Self { stream, pos: 0, len, span: Span::current(), errored: false }
    }
}

impl Stream for StreamMonitor {
    type Item = Result<Bytes, BoxError>;

    fn poll_next(self: std::pin::Pin<&mut Self>, cx: &mut std::task::Context<'_>) -> Poll<Option<Self::Item>> {
        let this = self.get_mut();
        let _entered = this.span.enter();
        let r = Pin::new(&mut this.stream).poll_next(cx);

        match &r {
            Poll::Pending => {},
            Poll::Ready(Some(Ok(bytes))) => {
                this.pos += bytes.len() as u64;
            }
            Poll::Ready(Some(Err(err))) => {
                error!(
                    http.response.body.bytes = this.len,
                    http.response.body.progress = this.pos,
                    "Response stream error: {}", Report(&**err as &(dyn Error + 'static))
                );
                this.errored = true;
            }
            Poll::Ready(None) => {}
        }

        r
    }
}

impl Drop for StreamMonitor {
    fn drop(&mut self) {
        let _entered = self.span.enter();

        let status = if self.pos >= self.len {
            "complete"
        } else if self.errored {
            "failed"
        } else {
            "canceled"
        };

        info!(
            http.response.body.bytes = self.len,
            http.response.body.progress = self.pos,
            zipstream.result = status,
            "Download {}", status
        );
    }
}

#[tokio::test]
async fn test_base_hyper_response() {
    use http_body_util::BodyExt;
    let req = Request::builder()
        .body(http_body_util::Empty::<Bytes>::new()).unwrap();

    let data = Bytes::from_static(b"0123456789");

    let res = hyper_response(&req, "application/test", "ETAG", "foo.zip", &data);

    assert_eq!(res.status(), StatusCode::OK);
    assert_eq!(res.headers().get(header::CONTENT_TYPE), Some(&header::HeaderValue::from_static("application/test")));
    assert_eq!(res.headers().get(header::CONTENT_DISPOSITION), Some(&header::HeaderValue::from_static("attachment; filename=\"foo.zip\"")));
    assert_eq!(res.headers().get(header::ETAG), Some(&header::HeaderValue::from_static("ETAG")));
    assert_eq!(res.headers().get(header::CONTENT_LENGTH), Some(&header::HeaderValue::from_static("10")));
    assert_eq!(res.into_body().collect().await.unwrap().to_bytes().as_ref(), b"0123456789");
}

#[tokio::test]
async fn test_range_hyper_response() {
    use http_body_util::BodyExt;

    let req = Request::builder()
        .header(header::RANGE, "bytes=4-8")
        .header(header::IF_RANGE, "ETAG")
        .body(http_body_util::Empty::<Bytes>::new()).unwrap();

    let data = Bytes::from_static(b"0123456789");

    let res = hyper_response(&req, "application/test", "ETAG", "foo.zip", &data);

    assert_eq!(res.status(), StatusCode::PARTIAL_CONTENT);
    assert_eq!(res.headers().get(header::CONTENT_TYPE), Some(&header::HeaderValue::from_static("application/test")));
    assert_eq!(res.headers().get(header::ETAG), Some(&header::HeaderValue::from_static("ETAG")));
    assert_eq!(res.headers().get(header::CONTENT_LENGTH), Some(&header::HeaderValue::from_static("5")));
    assert_eq!(res.headers().get(header::CONTENT_RANGE), Some(&header::HeaderValue::from_static("bytes 4-8/10")));
    assert_eq!(res.into_body().collect().await.unwrap().to_bytes().as_ref(), b"45678");
}

#[tokio::test]
async fn test_bad_if_range_hyper_response() {
    use http_body_util::BodyExt;

    let req = Request::builder()
        .header(header::RANGE, "bytes=4-8")
        .header(header::IF_RANGE, "WRONG")
        .body(http_body_util::Empty::<Bytes>::new()).unwrap();

    let data = Bytes::from_static(b"0123456789");

    let res = hyper_response(&req, "application/test", "ETAG", "foo.zip", &data);

    assert_eq!(res.status(), StatusCode::OK);
    assert_eq!(res.headers().get(header::CONTENT_LENGTH), Some(&header::HeaderValue::from_static("10")));
    assert_eq!(res.headers().get(header::CONTENT_RANGE), None);
    assert_eq!(res.into_body().collect().await.unwrap().to_bytes().as_ref(), b"0123456789");
}

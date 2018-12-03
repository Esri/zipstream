use std::sync::Arc;
use futures::{Future, stream, Stream};
use bytes::Bytes;
use rusoto_s3::{ S3, GetObjectRequest, GetObjectError };

type BoxBytesStream = Box<Stream<Item=Bytes, Error=BoxError> + Send>;
type BoxError = Box<dyn std::error::Error + 'static + Sync + Send>;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Range {
    pub start: u64,
    pub end: u64,
}

impl Range {
    fn take_prefix(&mut self, len: u64) -> Option<Range> {
        let prefix = if self.start < len {
            Some(Range { start: self.start, end: self.end.min(len) })
        } else { None };
        self.start = self.start.saturating_sub(len);
        self.end = self.end.saturating_sub(len);
        prefix
    }

    fn len(&self) -> u64 { self.end - self.start }

    fn to_http_range_header(&self) -> String {
        format!("bytes={}-{}", self.start, self.end-1)
    }
}

pub trait StreamRange {
    fn len(&self) -> u64;
    fn stream_range(&self, range: Range) -> BoxBytesStream;
}

impl StreamRange for Bytes {
    fn len(&self) -> u64 { Bytes::len(self) as u64 }
    fn stream_range(&self, range: Range) -> BoxBytesStream {
        Box::new(stream::once(Ok(self.slice(range.start as usize, range.end as usize))))
    }
}

pub struct S3Object {
    pub s3: Arc<S3>,
    pub bucket: String,
    pub key: String,
    pub len: u64,
}

impl StreamRange for S3Object {
    fn len(&self) -> u64 { self.len }
    fn stream_range(&self, range: Range) -> BoxBytesStream {
        let req = GetObjectRequest { 
            bucket: self.bucket.clone(),
            key: self.key.clone(),
            range: Some(range.to_http_range_header()),
            ..GetObjectRequest::default()
        };

        let len = range.len();
        let url = format!("s3://{}/{}", self.bucket, self.key);

        log::debug!("Get S3 file {} {}", url, range.to_http_range_header());

        Box::new(self.s3.get_object(req)
            .map_err(|err| {
                match &err {
                    GetObjectError::Unknown(resp) => format!("S3 GetObject failed with {} {}", resp.status, String::from_utf8_lossy(&resp.body[..])),
                    err => format!("S3 GetObject failed with {}", err),
                }.into()
            })
            .map(move |res| {
                log::debug!("S3 get complete for {}", url);

                if res.content_length != Some(len as i64) {
                    log::error!("S3 file size mismatch for {}, expected {:?}, got {:?}", url, len, res.content_length)
                }

                res.body.unwrap().map(Bytes::from).map_err(|err| {
                    format!("S3 stream failed with {}", err).into()
                })
            })
            .flatten_stream())
    }
}

pub struct Concatenated(pub Vec<Box<dyn StreamRange>>);

impl StreamRange for Concatenated {
    fn len(&self) -> u64 { self.0.iter().map(|x| x.len()).sum() }
    fn stream_range(&self, mut range: Range) -> BoxBytesStream {
        let mut streams = Vec::new();
        for part in &self.0 {
            if range.len() == 0 { break; }

            if let Some(inner_range) = range.take_prefix(part.len()) {
                streams.push(part.stream_range(inner_range));
            }
        }
        Box::new(stream::iter_ok::<_, BoxError>(streams.into_iter()).flatten())
    }
}

use hyper::{Request, Response, Body, StatusCode, header};

pub fn parse_range(range_val: &str, total_len: u64) -> Result<Option<Range>, &'static str> {
    if !range_val.starts_with("bytes=") {
        return Err("invalid range unit");
    }

    let range_val = &range_val["bytes=".len()..].trim();

    if range_val.contains(",") {
        return Ok(None); // multiple ranges unsupported, but it's legal to just ignore the header
    }

    if range_val.starts_with("-") {
        let s = range_val[1..].parse::<u64>().map_err(|_| "invalid range number")?;
        
        if s >= total_len {
            return Ok(None);
        }

        Ok(Some(Range { start: total_len-s, end: total_len }))
    } else if range_val.ends_with("-") {
        let s = range_val[..range_val.len()-1].parse::<u64>().map_err(|_| "invalid range number")?;
        
        if s >= total_len {
            return Ok(None);
        }

        Ok(Some(Range { start: s, end: total_len}))
    } else if let Some(h) = range_val.find("-") {
        let s = range_val[..h].parse::<u64>().map_err(|_| "invalid range number")?;
        let e = range_val[h+1..].parse::<u64>().map_err(|_| "invalid range number")?;

        if e >= total_len || s > e {
            return Ok(None);
        }

        Ok(Some(Range { start: s, end: e+1 }))
    } else {
        return Err("invalid range");
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

pub fn hyper_response(req: &Request<Body>, content_type: &str, etag: &str, filename: &str, data: &StreamRange) -> Response<Body> {
    let full_len = data.len();
    let full_range = Range { start: 0, end: full_len };

    let range = req.headers().get(hyper::header::RANGE)
        .filter(|_| req.headers().get(hyper::header::IF_RANGE).map_or(true, |val| val == etag))
        .and_then(|v| v.to_str().ok())
        .and_then(|v| parse_range(v, full_len).ok())
        .and_then(|x| x);

    let mut res = Response::builder();
    res.header(header::CONTENT_TYPE, content_type);
    res.header(header::ACCEPT_RANGES, "bytes");
    res.header(header::ETAG, etag);
    res.header(header::CONTENT_DISPOSITION, format!("attachment; filename=\"{}\"", filename));

    if let Some(range) = range {
        res.status(StatusCode::PARTIAL_CONTENT);
        res.header(header::CONTENT_RANGE, format!("bytes {}-{}/{}", range.start, range.end - 1, full_len));
        log::info!("Serving range {:?}", range);
    }

    let range = range.unwrap_or(full_range);

    res.header(header::CONTENT_LENGTH, range.len());

    let stream = data.stream_range(range).inspect_err(|err| {
        log::error!("Response stream error: {}", err);
    });

    res.body(Body::wrap_stream(stream)).unwrap()
}
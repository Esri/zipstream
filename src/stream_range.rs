// © 2019 3D Robotics. License: Apache-2.0
use std::sync::Arc;
use futures::{Future, stream, Stream};
use bytes::Bytes;
use rusoto_s3::{ S3, GetObjectRequest, GetObjectError };

type BoxBytesStream = Box<dyn Stream<Item=Bytes, Error=BoxError> + Send>;
type BoxError = Box<dyn std::error::Error + 'static + Sync + Send>;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Range {
    pub start: u64,
    pub end: u64,
}

impl Range {
    pub fn take_prefix(&mut self, len: u64) -> Option<Range> {
        let prefix = if self.start < len {
            Some(Range { start: self.start, end: self.end.min(len) })
        } else { None };
        self.start = self.start.saturating_sub(len);
        self.end = self.end.saturating_sub(len);
        prefix
    }

    pub fn len(&self) -> u64 { self.end - self.start }

    pub fn to_http_range_header(&self) -> String {
        format!("bytes={}-{}", self.start, self.end-1)
    }
}

/// An abstract stream of bytes that supports serving a range as a futures::Stream
pub trait StreamRange {
    /// Total number of bytes
    fn len(&self) -> u64;

    /// Create a stream that produces a range of the data
    fn stream_range(&self, range: Range) -> BoxBytesStream;
}

impl StreamRange for Bytes {
    fn len(&self) -> u64 { Bytes::len(self) as u64 }
    fn stream_range(&self, range: Range) -> BoxBytesStream {
        Box::new(stream::once(Ok(self.slice(range.start as usize, range.end as usize))))
    }
}

/// Implements `StreamRange` to serve an object from an S3 bucket
pub struct S3Object {
    pub s3: Arc<dyn S3>,
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

        Box::new(self.s3.get_object(req)
            .map_err(|err| {
                match &err {
                    GetObjectError::Unknown(resp) => format!("S3 GetObject failed with {} {}", resp.status, String::from_utf8_lossy(&resp.body[..])),
                    err => format!("S3 GetObject failed with {}", err),
                }.into()
            })
            .map(move |res| {
                log::info!("S3 get complete for {}", url);

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

/// A `StreamRange` constructed by concatentating multiple other `StreamRange` trait objects
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

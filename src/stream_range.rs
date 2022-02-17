// © 2019 3D Robotics. License: Apache-2.0
use aws_sdk_s3 as s3;
use std::pin::Pin;
use futures::{ future, TryFutureExt, stream, Stream, StreamExt, TryStreamExt };
use bytes::Bytes;

type BoxBytesStream = Pin<Box<dyn Stream<Item = Result<Bytes, BoxError>> + Send +'static>>;
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

    pub fn to_http_range_header(self) -> String {
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
        Box::pin(stream::once(future::ok(self.slice(range.start as usize..range.end as usize))))
    }
}

/// Implements `StreamRange` to serve an object from an S3 bucket
pub struct S3Object {
    pub client: s3::Client,
    pub bucket: String,
    pub key: String,
    pub len: u64,
}

impl StreamRange for S3Object {
    fn len(&self) -> u64 { self.len }
    fn stream_range(&self, range: Range) -> BoxBytesStream {
        let client = self.client.clone();
        let bucket = self.bucket.clone();
        let key = self.key.clone();

        let stream = async move {
            let len = range.len();
            let url = format!("s3://{}/{}", bucket, key);

            let req = client.get_object()
                .bucket(bucket)
                .key(key)
                .range(range.to_http_range_header());

            let res = req.send().await
                .map_err(|err| { format!("S3 GetObject failed with {}", err) })?;
            
            log::info!("S3 get complete for {}", url);

            if res.content_length != (len as i64) {
                log::error!("S3 file size mismatch for {}, expected {:?}, got {:?}", url, len, res.content_length)
            }

            Ok(res.body.map_err(|err| {
                format!("S3 stream failed with {}", err).into()
            }))
        };

        Box::pin(stream.try_flatten_stream())
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
        Box::pin(stream::iter(streams.into_iter()).flatten())
    }
}

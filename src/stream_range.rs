// © 2019 3D Robotics. License: Apache-2.0
use aws_sdk_s3 as s3;
use s3::primitives::ByteStream;
use std::{pin::Pin, task::{Context, Poll}};
use futures::{ future::{self, lazy}, FutureExt, TryFutureExt, stream, Stream, StreamExt };
use bytes::Bytes;

pub type BoxBytesStream = Pin<Box<dyn Stream<Item = Result<Bytes, BoxError>> + Send +'static>>;
pub type BoxError = Box<dyn std::error::Error + 'static + Sync + Send>;

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

        // The inner `Future` that makes the S3 request is large, so
        // lazily allocate it only when we begin streaming the specific file.
        Box::pin(lazy(move |_| {
            Box::pin(async move {
                let len = range.len();
                let url = format!("s3://{}/{}", bucket, key);

                let req = client.get_object()
                    .bucket(bucket)
                    .key(key)
                    .range(range.to_http_range_header());

                let res = req.send().await
                    .map_err(|err| { format!("S3 GetObject failed with {}", err) })?;

                log::info!("S3 get complete for {}", url);

                if res.content_length != Some(len as i64) {
                    log::error!("S3 file size mismatch for {}, expected {:?}, got {:?}", url, len, res.content_length)
                }

                Ok(ByteStreamWrap(res.body))
            })
        }).flatten().try_flatten_stream())
    }
}

/// Newtype wrapper implementing [`Stream`] for [`ByteStream`].
///
/// https://github.com/smithy-lang/smithy-rs/pull/2983 removed the `Stream` implementation.
pub struct ByteStreamWrap(ByteStream);

impl Stream for ByteStreamWrap {
    type Item = Result<Bytes, BoxError>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        Pin::new(&mut self.0).poll_next(cx).map_err(|e| e.into())
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

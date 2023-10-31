pub mod stream_range;
pub mod serve_range;
pub mod zip;
pub mod upstream;
pub mod s3url;


#[derive(Clone)]
pub struct Config {
    pub upstream: String,
    pub strip_prefix: String,
    pub via_zip_stream_header_value: String,
}

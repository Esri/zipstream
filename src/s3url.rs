// © 2019 3D Robotics. License: Apache-2.0
use std::fmt;
use serde::de;
use regex::Regex;
use std::str::FromStr;
use lazy_static::lazy_static;

/// A reference to a file on Amazon S3 by bucket and key.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct S3Url {
    pub bucket: String,
    pub key: String
}

impl fmt::Display for S3Url {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "s3://{}/{}", self.bucket, self.key)
    }
}

#[derive(Debug, PartialEq, Eq, Clone)]
pub struct ParseS3UrlError;

impl fmt::Display for ParseS3UrlError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "Invalid s3:// URL")
    }
}

impl FromStr for S3Url {
    type Err = ParseS3UrlError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        lazy_static! {
            static ref re: Regex = Regex::new(r"^s3://([^/]+)/(.+)$").unwrap();
        }

        let captures = re.captures(s).ok_or(ParseS3UrlError)?;

        Ok(S3Url {
            bucket: captures.get(1).unwrap().as_str().to_owned(),
            key: captures.get(2).unwrap().as_str().to_owned()
        })
    }
}

impl<'de> de::Deserialize<'de> for S3Url {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
        where D: de::Deserializer<'de>
    {
        let s = String::deserialize(deserializer)?;
        FromStr::from_str(&s).map_err(de::Error::custom)
    }
}

#[test]
fn test_s3url() {
    let parsed = "s3://bucketname/bar/baz.jpg".parse::<S3Url>();
    assert_eq!(parsed, Ok(S3Url { bucket: "bucketname".into(), key: "bar/baz.jpg".into() }));
    assert_eq!(parsed.unwrap().to_string(), "s3://bucketname/bar/baz.jpg");

    assert_eq!("http://foo/bar".parse::<S3Url>(), Err(ParseS3UrlError));
    assert_eq!("s3://foo".parse::<S3Url>(), Err(ParseS3UrlError));
}

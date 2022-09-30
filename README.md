# zipstream

A microservice to serve ZIP archives generated on-the-fly from files on Amazon S3.

It works by forwarding requests to an upstream HTTP server that returns a JSON manifest of files to
include, then generates the archive by requesting content from S3 as the download progresses.

Features:
  * Zip64 support (archives and files > 4GiB)
  * Content-length headers for an accurate download progress bar
  * Range requests so that partial or failed downloads can be resumed

In order to compute the length ahead of time and to support seeking to any position, it imposes a few limitations:
  * Size of each archive member and its CRC32 must be known ahead of time and included in the manifest.
  * Archive members are not compressed. (If serving files that are already compressed, ZIP compression would not have any benefit anyway)

### Usage

```
zipstream --listen <ip:port> --upstream <URL> --header-value <header-value> --strip-prefix <strip-prefix> 
```

  * `--listen <ip:port>`               IP:port to listen for HTTP connections [default: `127.0.0.1:3000`]
  * `--upstream <URL>`                 Upstream server that provides zip file manifests
  * `--header-value <header-value>`    Value passed in the X-Via-Zip-Stream header on the request to the upstream server [default: `true`]
  * `--strip-prefix <strip-prefix>`    Remove a required prefix from the URL path before proxying to upstream server [default: `''`]

Incoming requests are proxied to the upstream server. If the response from the upstream server does not include the `X-Zip-Stream: true` header, the response is passed through to the client as-is. When this header is included, the response parsed as a manifest of files to include in a zip file which is streamed back to the client.

The manifest is JSON in the following format:

```json
{
  "filename": "test.zip", // The download filename returned in a Content-disposition: attachment header
  "entries": [
    {
      "archive_name": "file1.jpg", // The file name as it will be included in the zip
      "length": 7293198, // Exact length in bytes
      "crc": 2113672619, // CRC32 checksum of the file content
      "source": "s3://bucketname/objectpath", // Source location of the file on S3
      "last_modified": "2020-04-24T19:12:24.268Z" // Timestamp to use as the last modified time in the archive
    },
    ...
  ]
}
```

### Demo

```console
$ cargo run --example demo-server & # a stand-in for your application

$ export AWS_REGION=us-east-1
$ export AWS_ACCESS_KEY_ID=...
$ export AWS_SECRET_ACCESS_KEY=...
$ cargo run -- --listen 127.0.0.1:3000 --upstream http://localhost:3001 &

$ curl http://localhost:3000/foo/bar/test.zip -o test.zip
Demo server got a request for /foo/bar/test.zip
[2022-09-29T22:22:09Z INFO  zipstream::upstream] Streaming zip file test.zip: 2 entries, 258 bytes
[2022-09-29T22:22:10Z INFO  zipstream::stream_range] S3 get complete for s3://sitescan-test/zipstream-demo/test1.txt
[2022-09-29T22:22:10Z INFO  zipstream::stream_range] S3 get complete for s3://sitescan-test/zipstream-demo/test2.txt

$ unzip -t test.zip
Archive:  test.zip
    testing: test1.txt                OK
    testing: test2.txt                OK
No errors detected in compressed data of test.zip.
```

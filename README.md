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

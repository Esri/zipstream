use bytes::{Bytes, BytesMut, BufMut};
use crate::stream_range::{ self, StreamRange };
use chrono::{DateTime, Utc, Datelike, Timelike};

/// A file to be included in a zip archive.
pub struct ZipEntry {
    /// Filename within the archive.
    pub archive_path: String,

    /// Contents of file.
    pub data: Box<dyn StreamRange>,

    /// CRC32 checksum of the file contents.
    /// This must be precomputed because it's included in the file header.
    pub crc: u32,

    /// Last modified date.
    /// If you want the zip file to be reproducible for Range requests, do
    /// not default to the current time.
    pub last_modified: DateTime<Utc>,
}

/// Options passed to `zip_stream`
#[derive(Clone, PartialEq, Debug, Default)]
pub struct ZipOptions {
    /// Create a zip file using zip64 extensions even if the file will be under 2^32 bytes.
    /// Otherwise, zip64 will be used only if necessary.
    pub force_zip64: bool,
}

// Zip format spec:
// https://pkware.cachefly.net/webdocs/casestudies/APPNOTE.TXT

const ZIP64_VERSION: u8 = 45;
const BASE_VERSION: u8 = 20;

fn zip_date(t: DateTime<Utc>) -> u16 {
    let year = t.year().saturating_sub(1980) as u16;
    let month = t.month() as u16;
    let day = t.day() as u16;
    day | month << 5 | year << 9
}

fn zip_time(t: DateTime<Utc>) -> u16 {
    let second = (t.second() / 2) as u16;
    let minute = t.minute() as u16;
    let hour = t.hour() as u16;
    second | minute << 5 | hour << 11  
}

#[test]
fn test_zip_date_time() {
    let t = "2006-10-11T15:40:56Z".parse::<DateTime<Utc>>().unwrap();
    assert_eq!(zip_time(t), 0x7d1c);
    assert_eq!(zip_date(t), 0x354b);
}

fn local_file_header(file: &ZipEntry, force_zip64: bool) -> Bytes {
    let needs_zip64 = file.data.len() >= 0xFFFFFFFF || force_zip64;
    let mut buf = BytesMut::with_capacity(30 + file.archive_path.len() + if needs_zip64 { 20 } else { 0 });

    buf.put_u32_le(0x04034b50); // local file header signature
    buf.put_u16_le(if needs_zip64 { ZIP64_VERSION } else { BASE_VERSION } as u16); //  version needed to extract
    buf.put_u16_le(0); // general purpose bit flag
    buf.put_u16_le(0); // compression method
    buf.put_u16_le(zip_time(file.last_modified)); // last mod file time
    buf.put_u16_le(zip_date(file.last_modified)); // last mod file date
    buf.put_u32_le(file.crc); // crc-32

    if needs_zip64 {
        buf.put_u32_le(0xFFFFFFFF); // compressed size
        buf.put_u32_le(0xFFFFFFFF); // uncompressed size
    } else {
        buf.put_u32_le(file.data.len() as u32); // compressed size
        buf.put_u32_le(file.data.len() as u32); // uncompressed size
    }

    buf.put_u16_le(file.archive_path.len() as u16); // file name length
    buf.put_u16_le(if needs_zip64 { 20 } else { 0 }); // extra field length

    // file name
    buf.put_slice(file.archive_path.as_bytes());

    if needs_zip64 {
        buf.put_u16_le(0x0001); // Zip64 extended information
        buf.put_u16_le(16); // Size of this "extra" block
        buf.put_u64_le(file.data.len()); // Original uncompressed file size
        buf.put_u64_le(file.data.len()); // Size of compressed data
    }

    buf.freeze()
}

fn central_directory_file_header(file: &ZipEntry, offset: u64, force_zip64: bool) -> Bytes {
    let needs_zip64 = file.data.len() >= 0xFFFFFFFF || offset >= 0xFFFFFFFF || force_zip64;
    let mut buf = BytesMut::with_capacity(46 + file.archive_path.len() + if needs_zip64 { 28 } else { 0 });

    buf.put_u32_le(0x02014b50); // central file header signature
    buf.put_u8(BASE_VERSION); // version made by = zip spec 4.5
    buf.put_u8(3); // version made by = unix
    buf.put_u16_le(if needs_zip64 { ZIP64_VERSION } else { BASE_VERSION } as u16); //  version needed to extract
    buf.put_u16_le(0); // general purpose bit flag
    buf.put_u16_le(0); // compression method
    buf.put_u16_le(zip_time(file.last_modified)); // last mod file time
    buf.put_u16_le(zip_date(file.last_modified)); // last mod file date
    buf.put_u32_le(file.crc); // crc-32

    if needs_zip64 {
        buf.put_u32_le(0xFFFFFFFF); // compressed size
        buf.put_u32_le(0xFFFFFFFF); // uncompressed size
    } else {
        buf.put_u32_le(file.data.len() as u32); // compressed size
        buf.put_u32_le(file.data.len() as u32); // uncompressed size
    }
    
    buf.put_u16_le(file.archive_path.len() as u16); // file name length
    buf.put_u16_le(if needs_zip64 { 28 } else { 0 }); // extra field length
    buf.put_u16_le(0); // file comment length
    buf.put_u16_le(0); // disk number start
    buf.put_u16_le(0); // internal file attributes
    buf.put_u32_le(0x81A40000); // external file attributes (-rw-r--r--)

    if needs_zip64 {
        buf.put_u32_le(0xFFFFFFFF);
    } else {
        buf.put_u32_le(offset as u32); // relative offset of local header
    }

    buf.extend(file.archive_path.as_bytes());

    if needs_zip64 {
        buf.put_u16_le(0x0001); // Zip64 extended information
        buf.put_u16_le(24); // Size of this "extra" block
        buf.put_u64_le(file.data.len()); // Original uncompressed file size
        buf.put_u64_le(file.data.len()); // Size of compressed data
        buf.put_u64_le(offset); // Offset of local header record
    }

    buf.freeze()
}

fn end_of_central_directory(central_directory_offset: u64, size_of_central_directory: u64, num_entries: u64, force_zip64: bool) -> Bytes {
    let mut buf = BytesMut::with_capacity(56 + 20 + 22);

    if num_entries >= 0xFFFF || size_of_central_directory >= 0xFFFFFFFF || central_directory_offset >= 0xFFFFFFFF || force_zip64 {
        // Zip64 end of central directory record
        buf.put_u32_le(0x06064b50); //  signature
        buf.put_u64_le(56-12); // size of zip64 end of central directory record
        buf.put_u16_le(ZIP64_VERSION as u16); // version made by
        buf.put_u16_le(ZIP64_VERSION as u16); // version needed to extract
        buf.put_u32_le(0); //   number of this disk
        buf.put_u32_le(0); //   number of the disk with the start of the central directory
        buf.put_u64_le(num_entries); //   total number of entries in the central directory on this disk
        buf.put_u64_le(num_entries); //   total number of entries in the central directory
        buf.put_u64_le(size_of_central_directory); //   size of the central directory
        buf.put_u64_le(central_directory_offset); //   offset of start of central directory with respect to the starting disk number

        // Zip64 end of central directory locator ()
        buf.put_u32_le(0x07064b50); //  signature
        buf.put_u32_le(0); // number of the disk with the start of the zip64 end of central directory
        buf.put_u64_le(central_directory_offset + size_of_central_directory); // relative offset of the zip64 end of central directory record
        buf.put_u32_le(1); //  total number of disks
    }

    let num_entries_16 = if num_entries >= 0xFFFF { 0xFFFF } else { num_entries as u16 };
    let size_of_central_directory_32 = if size_of_central_directory >= 0xFFFFFFFF { 0xFFFFFFFF } else { size_of_central_directory as u32 };
    let central_directory_offset_32 = if central_directory_offset >= 0xFFFFFFFF { 0xFFFFFFFF } else { central_directory_offset as u32};

    // End of central_directory (22 bytes)
    buf.put_u32_le(0x06054b50); //  end of central dir signature
    buf.put_u16_le(0); // number of this disk
    buf.put_u16_le(0); // number of the disk with the start of the central directory
    buf.put_u16_le(num_entries_16); // total number of entries in the central directory on this disk
    buf.put_u16_le(num_entries_16); // total number of entries in the central directory
    buf.put_u32_le(size_of_central_directory_32); // size of the central directory
    buf.put_u32_le(central_directory_offset_32); // offset of start of central directory with respect to the starting disk number
    buf.put_u16_le(0); //  .ZIP file comment length

    buf.freeze()
}

/// Create a `StreamRange` that produces a ZIP file with the passed entries.
pub fn zip_stream(files: impl IntoIterator<Item = ZipEntry>, options: ZipOptions) -> impl StreamRange {
    let mut data_parts: Vec<Box<dyn StreamRange>> = Vec::new();
    let mut central_directory_parts: Vec<Box<dyn StreamRange>> = Vec::new();
    let mut offset = 0;

    for file in files {
        let local_header = local_file_header(&file, options.force_zip64);
        let central_header = central_directory_file_header(&file, offset, options.force_zip64);

        offset += local_header.len() as u64 + file.data.len() as u64;

        data_parts.push(Box::new(local_header));
        data_parts.push(file.data);

        central_directory_parts.push(Box::new(central_header));
    }

    let num_entries = central_directory_parts.len() as u64;
    let size_of_central_directory = central_directory_parts.iter().map(|x| x.len() as u64).sum();

    data_parts.extend(central_directory_parts.into_iter());
    data_parts.push(Box::new(end_of_central_directory(offset, size_of_central_directory, num_entries, options.force_zip64)));

    stream_range::Concatenated(data_parts)
}

#[cfg(test)]
mod test {
    use super::*;
    use bytes::{Bytes};
    use crate::stream_range::{ Range, StreamRange };
    use futures::{Future, Stream};
    use std::process::Command;

    fn test_entries() -> Vec<ZipEntry> {
        vec![
            ZipEntry {
                archive_path: "foo.txt".into(),
                data: Box::new(Bytes::from_static(&b"xx"[..])),
                crc: 0xf8e1180f,
                last_modified: "2006-11-10T15:40:56Z".parse::<DateTime<Utc>>().unwrap(),
            },
            ZipEntry {
                archive_path: "bar.txt".into(),
                data: Box::new(Bytes::from_static(&b"ABC"[..])),
                crc: 0xa3830348,
                last_modified: "2018-12-06T20:15:59Z".parse::<DateTime<Utc>>().unwrap(),
            }
        ]
    }

    /// Exhaustively test that all subranges return the same data as a slice of the whole.
    #[test]
    fn test_concat() {
        let zip = zip_stream(test_entries(), ZipOptions::default());
        let buf = zip.stream_range(Range { start: 0, end: zip.len() }).concat2().wait().unwrap();

        assert_eq!(zip.len(), buf.len() as u64);

        for start in 0..zip.len() {
            for end in start..zip.len() {
                println!("{} {}", start, end);
                let slice = zip.stream_range(Range { start, end }).concat2().wait().unwrap();
                assert_eq!(buf.slice(start as usize, end as usize), slice, "{} {}", start, end);
            }
        }
    }

    /// Generate a 32-bit zip file and check it with zipinfo, unzip, and python.
    #[test]
    fn test_zip32() {
        let zip = zip_stream(test_entries(), ZipOptions { force_zip64: false });

        let buf = zip.stream_range(Range { start: 0, end: zip.len() }).concat2().wait().unwrap();
        std::fs::write("test.zip", &buf).unwrap();

        assert!(Command::new("zipinfo").arg("-v").arg("test.zip").status().unwrap().success());
        assert!(Command::new("unzip").arg("-t").arg("test.zip").status().unwrap().success());
        assert!(Command::new("python3").arg("-m").arg("zipfile").arg("-t").arg("test.zip").status().unwrap().success());
    }

    /// Generate a 64-bit zip file and check it with zipinfo, unzip, and python.
    #[test]
    fn test_zip64() {
        let zip = zip_stream(test_entries(), ZipOptions { force_zip64: true });

        let buf = zip.stream_range(Range { start: 0, end: zip.len() }).concat2().wait().unwrap();
        std::fs::write("test64.zip", &buf).unwrap();

        assert!(Command::new("zipinfo").arg("-v").arg("test64.zip").status().unwrap().success());
        assert!(Command::new("unzip").arg("-t").arg("test64.zip").status().unwrap().success());
        assert!(Command::new("python3").arg("-m").arg("zipfile").arg("-t").arg("test64.zip").status().unwrap().success());
    }
    
}
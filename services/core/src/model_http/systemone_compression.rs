//! One-layer System One Content-Encoding boundary. Call from a bounded blocking
//! worker after request admission and before JSON/multipart parsing. Never infer
//! an encoding from magic bytes or recursively decode the resulting content.
use axum::http::{HeaderMap, header};
use std::io::Read;

pub(crate) const MAX_BODY: usize = 32 * 1024 * 1024;
const MAX_ZSTD_WINDOW_LOG: u32 = 23;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum CompressionError {
    UnsupportedEncoding,
    Malformed,
    TooLarge,
}
impl CompressionError {
    pub(crate) fn status(self) -> u16 {
        match self {
            Self::UnsupportedEncoding => 415,
            Self::Malformed => 400,
            Self::TooLarge => 413,
        }
    }
    pub(crate) fn code(self) -> &'static str {
        match self {
            Self::UnsupportedEncoding => "unsupported_content_encoding",
            Self::Malformed => "malformed_compressed_body",
            Self::TooLarge => "body_too_large",
        }
    }
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Encoding {
    Raw,
    Gzip,
    Zstd,
}
fn encoding(headers: &HeaderMap) -> Result<Encoding, CompressionError> {
    let mut values = headers.get_all(header::CONTENT_ENCODING).iter();
    let Some(value) = values.next() else {
        return Ok(Encoding::Raw);
    };
    if values.next().is_some() {
        return Err(CompressionError::UnsupportedEncoding);
    }
    let value = value
        .to_str()
        .map_err(|_| CompressionError::UnsupportedEncoding)?;
    let value = value.trim_matches([' ', '\t']);
    // Content codings are case insensitive tokens. A comma, an empty value,
    // parameters, or duplicate header is not one supported coding.
    if value.eq_ignore_ascii_case("gzip") || value.eq_ignore_ascii_case("x-gzip") {
        Ok(Encoding::Gzip)
    } else if value.eq_ignore_ascii_case("zstd") {
        Ok(Encoding::Zstd)
    } else {
        Err(CompressionError::UnsupportedEncoding)
    }
}

/// The caller must also cap raw network collection at MAX_BODY. This second
/// check protects callers with already-buffered bodies. Both limits are inclusive.
/// On success remove Content-Encoding and the original Content-Length before
/// reconstructing a request for native JSON or multipart parsing.
pub(crate) fn decode(headers: &HeaderMap, raw: &[u8]) -> Result<Vec<u8>, CompressionError> {
    decode_limited(headers, raw, MAX_BODY)
}
fn decode_limited(
    headers: &HeaderMap,
    raw: &[u8],
    limit: usize,
) -> Result<Vec<u8>, CompressionError> {
    if raw.len() > limit {
        return Err(CompressionError::TooLarge);
    }
    match encoding(headers)? {
        Encoding::Raw => Ok(raw.to_vec()),
        Encoding::Gzip => {
            if raw.is_empty() {
                return Err(CompressionError::Malformed);
            }
            // RFC 1952 gzip files may contain concatenated members. Decode ALL
            // members with their trailers; a single-member decoder would silently
            // discard an appended member, corrupted trailer or garbage suffix.
            let mut decoder = flate2::bufread::MultiGzDecoder::new(raw);
            let decoded = read_bounded(&mut decoder, limit)?;
            if !decoder.into_inner().is_empty() {
                return Err(CompressionError::Malformed);
            }
            Ok(decoded)
        }
        Encoding::Zstd => {
            if raw.is_empty() {
                return Err(CompressionError::Malformed);
            }
            let mut decoder = zstd::stream::read::Decoder::with_buffer(raw)
                .map_err(|_| CompressionError::Malformed)?;
            // Configure the mature decoder BEFORE processing attacker headers;
            // never allocate the advertised window then check it afterward.
            decoder
                .window_log_max(MAX_ZSTD_WINDOW_LOG)
                .map_err(|_| CompressionError::Malformed)?;
            // Default zstd decoder validates concatenated frames until input EOF.
            let decoded = read_bounded(&mut decoder, limit)?;
            if !decoder.finish().is_empty() {
                return Err(CompressionError::Malformed);
            }
            Ok(decoded)
        }
    }
}
fn read_bounded(reader: &mut impl Read, limit: usize) -> Result<Vec<u8>, CompressionError> {
    let mut decoded = Vec::new();
    let mut buffer = [0_u8; 8192];
    loop {
        let read = reader
            .read(&mut buffer)
            .map_err(|_| CompressionError::Malformed)?;
        if read == 0 {
            return Ok(decoded);
        }
        if read > limit.saturating_sub(decoded.len()) {
            return Err(CompressionError::TooLarge);
        }
        decoded.extend_from_slice(&buffer[..read]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    fn headers(encoding: &str) -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert(header::CONTENT_ENCODING, encoding.parse().unwrap());
        headers
    }
    fn gzip(bytes: &[u8]) -> Vec<u8> {
        let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
        encoder.write_all(bytes).unwrap();
        encoder.finish().unwrap()
    }
    fn zstd(bytes: &[u8]) -> Vec<u8> {
        zstd::stream::encode_all(bytes, 1).unwrap()
    }

    #[test]
    fn systemone_compression_raw_and_supported_codings() {
        let input = br#"{"state":"hello"}"#;
        assert_eq!(decode(&HeaderMap::new(), input).unwrap(), input);
        for coding in ["gzip", "x-gzip", "GZip", "\tgzip "] {
            assert_eq!(decode(&headers(coding), &gzip(input)).unwrap(), input);
        }
        assert_eq!(decode(&headers("ZSTD"), &zstd(input)).unwrap(), input);
        // Raw data with a compression signature is never guessed/decompressed.
        let compressed = gzip(input);
        assert_eq!(decode(&HeaderMap::new(), &compressed).unwrap(), compressed);
    }
    #[test]
    fn systemone_compression_rejects_multiple_or_unsupported_codings() {
        for coding in [
            "",
            "identity",
            "br",
            "deflate",
            "gzip, zstd",
            "gzip,gzip",
            "gzip;level=1",
        ] {
            assert_eq!(
                decode(&headers(coding), b"{}"),
                Err(CompressionError::UnsupportedEncoding)
            );
        }
        let mut h = headers("gzip");
        h.append(header::CONTENT_ENCODING, "gzip".parse().unwrap());
        assert_eq!(
            decode(&h, &gzip(b"{}")),
            Err(CompressionError::UnsupportedEncoding)
        );
    }
    #[test]
    fn systemone_compression_gzip_multimember_is_one_coding() {
        let mut compressed = gzip(b"{\"state\":");
        compressed.extend(gzip(b"\"hello\"}"));
        assert_eq!(
            decode(&headers("gzip"), &compressed).unwrap(),
            b"{\"state\":\"hello\"}"
        );
        compressed.push(0);
        assert_eq!(
            decode(&headers("gzip"), &compressed),
            Err(CompressionError::Malformed)
        );
    }
    #[test]
    fn systemone_compression_validates_every_gzip_member_and_crc() {
        let encoded = gzip(b"{}");
        for remove in 1..=8 {
            assert_eq!(
                decode(&headers("gzip"), &encoded[..encoded.len() - remove]),
                Err(CompressionError::Malformed)
            );
        }
        let mut corrupt = encoded.clone();
        let at = corrupt.len() - 8;
        corrupt[at] ^= 1;
        assert_eq!(
            decode(&headers("gzip"), &corrupt),
            Err(CompressionError::Malformed)
        );
        let mut two = encoded;
        two.extend(corrupt);
        assert_eq!(
            decode(&headers("gzip"), &two),
            Err(CompressionError::Malformed)
        );
    }
    #[test]
    fn systemone_compression_zstd_frames_and_tail_are_checked() {
        let mut encoded = zstd(b"{\"state\":");
        encoded.extend(zstd(b"\"hi\"}"));
        assert_eq!(
            decode(&headers("zstd"), &encoded).unwrap(),
            b"{\"state\":\"hi\"}"
        );
        encoded.push(0);
        assert_eq!(
            decode(&headers("zstd"), &encoded),
            Err(CompressionError::Malformed)
        );
        let encoded = zstd(b"{}");
        assert_eq!(
            decode(&headers("zstd"), &encoded[..encoded.len() - 1]),
            Err(CompressionError::Malformed)
        );
    }
    #[test]
    fn systemone_compression_bad_or_empty_encoded_bodies_fail() {
        for coding in ["gzip", "zstd"] {
            for raw in [b"".as_slice(), b"not compressed".as_slice()] {
                assert_eq!(
                    decode(&headers(coding), raw),
                    Err(CompressionError::Malformed)
                );
            }
        }
        // A valid stream that expands to an empty body is a parser decision.
        assert_eq!(decode(&headers("gzip"), &gzip(b"")).unwrap(), b"");
        assert_eq!(decode(&headers("zstd"), &zstd(b"")).unwrap(), b"");
    }
    #[test]
    fn systemone_compression_limits_both_wire_and_expanded_body() {
        let plain = vec![b'x'; 1024];
        for (coding, encoded) in [("gzip", gzip(&plain)), ("zstd", zstd(&plain))] {
            assert!(encoded.len() < plain.len());
            assert_eq!(
                decode_limited(&headers(coding), &encoded, 1024).unwrap(),
                plain
            );
            assert_eq!(
                decode_limited(&headers(coding), &encoded, 1023),
                Err(CompressionError::TooLarge)
            );
            assert_eq!(
                decode_limited(&headers(coding), &encoded, encoded.len() - 1),
                Err(CompressionError::TooLarge)
            );
        }
        assert_eq!(
            decode_limited(&HeaderMap::new(), b"1234", 4).unwrap(),
            b"1234"
        );
        assert_eq!(
            decode_limited(&HeaderMap::new(), b"12345", 4),
            Err(CompressionError::TooLarge)
        );
        assert_eq!(
            decode(&HeaderMap::new(), &vec![0; MAX_BODY + 1]),
            Err(CompressionError::TooLarge)
        );
    }
    #[test]
    fn systemone_compression_does_not_recursively_decode() {
        let inner = zstd(b"{}");
        assert_eq!(decode(&headers("gzip"), &gzip(&inner)).unwrap(), inner);
    }
    #[test]
    fn systemone_compression_zstd_window_is_enforced_before_expansion() {
        let mut encoder = zstd::stream::write::Encoder::new(Vec::new(), 1).unwrap();
        encoder.window_log(24).unwrap();
        encoder.write_all(b"{}").unwrap();
        // Emit a streaming frame header with an unknown final content size.
        encoder.flush().unwrap();
        let encoded = encoder.finish().unwrap();
        assert_eq!(zstd::stream::decode_all(encoded.as_slice()).unwrap(), b"{}");
        assert_eq!(
            decode(&headers("zstd"), &encoded),
            Err(CompressionError::Malformed)
        );
    }
    #[test]
    fn systemone_compression_error_status_contract() {
        assert_eq!(CompressionError::Malformed.status(), 400);
        assert_eq!(CompressionError::TooLarge.status(), 413);
        assert_eq!(CompressionError::UnsupportedEncoding.status(), 415);
        assert_eq!(CompressionError::TooLarge.code(), "body_too_large");
    }
}

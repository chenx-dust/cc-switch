//! HTTP content-encoding helpers.

use axum::http::HeaderMap;
use std::io::Read;

/// Decompress body bytes according to a single content-encoding value.
pub(crate) fn decompress_body(
    content_encoding: &str,
    body: &[u8],
) -> Result<Vec<u8>, std::io::Error> {
    match content_encoding {
        "gzip" | "x-gzip" => {
            let mut decoder = flate2::read::GzDecoder::new(body);
            let mut decompressed = Vec::new();
            decoder.read_to_end(&mut decompressed)?;
            Ok(decompressed)
        }
        "deflate" => {
            // RFC 9110 里的 deflate 是 zlib 包裹格式；部分上游会发 raw deflate。
            let mut decompressed = Vec::new();
            let mut zlib = flate2::read::ZlibDecoder::new(body);
            match zlib.read_to_end(&mut decompressed) {
                Ok(_) => Ok(decompressed),
                Err(zlib_err) => {
                    log::debug!("deflate 按 zlib 解压失败（{zlib_err}），回退 raw deflate");
                    let mut decompressed = Vec::new();
                    let mut raw = flate2::read::DeflateDecoder::new(body);
                    raw.read_to_end(&mut decompressed)?;
                    Ok(decompressed)
                }
            }
        }
        "br" => {
            let mut decompressed = Vec::new();
            brotli::BrotliDecompress(&mut std::io::Cursor::new(body), &mut decompressed)?;
            Ok(decompressed)
        }
        "zstd" | "zst" => zstd::stream::decode_all(std::io::Cursor::new(body)),
        _ => {
            log::warn!("未知的 content-encoding: {content_encoding}，跳过解压");
            Ok(body.to_vec())
        }
    }
}

pub(crate) fn is_supported_content_encoding(content_encoding: &str) -> bool {
    matches!(
        content_encoding,
        "gzip" | "x-gzip" | "deflate" | "br" | "zstd" | "zst"
    )
}

/// Extract content-encoding, ignoring identity and empty values.
pub(crate) fn get_content_encoding(headers: &HeaderMap) -> Option<String> {
    headers
        .get("content-encoding")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.trim().to_lowercase())
        .filter(|s| !s.is_empty() && s != "identity")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decompress_body_deflate_handles_zlib_wrapped_per_rfc9110() {
        let payload = br#"{"ok":true}"#;
        let mut encoder =
            flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
        std::io::Write::write_all(&mut encoder, payload).unwrap();
        let compressed = encoder.finish().unwrap();

        let decompressed = decompress_body("deflate", &compressed).unwrap();
        assert_eq!(decompressed, payload);
    }

    #[test]
    fn decompress_body_deflate_falls_back_to_raw_stream() {
        let payload = br#"{"ok":true}"#;
        let mut encoder =
            flate2::write::DeflateEncoder::new(Vec::new(), flate2::Compression::default());
        std::io::Write::write_all(&mut encoder, payload).unwrap();
        let compressed = encoder.finish().unwrap();

        let decompressed = decompress_body("deflate", &compressed).unwrap();
        assert_eq!(decompressed, payload);
    }

    #[test]
    fn decompress_body_supports_zstd() {
        let payload = br#"{"ok":true}"#;
        let compressed = zstd::stream::encode_all(std::io::Cursor::new(payload), 0).unwrap();

        let decompressed = decompress_body("zstd", &compressed).unwrap();
        assert_eq!(decompressed, payload);
    }

    #[test]
    fn unsupported_encoding_is_not_marked_supported() {
        assert!(!is_supported_content_encoding("unknown-encoding"));
    }
}

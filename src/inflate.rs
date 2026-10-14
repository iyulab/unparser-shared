//! Reading compressed data from an untrusted file without letting it expand without bound.
//!
//! A few kilobytes of DEFLATE can stand for gigabytes of zeros — a "zip bomb". Read to the end,
//! one entry of it allocates until the process is killed: not an error a caller can handle,
//! an abort. [`InflateGuard`] wraps the decompressing reader of one entry or stream and fails it
//! with an [`io::Error`] once the data expands past what real documents do, following Apache
//! POI's `ZipSecureFile`, the reference reader for the same formats:
//!
//! - past [`GRACE_SIZE`] of output, the compressed size must stay at least [`MIN_INFLATE_RATIO`]
//!   of the output so far (POI: 1 % after 100 KiB) — text and markup compress 3–20 times,
//!   pictures barely, and nothing a document holds compresses a hundredfold;
//! - no entry expands past [`MAX_ENTRY_SIZE`] (POI: 4 GiB, the 32-bit ZIP limit).
//!
//! POI measures the compressed bytes consumed so far; this measures the entry's whole
//! compressed size, which a reader knows from the ZIP directory or the stream's length. The
//! ratio is therefore never smaller here than POI's, so nothing POI accepts is refused.
//!
//! [`is_inflation_limit`] tells the guard's error apart from an I/O error, so a caller can
//! classify it (an archive error, not a failing disk).
//!
//! ```
//! use std::io::Read;
//! use unparser_shared::inflate::{is_inflation_limit, InflateGuard};
//!
//! // 1 MiB of output claimed by a 1 KiB entry: past 100 KiB it is a bomb.
//! let bomb = std::io::repeat(0).take(1 << 20);
//! let err = InflateGuard::new(bomb, 1024).read_to_end(&mut Vec::new()).unwrap_err();
//! assert!(is_inflation_limit(&err));
//!
//! // The same output from 64 KiB of input is an ordinary 16-fold compression.
//! let text = std::io::repeat(b'a').take(1 << 20);
//! assert_eq!(InflateGuard::new(text, 64 * 1024).read_to_end(&mut Vec::new()).unwrap(), 1 << 20);
//! ```

use std::fmt;
use std::io::{self, Read};

/// The least the compressed size may be, as a share of the output, once the output passes
/// [`GRACE_SIZE`] — POI's `MIN_INFLATE_RATIO` default.
pub const MIN_INFLATE_RATIO: f64 = 0.01;

/// The output below which the ratio is not checked: a small entry cannot exhaust memory however
/// well it compresses (a part of zeros, a blank picture). POI's `GRACE_ENTRY_SIZE` default.
pub const GRACE_SIZE: u64 = 100 * 1024;

/// The most one entry or stream may expand to — POI's `MAX_ENTRY_SIZE` default.
pub const MAX_ENTRY_SIZE: u64 = 0xFFFF_FFFF;

/// A reader of decompressed data that fails once the data expands past the limits above.
#[derive(Debug)]
pub struct InflateGuard<R> {
    inner: R,
    compressed: u64,
    inflated: u64,
}

impl<R> InflateGuard<R> {
    /// Guard `inner`, the decompressing reader of data `compressed_size` bytes long as stored.
    pub fn new(inner: R, compressed_size: u64) -> Self {
        Self {
            inner,
            compressed: compressed_size,
            inflated: 0,
        }
    }
}

impl<R: Read> Read for InflateGuard<R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let n = self.inner.read(buf)?;
        self.inflated += n as u64;
        let ratio_broken = self.inflated > GRACE_SIZE
            && (self.compressed as f64) < self.inflated as f64 * MIN_INFLATE_RATIO;
        if ratio_broken || self.inflated > MAX_ENTRY_SIZE {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                InflationLimit {
                    compressed: self.compressed,
                    inflated: self.inflated,
                },
            ));
        }
        Ok(n)
    }
}

/// The error [`InflateGuard`] fails with.
#[derive(Debug)]
pub struct InflationLimit {
    compressed: u64,
    inflated: u64,
}

impl fmt::Display for InflationLimit {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{} compressed bytes expanded past {} bytes: more than a document's data expands \
             (a decompression bomb)",
            self.compressed, self.inflated
        )
    }
}

impl std::error::Error for InflationLimit {}

/// Whether `error` is [`InflateGuard`]'s — the data expanded past the limits — rather than a
/// failure of the reader underneath.
pub fn is_inflation_limit(error: &io::Error) -> bool {
    error
        .get_ref()
        .is_some_and(|inner| inner.is::<InflationLimit>())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn read_all(data: impl Read, compressed: u64) -> io::Result<usize> {
        InflateGuard::new(data, compressed).read_to_end(&mut Vec::new())
    }

    #[test]
    fn small_output_is_never_refused() {
        assert_eq!(
            read_all(io::repeat(0).take(GRACE_SIZE), 1).unwrap(),
            GRACE_SIZE as usize
        );
    }

    #[test]
    fn output_past_a_hundredfold_is_a_bomb() {
        let err = read_all(io::repeat(0).take(10 << 20), 10 * 1024).unwrap_err();
        assert!(is_inflation_limit(&err), "{err}");
        assert!(err.to_string().contains("decompression bomb"), "{err}");
    }

    #[test]
    fn ordinary_compression_passes() {
        // 20-fold: markup compresses about this well.
        assert_eq!(
            read_all(io::repeat(b'x').take(4 << 20), (4 << 20) / 20).unwrap(),
            4 << 20
        );
    }

    #[test]
    fn an_io_error_underneath_is_not_the_limit() {
        struct Failing;
        impl Read for Failing {
            fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
                Err(io::Error::other("disk"))
            }
        }
        let err = read_all(Failing, 10).unwrap_err();
        assert!(!is_inflation_limit(&err));
    }
}

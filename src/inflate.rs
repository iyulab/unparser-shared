//! Reading compressed data from an untrusted file without letting it expand without bound.
//!
//! A few kilobytes of DEFLATE can stand for gigabytes of zeros — a "zip bomb". Read to the end,
//! one entry of it allocates until the process is killed: not an error a caller can handle,
//! an abort. [`InflateGuard`] wraps the decompressing reader of one entry or stream and fails it
//! with an [`io::Error`] once the data expands past [`InflateLimits`]:
//!
//! - past a grace size of output, the output may be at most a fixed multiple of the compressed
//!   size — what real documents compress to, with room to spare;
//! - nothing expands past a maximum size.
//!
//! The limits differ by format, so each comes from that format's reference reader.
//! [`InflateLimits::POI`] — Apache POI's `ZipSecureFile`, the reference reader for OOXML — is the
//! default: past 100 KiB, at most 100 times the compressed size, and at most 4 GiB. Text and
//! markup compress 3–20 times, pictures barely, and nothing a document's parts hold compresses a
//! hundredfold. PDF streams do (a blank 1-bit image mask), and a PDF reader gives them its own
//! limits.
//!
//! POI measures the compressed bytes consumed so far; this measures the entry's whole
//! compressed size, which a reader knows from the ZIP directory or the stream's length. The
//! ratio is therefore never smaller here than POI's, so nothing POI accepts is refused.
//!
//! A decoder that is not a [`Read`] (one that builds its output in a loop) applies the same rule
//! with [`InflateLimits::check`]. [`is_inflation_limit`] tells the error apart from an I/O
//! error, so a caller can classify it (an archive error, not a failing disk).
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

/// How far compressed data may expand before it is taken for a decompression bomb.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InflateLimits {
    /// The output up to which the ratio is not checked: a small output cannot exhaust memory
    /// however well it compresses (a part of zeros, a blank picture).
    pub grace_size: u64,
    /// Past [`Self::grace_size`], the most the output may be as a multiple of the compressed
    /// size.
    pub max_expansion: u64,
    /// The most the output may be at all.
    pub max_size: u64,
}

impl InflateLimits {
    /// Apache POI's `ZipSecureFile` defaults: `GRACE_ENTRY_SIZE` 100 KiB, `MIN_INFLATE_RATIO`
    /// 0.01 (at most 100 times), `MAX_ENTRY_SIZE` 4 GiB (the 32-bit ZIP limit).
    pub const POI: Self = Self {
        grace_size: 100 * 1024,
        max_expansion: 100,
        max_size: 0xFFFF_FFFF,
    };

    /// Whether `inflated` bytes of output from `compressed` bytes stay within these limits.
    pub fn allows(&self, compressed: u64, inflated: u64) -> bool {
        inflated <= self.max_size
            && (inflated <= self.grace_size
                || inflated <= compressed.saturating_mul(self.max_expansion))
    }

    /// [`Self::allows`] as a result: the error [`InflateGuard`] fails with when it does not.
    pub fn check(&self, compressed: u64, inflated: u64) -> io::Result<()> {
        if self.allows(compressed, inflated) {
            Ok(())
        } else {
            Err(io::Error::new(
                io::ErrorKind::InvalidData,
                InflationLimit {
                    compressed,
                    inflated,
                },
            ))
        }
    }
}

impl Default for InflateLimits {
    fn default() -> Self {
        Self::POI
    }
}

/// A reader of decompressed data that fails once the data expands past its [`InflateLimits`].
#[derive(Debug)]
pub struct InflateGuard<R> {
    inner: R,
    compressed: u64,
    inflated: u64,
    limits: InflateLimits,
}

impl<R> InflateGuard<R> {
    /// Guard `inner`, the decompressing reader of data `compressed_size` bytes long as stored,
    /// with [`InflateLimits::POI`].
    pub fn new(inner: R, compressed_size: u64) -> Self {
        Self::with_limits(inner, compressed_size, InflateLimits::POI)
    }

    /// Guard `inner` with `limits`.
    pub fn with_limits(inner: R, compressed_size: u64, limits: InflateLimits) -> Self {
        Self {
            inner,
            compressed: compressed_size,
            inflated: 0,
            limits,
        }
    }
}

impl<R: Read> Read for InflateGuard<R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let n = self.inner.read(buf)?;
        self.inflated += n as u64;
        self.limits.check(self.compressed, self.inflated)?;
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
        let grace = InflateLimits::POI.grace_size;
        assert_eq!(
            read_all(io::repeat(0).take(grace), 1).unwrap(),
            grace as usize
        );
    }

    #[test]
    fn other_limits_move_both_bounds() {
        let limits = InflateLimits {
            grace_size: 1 << 20,
            max_expansion: 600,
            max_size: 8 << 20,
        };
        let read = |len: u64, compressed: u64| {
            InflateGuard::with_limits(io::repeat(0).take(len), compressed, limits)
                .read_to_end(&mut Vec::new())
        };
        // 500-fold past the grace: within 600.
        assert!(read(5 << 20, (5 << 20) / 500).is_ok());
        // 1000-fold: refused.
        assert!(is_inflation_limit(
            &read(5 << 20, (5 << 20) / 1000).unwrap_err()
        ));
        // Under the grace size any ratio passes; past the maximum nothing does.
        assert!(read(1 << 20, 1).is_ok());
        assert!(is_inflation_limit(&read(9 << 20, 9 << 20).unwrap_err()));
    }

    #[test]
    fn check_applies_the_rule_without_a_reader() {
        let poi = InflateLimits::default();
        assert!(poi.check(1, poi.grace_size).is_ok());
        let err = poi.check(1, poi.grace_size + 1).unwrap_err();
        assert!(is_inflation_limit(&err));
        assert!(poi.allows(u64::MAX, poi.max_size));
        assert!(!poi.allows(u64::MAX, poi.max_size + 1));
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

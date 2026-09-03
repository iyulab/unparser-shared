//! Error-kind numbering for the `un*` family: the values that are already shared, and
//! the bands that keep future ones from colliding.
//!
//! # What is here, and what deliberately is not
//!
//! Each library owns its own `ErrorKind` enum, because the reasons a PDF fails to parse
//! are not the reasons a spreadsheet does. What they share is the *numbering* those enums
//! live in, since the discriminants cross the C ABI as plain integers.
//!
//! This module therefore exports **only values that are already identical across the
//! family**. It does not offer a broad set of "common kinds", and that restraint is the
//! whole design: a library adopting a constant whose value differs from what it currently
//! ships would be renumbering its public ABI. Above [`UNKNOWN_FORMAT`] the family already
//! disagrees — the low band was assigned independently before anyone thought to align it —
//! so any value published here would be a renumbering request aimed at somebody.
//!
//! For everything else, this module offers **bands** rather than values: a range each
//! library allocates from, so that a kind added next year does not land on a number that
//! already means something elsewhere.
//!
//! # Bands
//!
//! | Range | Meaning |
//! |---|---|
//! | `0` | Success. Never a valid kind — see [`NONE`] |
//! | `1..=17` | Assigned before the convention existed. Frozen where shipped; see below |
//! | [`COMMON_BAND`] | New reasons that genuinely apply to more than one library |
//! | [`BOUNDARY_BAND`] | Failures of the ABI call itself, with no library-side counterpart |
//! | `200..` | One band per library, [`LIBRARY_BAND_SIZE`] wide. See [`library_band`] |
//!
//! The `1..=17` range is history, not design. Numbers in it are frozen for any library
//! that has shipped them, and a library that has *not* shipped a given number should not
//! assume it is free to mean something new — a consumer reading two of these libraries
//! sees one integer space. Add new reasons from [`COMMON_BAND`] or a library band.
//!
//! # Where a failure is attributed
//!
//! Two libraries classified the same failure differently — serialising a rendered result
//! to JSON — because nothing said where it belonged. A shared numbering is only useful if
//! the same failure lands on the same number, so the rule is stated here rather than
//! rediscovered:
//!
//! **Failing to serialise a rendered result is a rendering failure.** Producing output is
//! rendering, and it stays rendering when the last step of producing it is serialisation.
//! It is not a generic failure, and it is not a boundary failure — nothing is wrong with
//! how the call was made.
//!
//! A library with no rendering reason of its own takes one from its own band rather than
//! reaching for [`OTHER`], which means "this failure carries no classification" and is
//! worth keeping true.
//!
//! # The contract consumers are promised
//!
//! Every library in the family documents the same four rules. They are restated here
//! because a shared numbering is only worth having if the rules are shared too:
//!
//! 1. A new reason takes a new number. Existing numbers are never reused or renumbered.
//! 2. An unrecognised value is not an error — treat it as a generic failure and keep the
//!    number. A newer library stays usable by an older caller precisely because of this.
//! 3. `0` means success, never a reason.
//! 4. Reasons are `#[non_exhaustive]`: match with a `_ =>` arm.

use std::ffi::c_int;
use std::ops::RangeInclusive;

/// No error is recorded: the last call on this thread succeeded.
///
/// Not a valid kind. Zero is reserved for this so that a zeroed struct or an
/// uninitialised `int` cannot be mistaken for a classified failure.
pub const NONE: c_int = 0;

/// A failure with no more specific classification.
///
/// Exists so that bindings have a value for a failure that carries no classification —
/// one raised by the binding itself, say. It must never be confused with success.
///
/// Shared across the family; adopting it is a no-op for libraries that already use `1`.
pub const OTHER: c_int = 1;

/// An I/O failure: a missing file, an unreadable one, a short read.
///
/// Shared across the family.
pub const IO: c_int = 2;

/// The input is not a document this library recognises at all.
///
/// Distinct from "recognised but unsupported", which is a library-specific reason.
///
/// Shared across the family.
pub const UNKNOWN_FORMAT: c_int = 3;

/// Reasons that apply to more than one library and were assigned after the convention.
///
/// A reason belongs here only once **two or more** libraries expose it. One library's
/// need is served by its own band — a shared numbering earns its keep by describing what
/// is actually shared, not by anticipating.
pub const COMMON_BAND: RangeInclusive<c_int> = 18..=99;

/// Reasons that describe a failure of the ABI call itself.
///
/// These have no library-side counterpart: nothing went wrong with the document, the
/// call was made wrongly or could not deliver its result. The three values in this band
/// are the ones the family already agrees on.
pub const BOUNDARY_BAND: RangeInclusive<c_int> = 100..=199;

/// An argument was null, or a string argument was not valid UTF-8.
pub const INVALID_ARGUMENT: c_int = 100;

/// A panic was caught at the ABI boundary.
///
/// Unwinding across `extern "C"` is undefined behaviour, so every entry point catches.
/// Reaching this value means a bug in the library, not in the caller's input.
pub const PANIC: c_int = 101;

/// The result could not cross the ABI: a string holding an interior NUL byte.
///
/// The output exists and is correct as far as the library is concerned; it simply cannot
/// be represented as a C string. Reported rather than truncated, because silently
/// returning the text up to the NUL loses the rest without saying so.
pub const INVALID_OUTPUT: c_int = 102;

/// Width of each library's private band.
pub const LIBRARY_BAND_SIZE: c_int = 100;

/// First number available to library bands.
pub const FIRST_LIBRARY_BAND: c_int = 200;

/// The band belonging to library `index`, counting from zero.
///
/// Bands are handed out in a fixed order so that a number identifies which library
/// produced it. The current allocation:
///
/// | Index | Library | Band |
/// |---|---|---|
/// | 0 | `unpdf` | `200..=299` |
/// | 1 | `undoc` | `300..=399` |
/// | 2 | `unhwp` | `400..=499` |
///
/// ```
/// # use unparser_shared::kind;
/// assert_eq!(kind::library_band(2), 400..=499);
/// ```
pub const fn library_band(index: c_int) -> RangeInclusive<c_int> {
    let start = FIRST_LIBRARY_BAND + index * LIBRARY_BAND_SIZE;
    start..=(start + LIBRARY_BAND_SIZE - 1)
}

/// Whether `kind` names a failure at the ABI boundary rather than in the document.
///
/// Useful to a binding that wants to report "you called this wrongly" differently from
/// "your document is broken".
///
/// ```
/// # use unparser_shared::kind;
/// assert!(kind::is_boundary(kind::PANIC));
/// assert!(!kind::is_boundary(kind::IO));
/// assert!(!kind::is_boundary(kind::NONE));
/// ```
pub fn is_boundary(kind: c_int) -> bool {
    BOUNDARY_BAND.contains(&kind)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// These four values are the family contract. A change here is a breaking change for
    /// three published ABIs at once, so it is asserted rather than merely documented.
    #[test]
    fn the_shared_values_are_what_the_family_already_ships() {
        assert_eq!(NONE, 0);
        assert_eq!(OTHER, 1);
        assert_eq!(IO, 2);
        assert_eq!(UNKNOWN_FORMAT, 3);
        assert_eq!(INVALID_ARGUMENT, 100);
        assert_eq!(PANIC, 101);
        assert_eq!(INVALID_OUTPUT, 102);
    }

    #[test]
    fn bands_do_not_overlap_and_leave_the_historical_range_alone() {
        assert!(*COMMON_BAND.start() > UNKNOWN_FORMAT);
        assert_eq!(
            *COMMON_BAND.start(),
            18,
            "1..=17 is history, not ours to hand out"
        );
        assert!(COMMON_BAND.end() < BOUNDARY_BAND.start());
        assert!(*BOUNDARY_BAND.end() < FIRST_LIBRARY_BAND);
    }

    #[test]
    fn boundary_values_live_in_the_boundary_band() {
        for kind in [INVALID_ARGUMENT, PANIC, INVALID_OUTPUT] {
            assert!(is_boundary(kind), "{kind} should be a boundary reason");
        }
        for kind in [NONE, OTHER, IO, UNKNOWN_FORMAT] {
            assert!(!is_boundary(kind), "{kind} should not be a boundary reason");
        }
    }

    #[test]
    fn library_bands_are_adjacent_and_disjoint() {
        let first = library_band(0);
        let second = library_band(1);
        let third = library_band(2);

        assert_eq!(first, 200..=299);
        assert_eq!(second, 300..=399);
        assert_eq!(third, 400..=499);
        assert_eq!(*second.start(), *first.end() + 1);
        assert_eq!(*third.start(), *second.end() + 1);
        assert!(!first.contains(second.start()));
    }
}

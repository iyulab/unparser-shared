//! Thread-local last-error storage and the panic guard, for a C ABI that returns
//! sentinels rather than error values.
//!
//! # The shape this supports
//!
//! A C entry point cannot return a Rust `Result`, so the family's libraries return null
//! or a sentinel and record *why* separately, to be read by `<lib>_last_error()` and
//! `<lib>_last_error_kind()`. Three things have to hold for that to be trustworthy:
//!
//! - **Message and kind move together.** A message paired with a stale kind is worse than
//!   no kind at all, because it looks authoritative. [`LastErrorSlot::set`] writes both.
//! - **Nothing unwinds across the boundary.** Unwinding out of `extern "C"` is undefined
//!   behaviour, so every entry point wraps its body — see [`catch`].
//! - **Classification survives to the boundary.** Rendering an error to a string early
//!   discards the reason. Closures therefore carry [`FfiError`], not `String`.
//!
//! # Why the slot is yours, not ours
//!
//! [`LastErrorSlot`] is a type you place in your own `thread_local!`, rather than storage
//! this crate owns. That is deliberate and load-bearing.
//!
//! Each library ships as its own cdylib, so today each has its own statics. But this crate
//! is linked into all of them, and a Rust binary depending on two of them would share one
//! copy — at which point a single shared static would make `unpdf_last_error()` return
//! whatever the preceding `undoc` call recorded. Isolation cannot rest on how consumers
//! happen to link; declaring the slot in the consuming crate makes it structural.
//!
//! ```
//! use unparser_shared::ffi::LastErrorSlot;
//!
//! thread_local! {
//!     static LAST_ERROR: LastErrorSlot = const { LastErrorSlot::new() };
//! }
//!
//! unparser_shared::export_last_error_abi!(LAST_ERROR, demo_last_error, demo_last_error_kind);
//! ```

use std::cell::{Cell, RefCell};
use std::ffi::{c_char, c_int, CStr, CString};
use std::panic::{catch_unwind, UnwindSafe};
use std::ptr;

use crate::kind;

/// A failure on its way out of an entry point: why it failed, and what to say about it.
///
/// Carried instead of a rendered string so the classification reaches the boundary. The
/// message is a `String` because it is about to be copied into a `CString` anyway.
pub type FfiError = (c_int, String);

/// The last failure recorded on this thread, and its classification.
///
/// Place one in a `thread_local!` in your own crate — see the [module docs](self) for why
/// this crate does not own it.
#[derive(Debug, Default)]
pub struct LastErrorSlot {
    message: RefCell<Option<CString>>,
    kind: Cell<c_int>,
}

impl LastErrorSlot {
    /// An empty slot: no message, kind [`kind::NONE`].
    ///
    /// `const` so it can be used in `thread_local! { static X: _ = const { .. } }`, which
    /// avoids the lazy-initialisation check on every access.
    pub const fn new() -> Self {
        Self {
            message: RefCell::new(None),
            kind: Cell::new(kind::NONE),
        }
    }

    /// Record a failure, replacing whatever was there.
    ///
    /// A message holding an interior NUL cannot become a C string; the kind is still
    /// recorded and the message is dropped, so the caller gets a classification and a null
    /// message rather than a classification paired with the *previous* message.
    pub fn set(&self, kind: c_int, message: &str) {
        // Written first: if the conversion fails, the stale message must already be gone.
        *self.message.borrow_mut() = CString::new(message).ok();
        self.kind.set(kind);
    }

    /// Record a failure from an [`FfiError`].
    pub fn set_error(&self, error: &FfiError) {
        self.set(error.0, &error.1);
    }

    /// Forget any recorded failure. Call on entry to an operation that succeeded, so a
    /// caller polling the slot does not see an old failure attributed to a new call.
    pub fn clear(&self) {
        *self.message.borrow_mut() = None;
        self.kind.set(kind::NONE);
    }

    /// The recorded message as a C string, or null when none is recorded.
    ///
    /// # Safety of the returned pointer
    ///
    /// Borrowed from thread-local storage: valid until the next call that writes this slot
    /// on the same thread. Callers across the ABI must copy it, not retain it. The
    /// pointer is produced inside the borrow and no reference escapes, so the value cannot
    /// be freed while a caller is still inside the same call.
    pub fn message_ptr(&self) -> *const c_char {
        self.message
            .borrow()
            .as_ref()
            .map(|message| message.as_ptr())
            .unwrap_or(ptr::null())
    }

    /// The recorded classification, or [`kind::NONE`] when the last call succeeded.
    pub fn kind(&self) -> c_int {
        self.kind.get()
    }
}

/// Run `body`, turning a panic into [`kind::PANIC`] instead of unwinding.
///
/// Use at every `extern "C"` entry point. A panic is a bug in the library rather than a
/// classified failure of the caller's input, so it is reported as such rather than being
/// dressed up as a document problem.
///
/// The panic payload is not forwarded: a panic message is an implementation detail and
/// often not `'static`. What the caller needs is that it happened.
///
/// ```
/// # use unparser_shared::{ffi, kind};
/// let ok: Result<i32, ffi::FfiError> = ffi::catch(|| Ok(7));
/// assert_eq!(ok.unwrap(), 7);
///
/// let boom: Result<i32, ffi::FfiError> = ffi::catch(|| panic!("bug"));
/// assert_eq!(boom.unwrap_err().0, kind::PANIC);
/// ```
pub fn catch<T, F>(body: F) -> Result<T, FfiError>
where
    F: FnOnce() -> Result<T, FfiError> + UnwindSafe,
{
    match catch_unwind(body) {
        Ok(result) => result,
        Err(_) => Err((
            kind::PANIC,
            "a panic was caught at the ABI boundary".to_string(),
        )),
    }
}

/// A null or non-UTF-8 argument.
///
/// ```
/// # use unparser_shared::{ffi, kind};
/// assert_eq!(ffi::invalid_argument("path was null").0, kind::INVALID_ARGUMENT);
/// ```
pub fn invalid_argument(message: impl Into<String>) -> FfiError {
    (kind::INVALID_ARGUMENT, message.into())
}

/// A result that cannot cross the ABI because it holds an interior NUL byte.
///
/// The wording is fixed so the family reports this identically; consumers have been seen
/// matching on it before `*_last_error_kind` existed.
pub fn invalid_output() -> FfiError {
    (
        kind::INVALID_OUTPUT,
        "output contains null byte".to_string(),
    )
}

/// Read a non-null C string argument as `&str`, classifying a non-UTF-8 one.
///
/// Separate from [`with_c_str!`](crate::with_c_str) because an entry point with an
/// out-parameter has to reject a null pointer *before* it starts producing a result — its
/// failure path writes through that pointer — so it does its own null check and needs only
/// the conversion. Everywhere else, [`with_c_str!`](crate::with_c_str) does both.
///
/// # Safety
///
/// `ptr` must be non-null and point to a NUL-terminated string that stays valid for `'a`.
/// The lifetime is unbounded, as with [`std::ffi::CStr::from_ptr`]: nothing here can check
/// how long the caller's buffer lives.
///
/// ```
/// # use std::ffi::CString;
/// let path = CString::new("document.pdf").unwrap();
/// let read = unsafe { unparser_shared::ffi::c_str_utf8(path.as_ptr()) };
/// assert_eq!(read.unwrap(), "document.pdf");
/// ```
pub unsafe fn c_str_utf8<'a>(ptr: *const c_char) -> Result<&'a str, FfiError> {
    unsafe { CStr::from_ptr(ptr) }
        .to_str()
        .map_err(|error| invalid_argument(error.to_string()))
}

/// Declare `<name>_last_error` and `<name>_last_error_kind` over a [`LastErrorSlot`].
///
/// The function names are passed in full rather than derived from a prefix, because
/// `macro_rules!` cannot concatenate identifiers and an exported symbol is not worth a
/// proc-macro dependency. Spelling them out also means the symbol appears literally in
/// the source, which is what someone greps for.
///
/// ```
/// use unparser_shared::ffi::LastErrorSlot;
///
/// thread_local! {
///     static LAST_ERROR: LastErrorSlot = const { LastErrorSlot::new() };
/// }
///
/// unparser_shared::export_last_error_abi!(LAST_ERROR, mylib_last_error, mylib_last_error_kind);
/// ```
#[macro_export]
macro_rules! export_last_error_abi {
    ($slot:path, $message_fn:ident, $kind_fn:ident) => {
        /// The message for the last failure on this thread, or null if none.
        ///
        /// # Safety
        ///
        /// The pointer is borrowed from thread-local storage and is valid until the next
        /// call into this library on the same thread. Copy it; do not retain it.
        #[no_mangle]
        pub extern "C" fn $message_fn() -> *const ::std::ffi::c_char {
            $slot.with(|slot| $crate::ffi::LastErrorSlot::message_ptr(slot))
        }

        /// Why the last call on this thread failed, or `0` if it succeeded.
        ///
        /// A new reason takes a new number and existing numbers are never reused, so an
        /// unrecognised value should be treated as a generic failure rather than an error.
        #[no_mangle]
        pub extern "C" fn $kind_fn() -> ::std::ffi::c_int {
            $slot.with(|slot| $crate::ffi::LastErrorSlot::kind(slot))
        }
    };
}

/// Assert that an error-kind enum's discriminants are what they were.
///
/// The discriminants are a public ABI contract, so a reordering that a compiler is happy
/// with is a breaking change for every consumer. This turns that into a test failure.
///
/// ```
/// #[derive(Debug, Clone, Copy, PartialEq, Eq)]
/// #[repr(i32)]
/// pub enum ErrorKind {
///     Other = 1,
///     Io = 2,
/// }
///
/// unparser_shared::assert_stable_kinds! {
///     ErrorKind, discriminants_are_stable,
///     Other = 1,
///     Io = 2,
/// }
/// ```
#[macro_export]
macro_rules! assert_stable_kinds {
    ($enum:ty, $test_name:ident, $($variant:ident = $value:expr),+ $(,)?) => {
        #[test]
        fn $test_name() {
            $(
                assert_eq!(
                    <$enum>::$variant as ::std::ffi::c_int,
                    $value as ::std::ffi::c_int,
                    concat!(
                        stringify!($variant),
                        " changed value. Discriminants are a public ABI contract: a new \
                         reason takes a new number, existing ones are never renumbered."
                    )
                );
            )+
        }
    };
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::CStr;

    thread_local! {
        static LAST_ERROR: LastErrorSlot = const { LastErrorSlot::new() };
    }

    export_last_error_abi!(LAST_ERROR, test_last_error, test_last_error_kind);

    fn recorded_message() -> Option<String> {
        let ptr = test_last_error();
        if ptr.is_null() {
            None
        } else {
            // Safety: the pointer came from the slot and is not retained past this call.
            Some(
                unsafe { CStr::from_ptr(ptr) }
                    .to_string_lossy()
                    .into_owned(),
            )
        }
    }

    #[test]
    fn an_untouched_slot_reads_as_success() {
        LAST_ERROR.with(|slot| slot.clear());
        assert_eq!(test_last_error_kind(), kind::NONE);
        assert_eq!(recorded_message(), None);
    }

    #[test]
    fn setting_and_clearing_moves_message_and_kind_together() {
        LAST_ERROR.with(|slot| slot.set(kind::IO, "file not found"));
        assert_eq!(test_last_error_kind(), kind::IO);
        assert_eq!(recorded_message().as_deref(), Some("file not found"));

        LAST_ERROR.with(|slot| slot.clear());
        assert_eq!(test_last_error_kind(), kind::NONE);
        assert_eq!(recorded_message(), None);
    }

    /// The failure mode this ordering exists for: a message that cannot become a C string
    /// must not leave the *previous* message standing next to the new kind, which would
    /// read as an authoritative explanation of the wrong failure.
    #[test]
    fn a_message_holding_a_nul_does_not_leave_the_previous_one_behind() {
        LAST_ERROR.with(|slot| slot.set(kind::IO, "first failure"));
        LAST_ERROR.with(|slot| slot.set(kind::PANIC, "second\0failure"));

        assert_eq!(test_last_error_kind(), kind::PANIC);
        assert_eq!(recorded_message(), None, "the stale message must be gone");
    }

    #[test]
    fn set_error_records_both_halves() {
        LAST_ERROR.with(|slot| slot.set_error(&invalid_output()));
        assert_eq!(test_last_error_kind(), kind::INVALID_OUTPUT);
        assert_eq!(
            recorded_message().as_deref(),
            Some("output contains null byte")
        );
        LAST_ERROR.with(|slot| slot.clear());
    }

    /// Isolation is the reason the slot lives in the consumer. Asserted so that a future
    /// refactor moving storage into this crate fails here rather than in production.
    #[test]
    fn a_slot_is_per_thread() {
        LAST_ERROR.with(|slot| slot.set(kind::IO, "main thread"));

        std::thread::spawn(|| {
            assert_eq!(
                test_last_error_kind(),
                kind::NONE,
                "another thread must start clean"
            );
        })
        .join()
        .expect("thread should not panic");

        assert_eq!(
            test_last_error_kind(),
            kind::IO,
            "this thread keeps its own"
        );
        LAST_ERROR.with(|slot| slot.clear());
    }

    #[test]
    fn catch_passes_success_and_classified_failure_through() {
        assert_eq!(catch(|| Ok(41 + 1)).unwrap(), 42);

        let err = catch::<(), _>(|| Err(invalid_argument("path was null"))).unwrap_err();
        assert_eq!(err.0, kind::INVALID_ARGUMENT);
        assert_eq!(err.1, "path was null");
    }

    #[test]
    fn catch_turns_a_panic_into_a_boundary_reason() {
        let previous = std::panic::take_hook();
        std::panic::set_hook(Box::new(|_| {}));
        let err = catch::<(), _>(|| panic!("deliberate")).unwrap_err();
        std::panic::set_hook(previous);

        assert_eq!(err.0, kind::PANIC);
        assert!(kind::is_boundary(err.0));
        assert!(
            !err.1.contains("deliberate"),
            "the panic payload is an implementation detail: {}",
            err.1
        );
    }
}

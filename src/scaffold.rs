//! Macros that assemble a C entry point out of the primitives in [`crate::ffi`].
//!
//! # What is repeated, and why a macro
//!
//! [`crate::ffi`] hands out the *materials* — a last-error slot, a panic guard, the two
//! boundary failure reasons. Assembling them is five steps, and only one of them says
//! anything about the library:
//!
//! 1. clear the slot, so a caller polling it does not read a previous failure. The slot is
//!    cleared on entry, not on success: a body that itself calls another entry point on
//!    this thread leaves that call's failure visible
//! 2. reject a null handle with [`crate::ffi::invalid_argument`] and return the sentinel
//! 3. run the body inside [`crate::ffi::catch`] ← the only step with domain content
//! 4. move the result across the ABI, reporting [`crate::ffi::invalid_output`] if it cannot
//! 5. record a failure and return the sentinel
//!
//! Steps 1, 2, 4 and 5 are about eighteen lines with no library content in them, and the
//! sentinel in steps 2 and 5 differs by return type — null for a pointer, `-1` for a
//! count. That is why these are several macros rather than one: a single general macro
//! would have to be told the sentinel, and being told is how the wrong one gets used.
//!
//! # Names are spelled out, not built
//!
//! Every macro takes the exported function name in full, following
//! [`crate::export_last_error_abi!`]: `macro_rules!` cannot concatenate identifiers, and
//! an exported symbol appearing literally in the source is what someone greps for.
//! Doc comments are passed through from the call site, because what an entry point means
//! is not something this crate knows.

/// Read a nullable C string argument as `&str`, classifying both ways it can fail.
///
/// Expands to a `Result`, so the usual form is `with_c_str!(path)?` inside a
/// [`catch`](crate::ffi::catch) closure. The null message is built from the argument's own
/// name — `with_c_str!(resource_id)` reports `"resource_id is null"`, which is the wording
/// the family already uses.
///
/// Use this where the entry point has no out-parameter, so that classifying a null
/// argument inside the closure is indistinguishable from classifying it before. Where
/// there *is* an out-parameter, check for null first and use
/// [`ffi::c_str_utf8`](crate::ffi::c_str_utf8) here instead — see
/// [`export_bytes_getter!`](crate::export_bytes_getter).
///
/// # Safety
///
/// `$ptr` must be null or point to a NUL-terminated string. This expands to an unsafe
/// read, so it has to appear in an `unsafe` context.
///
/// ```
/// # use std::ffi::{c_char, CString};
/// # use unparser_shared::ffi::FfiError;
/// unsafe fn name_length(path: *const c_char) -> Result<usize, FfiError> {
///     let path = unsafe { unparser_shared::with_c_str!(path) }?;
///     Ok(path.len())
/// }
///
/// let path = CString::new("a.txt").unwrap();
/// assert_eq!(unsafe { name_length(path.as_ptr()) }.unwrap(), 5);
/// assert_eq!(
///     unsafe { name_length(std::ptr::null()) }.unwrap_err().1,
///     "path is null"
/// );
/// ```
#[macro_export]
macro_rules! with_c_str {
    ($ptr:ident) => {
        if $ptr.is_null() {
            ::std::result::Result::Err($crate::ffi::invalid_argument(concat!(
                stringify!($ptr),
                " is null"
            )))
        } else {
            $crate::ffi::c_str_utf8($ptr)
        }
    };
}

/// Declare an opaque document handle and the function that frees it.
///
/// The handle is what a C caller holds: a `#[repr(C)]` newtype around the library's own
/// document type with one private field. The field name is given rather than invented,
/// because every entry point projects through it (`(*doc).inner`) and that has to be
/// readable in the consuming source.
///
/// `#[repr(C)]` is emitted because the three shipped libraries declare it. It says
/// nothing useful about a Rust field, and it is kept only so that adopting this macro
/// leaves the declared type identical.
///
/// ```
/// # struct Document;
/// unparser_shared::export_handle! {
///     /// Opaque handle to a parsed document.
///     handle DemoDoc { inner: Document },
///
///     /// Free a document handle.
///     ///
///     /// # Safety
///     ///
///     /// - `doc` must be a pointer a parse function returned, or null.
///     /// - After this call the handle is invalid and must not be used.
///     free demo_release_document,
/// }
/// ```
#[macro_export]
macro_rules! export_handle {
    (
        $(#[$handle_meta:meta])*
        handle $handle:ident { $field:ident: $inner:ty },

        $(#[$free_meta:meta])*
        free $free_fn:ident $(,)?
    ) => {
        $(#[$handle_meta])*
        #[repr(C)]
        pub struct $handle {
            $field: $inner,
        }

        $(#[$free_meta])*
        #[no_mangle]
        pub unsafe extern "C" fn $free_fn(doc: *mut $handle) {
            if !doc.is_null() {
                // Wrapped explicitly rather than relying on the unsafe fn body: this
                // expands both in crates that deny `unsafe_op_in_unsafe_fn` and in crates
                // that do not, and a block around a genuinely unsafe operation is correct
                // under either.
                let _ = unsafe { ::std::boxed::Box::from_raw(doc) };
            }
        }
    };
}

/// Declare the release function for strings this library hands out.
///
/// Every entry point returning `*mut c_char` transfers ownership, so exactly one of these
/// belongs beside them. Reclaiming the allocation has to happen in the library that made
/// it, which is why it cannot be `free()` on the caller's side.
///
/// ```
/// unparser_shared::export_free_string!(
///     /// Free a string produced by this library.
///     ///
///     /// # Safety
///     ///
///     /// - `s` must be a pointer this library returned, or null.
///     demo_release_string
/// );
/// ```
#[macro_export]
macro_rules! export_free_string {
    ($(#[$meta:meta])* $free_fn:ident) => {
        $(#[$meta])*
        #[no_mangle]
        pub unsafe extern "C" fn $free_fn(s: *mut ::std::ffi::c_char) {
            if !s.is_null() {
                let _ = unsafe { ::std::ffi::CString::from_raw(s) };
            }
        }
    };
}

/// Declare the release function for byte buffers this library hands out.
///
/// The length is taken back because the buffer was a `Box<[u8]>`: reconstructing it needs
/// the same length the producing call reported. A zero length is ignored rather than
/// reclaimed — an empty boxed slice has no allocation to return.
///
/// ```
/// unparser_shared::export_free_bytes!(
///     /// Free a byte buffer produced by this library.
///     ///
///     /// # Safety
///     ///
///     /// - `data` must be a pointer this library returned, or null.
///     /// - `len` must be the length that call reported.
///     demo_release_bytes
/// );
/// ```
#[macro_export]
macro_rules! export_free_bytes {
    ($(#[$meta:meta])* $free_fn:ident) => {
        $(#[$meta])*
        #[no_mangle]
        pub unsafe extern "C" fn $free_fn(data: *mut u8, len: usize) {
            if !data.is_null() && len > 0 {
                let _ = unsafe {
                    ::std::boxed::Box::from_raw(::std::ptr::slice_from_raw_parts_mut(
                        data, len,
                    ))
                };
            }
        }
    };
}

/// Declare an entry point that hands out an owned string.
///
/// `$body` runs inside [`catch`](crate::ffi::catch) and evaluates to
/// `Result<String, FfiError>`. Everything around it is the same in every such entry
/// point: clear the slot, reject a null handle, move the string into a `CString`,
/// report [`invalid_output`](crate::ffi::invalid_output) if it holds an interior NUL,
/// record a failure and return null.
///
/// The handle parameter's name is taken from the call site so that `$body` can project
/// through it — `{ Ok((*doc).inner.title()) }`. The handle is `*const`, since producing a
/// string does not modify the document.
///
/// The null-handle message is `"document is null"`, fixed here so the family reports it
/// identically — the same reason [`invalid_output`](crate::ffi::invalid_output)'s wording
/// is fixed.
///
/// ```
/// # use std::ffi::{c_int, CStr};
/// # use unparser_shared::ffi::LastErrorSlot;
/// # struct Report { lines: Vec<String> }
/// # thread_local! { static LAST_ERROR: LastErrorSlot = const { LastErrorSlot::new() }; }
/// # unparser_shared::export_last_error_abi!(LAST_ERROR, demo2_last_error, demo2_last_error_kind);
/// # unparser_shared::export_handle! {
/// #     /// Handle.
/// #     handle Demo2Document { inner: Report },
/// #     /// Free.
/// #     ///
/// #     /// # Safety
/// #     /// `doc` must come from this library.
/// #     free demo2_free_document,
/// # }
/// # unparser_shared::export_free_string!(
/// #     /// Free.
/// #     ///
/// #     /// # Safety
/// #     /// `s` must come from this library.
/// #     demo2_free_string
/// # );
/// unparser_shared::export_string_getter!(
///     /// The report's line at `index`, or a failure when it is out of range.
///     ///
///     /// # Safety
///     ///
///     /// - `doc` must be a valid handle.
///     /// - The returned string must be freed with `demo2_free_string`.
///     LAST_ERROR,
///     demo2_line(doc: Demo2Document, index: c_int),
///     {
///         match unsafe { (&(*doc).inner.lines).get(index as usize) } {
///             Some(line) => Ok(line.clone()),
///             None => Err((unparser_shared::kind::OTHER, format!("no line {index}"))),
///         }
///     }
/// );
///
/// let doc = Box::into_raw(Box::new(Demo2Document {
///     inner: Report { lines: vec!["first".to_string()] },
/// }));
///
/// let line = unsafe { demo2_line(doc, 0) };
/// assert_eq!(unsafe { CStr::from_ptr(line) }.to_str().unwrap(), "first");
/// unsafe { demo2_free_string(line) };
///
/// assert!(unsafe { demo2_line(doc, 9) }.is_null());
/// assert_eq!(demo2_last_error_kind(), unparser_shared::kind::OTHER);
///
/// unsafe { demo2_free_document(doc) };
/// ```
#[macro_export]
macro_rules! export_string_getter {
    (
        $(#[$meta:meta])*
        $slot:path,
        $name:ident($handle:ident: $handle_ty:ty $(, $arg:ident: $arg_ty:ty)* $(,)?),
        $body:block
    ) => {
        $(#[$meta])*
        #[no_mangle]
        pub unsafe extern "C" fn $name(
            $handle: *const $handle_ty,
            $($arg: $arg_ty,)*
        ) -> *mut ::std::ffi::c_char {
            $slot.with(|slot| $crate::ffi::LastErrorSlot::clear(slot));

            if $handle.is_null() {
                $slot.with(|slot| {
                    $crate::ffi::LastErrorSlot::set_error(
                        slot,
                        &$crate::ffi::invalid_argument("document is null"),
                    )
                });
                return ::std::ptr::null_mut();
            }

            let produced: ::std::result::Result<::std::string::String, $crate::ffi::FfiError> =
                $crate::ffi::catch(|| $body);

            match produced {
                ::std::result::Result::Ok(text) => match ::std::ffi::CString::new(text) {
                    ::std::result::Result::Ok(owned) => owned.into_raw(),
                    ::std::result::Result::Err(_) => {
                        $slot.with(|slot| {
                            $crate::ffi::LastErrorSlot::set_error(
                                slot,
                                &$crate::ffi::invalid_output(),
                            )
                        });
                        ::std::ptr::null_mut()
                    }
                },
                ::std::result::Result::Err(error) => {
                    $slot.with(|slot| $crate::ffi::LastErrorSlot::set_error(slot, &error));
                    ::std::ptr::null_mut()
                }
            }
        }
    };
}

/// Declare an entry point that hands out a string which may legitimately be absent.
///
/// `$body` evaluates to `Result<Option<String>, FfiError>`. `Ok(None)` returns null with
/// the slot left at success, because absence is not a failure — document metadata that was
/// never set is the case this exists for.
///
/// That makes null ambiguous on its own, and deliberately so: the kind channel is what
/// separates the three outcomes a caller has to tell apart.
///
/// | `$body` | Returns | Kind |
/// |---|---|---|
/// | `Ok(Some(text))` | the string | success |
/// | `Ok(None)` | null | success — there is nothing to give |
/// | `Ok(Some(text))` holding an interior NUL | null | [`kind::INVALID_OUTPUT`](crate::kind::INVALID_OUTPUT) |
/// | `Err(..)` | null | whatever the body classified |
///
/// Use [`export_string_getter!`](crate::export_string_getter) where absence cannot happen
/// or is genuinely an empty string. Do not fold the two by defaulting absent to `""` —
/// that tells a caller the field is set and blank.
///
/// ```
/// # use unparser_shared::ffi::LastErrorSlot;
/// # struct Meta { author: Option<String> }
/// # thread_local! { static LAST_ERROR: LastErrorSlot = const { LastErrorSlot::new() }; }
/// # unparser_shared::export_last_error_abi!(LAST_ERROR, demo3_last_error, demo3_last_error_kind);
/// # unparser_shared::export_handle! {
/// #     /// Handle.
/// #     handle Demo3Document { inner: Meta },
/// #     /// Free.
/// #     ///
/// #     /// # Safety
/// #     /// `doc` must come from this library.
/// #     free demo3_free_document,
/// # }
/// unparser_shared::export_optional_string_getter!(
///     /// The document author, or null when none is recorded.
///     ///
///     /// # Safety
///     ///
///     /// - `doc` must be a valid handle.
///     /// - The returned string must be freed by this library.
///     LAST_ERROR,
///     demo3_author(doc: Demo3Document),
///     { Ok(unsafe { (*doc).inner.author.clone() }) }
/// );
///
/// let absent = Box::into_raw(Box::new(Demo3Document { inner: Meta { author: None } }));
/// assert!(unsafe { demo3_author(absent) }.is_null());
/// assert_eq!(demo3_last_error_kind(), unparser_shared::kind::NONE);
/// unsafe { demo3_free_document(absent) };
/// ```
#[macro_export]
macro_rules! export_optional_string_getter {
    (
        $(#[$meta:meta])*
        $slot:path,
        $name:ident($handle:ident: $handle_ty:ty $(, $arg:ident: $arg_ty:ty)* $(,)?),
        $body:block
    ) => {
        $(#[$meta])*
        #[no_mangle]
        pub unsafe extern "C" fn $name(
            $handle: *const $handle_ty,
            $($arg: $arg_ty,)*
        ) -> *mut ::std::ffi::c_char {
            $slot.with(|slot| $crate::ffi::LastErrorSlot::clear(slot));

            if $handle.is_null() {
                $slot.with(|slot| {
                    $crate::ffi::LastErrorSlot::set_error(
                        slot,
                        &$crate::ffi::invalid_argument("document is null"),
                    )
                });
                return ::std::ptr::null_mut();
            }

            let produced: ::std::result::Result<
                ::std::option::Option<::std::string::String>,
                $crate::ffi::FfiError,
            > = $crate::ffi::catch(|| $body);

            match produced {
                ::std::result::Result::Ok(::std::option::Option::Some(text)) => {
                    match ::std::ffi::CString::new(text) {
                        ::std::result::Result::Ok(owned) => owned.into_raw(),
                        ::std::result::Result::Err(_) => {
                            $slot.with(|slot| {
                                $crate::ffi::LastErrorSlot::set_error(
                                    slot,
                                    &$crate::ffi::invalid_output(),
                                )
                            });
                            ::std::ptr::null_mut()
                        }
                    }
                }
                ::std::result::Result::Ok(::std::option::Option::None) => {
                    ::std::ptr::null_mut()
                }
                ::std::result::Result::Err(error) => {
                    $slot.with(|slot| $crate::ffi::LastErrorSlot::set_error(slot, &error));
                    ::std::ptr::null_mut()
                }
            }
        }
    };
}

/// Declare an entry point that hands out a count.
///
/// `$body` evaluates to `Result<c_int, FfiError>`. Failure — including a null handle —
/// returns `-1`, which is why this is its own macro rather than a sentinel argument to a
/// general one: a sentinel that can be passed in is a sentinel that can be passed wrong.
///
/// The slot is cleared on entry like everywhere else, and here that is load-bearing rather
/// than tidy. A count getter returns a value instead of a pointer, so a caller polling
/// `<lib>_last_error_kind` after it has no other way to know the recorded failure is not
/// someone else's.
///
/// ```
/// # use unparser_shared::ffi::LastErrorSlot;
/// # struct Book { pages: Vec<u8> }
/// # thread_local! { static LAST_ERROR: LastErrorSlot = const { LastErrorSlot::new() }; }
/// # unparser_shared::export_last_error_abi!(LAST_ERROR, demo4_last_error, demo4_last_error_kind);
/// # unparser_shared::export_handle! {
/// #     /// Handle.
/// #     handle Demo4Document { inner: Book },
/// #     /// Free.
/// #     ///
/// #     /// # Safety
/// #     /// `doc` must come from this library.
/// #     free demo4_free_document,
/// # }
/// unparser_shared::export_count_getter!(
///     /// The number of pages, or -1 on failure.
///     ///
///     /// # Safety
///     ///
///     /// - `doc` must be a valid handle.
///     LAST_ERROR,
///     demo4_page_count(doc: Demo4Document),
///     { Ok(unsafe { (*doc).inner.pages.len() } as std::ffi::c_int) }
/// );
///
/// assert_eq!(unsafe { demo4_page_count(std::ptr::null()) }, -1);
/// assert_eq!(demo4_last_error_kind(), unparser_shared::kind::INVALID_ARGUMENT);
///
/// let doc = Box::into_raw(Box::new(Demo4Document { inner: Book { pages: vec![1, 2] } }));
/// assert_eq!(unsafe { demo4_page_count(doc) }, 2);
/// assert_eq!(demo4_last_error_kind(), unparser_shared::kind::NONE);
/// unsafe { demo4_free_document(doc) };
/// ```
#[macro_export]
macro_rules! export_count_getter {
    (
        $(#[$meta:meta])*
        $slot:path,
        $name:ident($handle:ident: $handle_ty:ty $(, $arg:ident: $arg_ty:ty)* $(,)?),
        $body:block
    ) => {
        $(#[$meta])*
        #[no_mangle]
        pub unsafe extern "C" fn $name(
            $handle: *const $handle_ty,
            $($arg: $arg_ty,)*
        ) -> ::std::ffi::c_int {
            $slot.with(|slot| $crate::ffi::LastErrorSlot::clear(slot));

            if $handle.is_null() {
                $slot.with(|slot| {
                    $crate::ffi::LastErrorSlot::set_error(
                        slot,
                        &$crate::ffi::invalid_argument("document is null"),
                    )
                });
                return -1;
            }

            let counted: ::std::result::Result<::std::ffi::c_int, $crate::ffi::FfiError> =
                $crate::ffi::catch(|| $body);

            match counted {
                ::std::result::Result::Ok(count) => count,
                ::std::result::Result::Err(error) => {
                    $slot.with(|slot| $crate::ffi::LastErrorSlot::set_error(slot, &error));
                    -1
                }
            }
        }
    };
}

/// Declare an entry point that hands out an owned byte buffer selected by a C-string key.
///
/// The signature is fixed — `(handle, key, out_len) -> *mut u8` — because that is the
/// shape every consumer of this ABI already exports, and only the names come from the call
/// site. An entry point needing a different shape is written by hand.
///
/// `$body` evaluates to `Result<Vec<u8>, FfiError>`. The macro moves that into a
/// `Box<[u8]>`, writes its length through `out_len`, and hands over the pointer; the
/// matching [`export_free_bytes!`](crate::export_free_bytes) reclaims it from the same two
/// values.
///
/// An empty `Vec` is a success like any other: it is handed over as a non-null pointer with
/// `out_len` set to `0`, distinct from the null this returns on failure. A caller that
/// treats a zero length as absence will misread an empty result as one that was never
/// produced.
///
/// # Why every argument is checked before the closure
///
/// The failure path writes `*out_len = 0`, so *when* an argument is rejected is
/// observable. All three null checks therefore happen before
/// [`catch`](crate::ffi::catch) runs, and a rejected argument leaves the caller's length
/// variable exactly as they left it — "not attempted" stays distinguishable from
/// "attempted and produced nothing".
///
/// That is also why `$body` reads the key with
/// [`ffi::c_str_utf8`](crate::ffi::c_str_utf8) rather than
/// [`with_c_str!`](crate::with_c_str): the null check is already done, and the UTF-8
/// conversion belongs inside the closure, where its failure does zero the length.
///
/// ```
/// # use std::collections::HashMap;
/// # use std::ffi::CString;
/// # use unparser_shared::ffi::LastErrorSlot;
/// # struct Archive { entries: HashMap<String, Vec<u8>> }
/// # thread_local! { static LAST_ERROR: LastErrorSlot = const { LastErrorSlot::new() }; }
/// # unparser_shared::export_last_error_abi!(LAST_ERROR, demo5_last_error, demo5_last_error_kind);
/// # unparser_shared::export_handle! {
/// #     /// Handle.
/// #     handle Demo5Document { inner: Archive },
/// #     /// Free.
/// #     ///
/// #     /// # Safety
/// #     /// `doc` must come from this library.
/// #     free demo5_free_document,
/// # }
/// # unparser_shared::export_free_bytes!(
/// #     /// Free.
/// #     ///
/// #     /// # Safety
/// #     /// `data` and `len` must come from this library.
/// #     demo5_free_bytes
/// # );
/// unparser_shared::export_bytes_getter!(
///     /// The bytes of the entry named `name`.
///     ///
///     /// # Safety
///     ///
///     /// - `doc` must be a valid handle.
///     /// - `name` must be a valid null-terminated UTF-8 string.
///     /// - `out_len` must point to storage for the length.
///     /// - The returned pointer must be freed with `demo5_free_bytes`.
///     LAST_ERROR,
///     demo5_entry(doc: Demo5Document, name, out out_len),
///     {
///         let name = unsafe { unparser_shared::ffi::c_str_utf8(name) }?;
///         match unsafe { (*doc).inner.entries.get(name) } {
///             Some(bytes) => Ok(bytes.clone()),
///             None => Err((unparser_shared::kind::OTHER, format!("no entry {name}"))),
///         }
///     }
/// );
///
/// let mut entries = HashMap::new();
/// entries.insert("a.bin".to_string(), vec![1u8, 2, 3]);
/// let doc = Box::into_raw(Box::new(Demo5Document { inner: Archive { entries } }));
/// let name = CString::new("a.bin").unwrap();
///
/// let mut out_len = 0usize;
/// let bytes = unsafe { demo5_entry(doc, name.as_ptr(), &mut out_len) };
/// assert_eq!(out_len, 3);
/// assert_eq!(unsafe { std::slice::from_raw_parts(bytes, out_len) }, [1, 2, 3]);
///
/// unsafe {
///     demo5_free_bytes(bytes, out_len);
///     demo5_free_document(doc);
/// }
/// ```
#[macro_export]
macro_rules! export_bytes_getter {
    (
        $(#[$meta:meta])*
        $slot:path,
        $name:ident($handle:ident: $handle_ty:ty, $key:ident, out $out_len:ident $(,)?),
        $body:block
    ) => {
        $(#[$meta])*
        #[no_mangle]
        pub unsafe extern "C" fn $name(
            $handle: *const $handle_ty,
            $key: *const ::std::ffi::c_char,
            $out_len: *mut usize,
        ) -> *mut u8 {
            $slot.with(|slot| $crate::ffi::LastErrorSlot::clear(slot));

            // Checked before the closure: the failure path below writes through
            // `$out_len`, so a rejected argument must leave it as the caller left it.
            if $handle.is_null() {
                $slot.with(|slot| {
                    $crate::ffi::LastErrorSlot::set_error(
                        slot,
                        &$crate::ffi::invalid_argument("document is null"),
                    )
                });
                return ::std::ptr::null_mut();
            }

            if $key.is_null() {
                $slot.with(|slot| {
                    $crate::ffi::LastErrorSlot::set_error(
                        slot,
                        &$crate::ffi::invalid_argument(concat!(
                            stringify!($key),
                            " is null"
                        )),
                    )
                });
                return ::std::ptr::null_mut();
            }

            if $out_len.is_null() {
                $slot.with(|slot| {
                    $crate::ffi::LastErrorSlot::set_error(
                        slot,
                        &$crate::ffi::invalid_argument(concat!(
                            stringify!($out_len),
                            " is null"
                        )),
                    )
                });
                return ::std::ptr::null_mut();
            }

            let produced: ::std::result::Result<
                ::std::vec::Vec<u8>,
                $crate::ffi::FfiError,
            > = $crate::ffi::catch(|| $body);

            match produced {
                ::std::result::Result::Ok(data) => {
                    let length = data.len();
                    let raw =
                        ::std::boxed::Box::into_raw(data.into_boxed_slice()) as *mut u8;
                    // The null check above is what makes this write sound; wrapped
                    // explicitly so the expansion compiles under
                    // `unsafe_op_in_unsafe_fn` too.
                    unsafe { *$out_len = length };
                    raw
                }
                ::std::result::Result::Err(error) => {
                    $slot.with(|slot| $crate::ffi::LastErrorSlot::set_error(slot, &error));
                    unsafe { *$out_len = 0 };
                    ::std::ptr::null_mut()
                }
            }
        }
    };
}

#[cfg(test)]
mod tests {
    use std::ffi::{c_char, CString};

    use crate::ffi::FfiError;

    unsafe fn read(ptr: *const c_char) -> Result<&'static str, FfiError> {
        unsafe { crate::with_c_str!(ptr) }
    }

    #[test]
    fn a_null_c_string_argument_is_named_in_its_own_message() {
        let failure = unsafe { read(std::ptr::null()) }.unwrap_err();
        assert_eq!(failure.0, crate::kind::INVALID_ARGUMENT);
        assert_eq!(
            failure.1, "ptr is null",
            "the message is built from the argument's own name"
        );
    }

    #[test]
    fn a_valid_c_string_argument_reads_through() {
        let text = CString::new("document.hwp").unwrap();
        assert_eq!(unsafe { read(text.as_ptr()) }.unwrap(), "document.hwp");
    }

    #[test]
    fn a_non_utf8_c_string_argument_is_an_invalid_argument() {
        // 0xFF is not a valid UTF-8 lead byte.
        let raw = [0xFFu8, 0x00];
        let failure = unsafe { read(raw.as_ptr() as *const c_char) }.unwrap_err();
        assert_eq!(failure.0, crate::kind::INVALID_ARGUMENT);
        assert!(
            !failure.1.is_empty(),
            "the conversion error must survive as a message"
        );
    }

    use std::collections::HashMap;

    use crate::ffi::LastErrorSlot;

    /// Stands in for a library's document. Deliberately not a document: the fields are
    /// only the *shapes* an entry point returns — a string, an absent string, a count,
    /// a byte buffer keyed by name.
    #[derive(Default)]
    struct Demo {
        body: String,
        title: Option<String>,
        blobs: HashMap<String, Vec<u8>>,
    }

    thread_local! {
        static DEMO_ERROR: LastErrorSlot = const { LastErrorSlot::new() };
    }

    crate::export_last_error_abi!(DEMO_ERROR, demo_last_error, demo_last_error_kind);

    crate::export_handle! {
        /// Opaque handle to a demo document.
        handle DemoDocument { inner: Demo },

        /// Free a demo handle.
        ///
        /// # Safety
        ///
        /// - `doc` must be a pointer this test module produced, or null.
        /// - After this call the handle is invalid and must not be used.
        free demo_free_document,
    }

    crate::export_free_string!(
        /// Free a string produced by a demo entry point.
        ///
        /// # Safety
        ///
        /// - `s` must be a pointer a demo entry point returned, or null.
        demo_free_string
    );

    crate::export_free_bytes!(
        /// Free a byte buffer produced by a demo entry point.
        ///
        /// # Safety
        ///
        /// - `data` must be a pointer a demo entry point returned, or null.
        /// - `len` must be the length that call reported.
        demo_free_bytes
    );

    fn demo(inner: Demo) -> *mut DemoDocument {
        Box::into_raw(Box::new(DemoDocument { inner }))
    }

    #[test]
    fn a_handle_round_trips_through_its_free_function() {
        let doc = demo(Demo::default());
        assert!(!doc.is_null());
        // A smoke test, deliberately: a stock `cargo test` run has no leak detector, so
        // this cannot show the allocation was returned. What it does show is that the
        // pair agrees on the pointer — a mismatched type or an extra indirection would
        // abort here.
        unsafe { demo_free_document(doc) };
    }

    #[test]
    fn every_free_function_accepts_null() {
        unsafe {
            demo_free_document(std::ptr::null_mut());
            demo_free_string(std::ptr::null_mut());
            demo_free_bytes(std::ptr::null_mut(), 0);
        }
    }

    /// A zero length must leave the caller's buffer alone.
    ///
    /// This pins the observable contract, not the guard: dropping a zero-length
    /// `Box<[u8]>` would not reach the allocator either, since `Global::deallocate`
    /// skips a zero-size layout. The `len > 0` guard earns its keep elsewhere — it means
    /// no `Box` is ever reconstructed from a pointer at all, which is what a
    /// provenance-checking run would object to.
    #[test]
    fn free_bytes_ignores_a_zero_length_buffer() {
        let mut byte = 7u8;
        unsafe { demo_free_bytes(&mut byte, 0) };
        assert_eq!(byte, 7);
    }

    crate::export_string_getter!(
        /// The document body.
        ///
        /// # Safety
        ///
        /// - `doc` must be a valid handle.
        /// - The returned string must be freed with `demo_free_string`.
        DEMO_ERROR,
        demo_body(doc: DemoDocument),
        { Ok(unsafe { (*doc).inner.body.clone() }) }
    );

    crate::export_string_getter!(
        /// The document body, repeated `times` times.
        ///
        /// # Safety
        ///
        /// - `doc` must be a valid handle.
        /// - The returned string must be freed with `demo_free_string`.
        DEMO_ERROR,
        demo_body_repeated(doc: DemoDocument, times: ::std::ffi::c_int),
        { Ok(unsafe { (*doc).inner.body.repeat(times.max(0) as usize) }) }
    );

    crate::export_string_getter!(
        /// The document body, or a classified failure when it is empty.
        ///
        /// # Safety
        ///
        /// - `doc` must be a valid handle.
        DEMO_ERROR,
        demo_body_or_fail(doc: DemoDocument),
        {
            let body = unsafe { &(*doc).inner.body };
            if body.is_empty() {
                return Err((crate::kind::IO, "no body".to_string()));
            }
            Ok(body.clone())
        }
    );

    crate::export_string_getter!(
        /// Always panics, to prove the guard is in place.
        ///
        /// # Safety
        ///
        /// - `doc` must be a valid handle.
        // Passed through to the generated item, which also proves the macro forwards
        // arbitrary attributes and not only doc comments.
        #[allow(unreachable_code)]
        DEMO_ERROR,
        demo_body_panics(doc: DemoDocument),
        { panic!("deliberate") }
    );

    fn owned(text: *mut c_char) -> String {
        assert!(!text.is_null());
        let owned = unsafe { std::ffi::CStr::from_ptr(text) }
            .to_string_lossy()
            .into_owned();
        unsafe { demo_free_string(text) };
        owned
    }

    fn demo_with_body(body: &str) -> *mut DemoDocument {
        demo(Demo {
            body: body.to_string(),
            ..Demo::default()
        })
    }

    #[test]
    fn a_string_getter_returns_its_value_and_reports_success() {
        let doc = demo_with_body("hello");
        assert_eq!(owned(unsafe { demo_body(doc) }), "hello");
        assert_eq!(demo_last_error_kind(), crate::kind::NONE);
        unsafe { demo_free_document(doc) };
    }

    #[test]
    fn a_string_getter_passes_its_extra_arguments_to_the_body() {
        let doc = demo_with_body("ab");
        assert_eq!(owned(unsafe { demo_body_repeated(doc, 3) }), "ababab");
        unsafe { demo_free_document(doc) };
    }

    #[test]
    fn a_null_handle_is_an_invalid_argument_named_document() {
        assert!(unsafe { demo_body(std::ptr::null()) }.is_null());
        assert_eq!(demo_last_error_kind(), crate::kind::INVALID_ARGUMENT);
        let message = unsafe { std::ffi::CStr::from_ptr(demo_last_error()) }
            .to_string_lossy()
            .into_owned();
        assert_eq!(
            message, "document is null",
            "the family reports this identically, so the wording is fixed here"
        );
    }

    /// The reason a caller can poll the slot after a *successful* call: it was cleared on
    /// entry, so a leftover failure cannot be attributed to this call.
    #[test]
    fn a_successful_call_clears_a_previous_failure() {
        assert!(unsafe { demo_body(std::ptr::null()) }.is_null());
        assert_ne!(demo_last_error_kind(), crate::kind::NONE);

        let doc = demo_with_body("fresh");
        assert_eq!(owned(unsafe { demo_body(doc) }), "fresh");
        assert_eq!(demo_last_error_kind(), crate::kind::NONE);
        unsafe { demo_free_document(doc) };
    }

    #[test]
    fn a_classified_body_failure_reaches_the_slot() {
        let doc = demo_with_body("");
        assert!(unsafe { demo_body_or_fail(doc) }.is_null());
        assert_eq!(demo_last_error_kind(), crate::kind::IO);
        unsafe { demo_free_document(doc) };
    }

    /// An interior NUL is the one failure the macro raises by itself: the value exists and
    /// is correct, it just cannot be a C string.
    #[test]
    fn a_value_holding_an_interior_nul_is_reported_not_truncated() {
        let doc = demo_with_body("before\0after");
        assert!(unsafe { demo_body(doc) }.is_null());
        assert_eq!(demo_last_error_kind(), crate::kind::INVALID_OUTPUT);
        unsafe { demo_free_document(doc) };
    }

    #[test]
    fn a_panic_in_the_body_does_not_cross_the_boundary() {
        let previous = std::panic::take_hook();
        std::panic::set_hook(Box::new(|_| {}));
        let doc = demo_with_body("x");
        let returned = unsafe { demo_body_panics(doc) };
        std::panic::set_hook(previous);

        assert!(returned.is_null());
        assert_eq!(demo_last_error_kind(), crate::kind::PANIC);
        unsafe { demo_free_document(doc) };
    }

    crate::export_optional_string_getter!(
        /// The document title.
        ///
        /// # Safety
        ///
        /// - `doc` must be a valid handle.
        /// - Returns null when no title is set, leaving the kind at success.
        /// - The returned string must be freed with `demo_free_string`.
        DEMO_ERROR,
        demo_title(doc: DemoDocument),
        { Ok(unsafe { (*doc).inner.title.clone() }) }
    );

    fn demo_with_title(title: Option<&str>) -> *mut DemoDocument {
        demo(Demo {
            title: title.map(str::to_string),
            ..Demo::default()
        })
    }

    #[test]
    fn a_present_optional_value_is_handed_out() {
        let doc = demo_with_title(Some("Quarterly Report"));
        assert_eq!(owned(unsafe { demo_title(doc) }), "Quarterly Report");
        assert_eq!(demo_last_error_kind(), crate::kind::NONE);
        unsafe { demo_free_document(doc) };
    }

    /// An absent value is not a failure. This is the whole reason this macro is separate
    /// from `export_string_getter!`: collapsing absent to an empty string would tell a
    /// caller there is a title and it is blank.
    #[test]
    fn an_absent_optional_value_is_null_and_still_success() {
        let doc = demo_with_title(None);
        assert!(unsafe { demo_title(doc) }.is_null());
        assert_eq!(demo_last_error_kind(), crate::kind::NONE);
        unsafe { demo_free_document(doc) };
    }

    /// The counterpart: a value that exists but cannot cross must not read as absent.
    /// Both return null, so the kind is the only thing that separates "there is nothing"
    /// from "we could not give it to you".
    #[test]
    fn an_unrepresentable_optional_value_is_not_reported_as_absent() {
        let doc = demo_with_title(Some("has\0interior nul"));
        assert!(unsafe { demo_title(doc) }.is_null());
        assert_eq!(demo_last_error_kind(), crate::kind::INVALID_OUTPUT);
        unsafe { demo_free_document(doc) };
    }

    #[test]
    fn an_optional_getter_rejects_a_null_handle() {
        assert!(unsafe { demo_title(std::ptr::null()) }.is_null());
        assert_eq!(demo_last_error_kind(), crate::kind::INVALID_ARGUMENT);
    }

    crate::export_count_getter!(
        /// The number of blobs in the document.
        ///
        /// # Safety
        ///
        /// - `doc` must be a valid handle.
        /// - Returns -1 on failure.
        DEMO_ERROR,
        demo_blob_count(doc: DemoDocument),
        { Ok(unsafe { (*doc).inner.blobs.len() } as ::std::ffi::c_int) }
    );

    fn demo_with_blob(id: &str, data: &[u8]) -> *mut DemoDocument {
        let mut blobs = HashMap::new();
        blobs.insert(id.to_string(), data.to_vec());
        demo(Demo {
            blobs,
            ..Demo::default()
        })
    }

    #[test]
    fn a_count_getter_returns_its_count() {
        let doc = demo_with_blob("logo.png", b"\x89PNG");
        assert_eq!(unsafe { demo_blob_count(doc) }, 1);
        assert_eq!(demo_last_error_kind(), crate::kind::NONE);
        unsafe { demo_free_document(doc) };
    }

    #[test]
    fn a_count_getter_returns_minus_one_for_a_null_handle() {
        assert_eq!(unsafe { demo_blob_count(std::ptr::null()) }, -1);
        assert_eq!(demo_last_error_kind(), crate::kind::INVALID_ARGUMENT);
    }

    /// A count getter returns a value, not a pointer, so a caller cannot tell from the
    /// return alone whether a recorded failure is theirs. Clearing on entry is what makes
    /// polling the slot after a count meaningful — the shipped ABI does this and it is
    /// easy to drop when the body is one line.
    #[test]
    fn a_count_getter_clears_a_previous_failure() {
        assert_eq!(unsafe { demo_blob_count(std::ptr::null()) }, -1);
        assert_ne!(demo_last_error_kind(), crate::kind::NONE);

        let doc = demo(Demo::default());
        assert_eq!(unsafe { demo_blob_count(doc) }, 0);
        assert_eq!(
            demo_last_error_kind(),
            crate::kind::NONE,
            "a count of zero is a success, not a leftover failure"
        );
        unsafe { demo_free_document(doc) };
    }

    crate::export_bytes_getter!(
        /// The bytes of the blob named `id`.
        ///
        /// # Safety
        ///
        /// - `doc` must be a valid handle.
        /// - `id` must be a valid null-terminated UTF-8 string.
        /// - `out_len` must point to storage for the length.
        /// - The returned pointer must be freed with `demo_free_bytes`.
        DEMO_ERROR,
        demo_blob(doc: DemoDocument, id, out out_len),
        {
            let id = unsafe { crate::ffi::c_str_utf8(id) }?;
            match unsafe { (*doc).inner.blobs.get(id) } {
                Some(data) => Ok(data.clone()),
                None => Err((crate::kind::OTHER, format!("no blob {id}"))),
            }
        }
    );

    #[test]
    fn a_bytes_getter_hands_over_the_buffer_and_its_length() {
        let doc = demo_with_blob("logo.png", b"\x89PNG\r\n");
        let id = CString::new("logo.png").unwrap();

        let mut out_len: usize = 0;
        let data = unsafe { demo_blob(doc, id.as_ptr(), &mut out_len) };
        assert!(!data.is_null());
        assert_eq!(out_len, 6);
        assert_eq!(
            unsafe { std::slice::from_raw_parts(data, out_len) },
            b"\x89PNG\r\n"
        );

        unsafe {
            demo_free_bytes(data, out_len);
            demo_free_document(doc);
        }
    }

    /// The behaviour this macro checks all three arguments before the closure for: a
    /// rejected argument leaves the caller's length variable exactly as they left it, so
    /// "not attempted" stays distinguishable from "attempted and produced nothing".
    #[test]
    fn a_rejected_argument_leaves_out_len_untouched() {
        let doc = demo_with_blob("logo.png", b"\x89PNG");
        let id = CString::new("logo.png").unwrap();
        const SEEDED: usize = 0xDEAD;

        let mut out_len: usize = SEEDED;
        assert!(unsafe { demo_blob(std::ptr::null(), id.as_ptr(), &mut out_len) }.is_null());
        assert_eq!(out_len, SEEDED, "a null handle must not write out_len");
        assert_eq!(demo_last_error_kind(), crate::kind::INVALID_ARGUMENT);

        assert!(unsafe { demo_blob(doc, std::ptr::null(), &mut out_len) }.is_null());
        assert_eq!(out_len, SEEDED, "a null key must not write out_len");
        assert_eq!(demo_last_error_kind(), crate::kind::INVALID_ARGUMENT);

        // A key that is merely absent *is* attempted, so the length is zeroed.
        let missing = CString::new("absent.png").unwrap();
        assert!(unsafe { demo_blob(doc, missing.as_ptr(), &mut out_len) }.is_null());
        assert_eq!(out_len, 0, "a lookup that failed reports zero length");
        assert_eq!(demo_last_error_kind(), crate::kind::OTHER);

        unsafe { demo_free_document(doc) };
    }

    #[test]
    fn a_null_out_len_is_rejected_and_named() {
        let doc = demo_with_blob("logo.png", b"\x89PNG");
        let id = CString::new("logo.png").unwrap();

        assert!(unsafe { demo_blob(doc, id.as_ptr(), std::ptr::null_mut()) }.is_null());
        let message = unsafe { std::ffi::CStr::from_ptr(demo_last_error()) }
            .to_string_lossy()
            .into_owned();
        assert_eq!(message, "out_len is null");

        unsafe { demo_free_document(doc) };
    }

    #[test]
    fn a_non_utf8_key_is_classified_and_zeroes_the_length() {
        let doc = demo_with_blob("logo.png", b"\x89PNG");
        let raw = [0xFFu8, 0x00];

        let mut out_len: usize = 0xBEEF;
        let data = unsafe { demo_blob(doc, raw.as_ptr() as *const c_char, &mut out_len) };
        assert!(data.is_null());
        assert_eq!(demo_last_error_kind(), crate::kind::INVALID_ARGUMENT);
        assert_eq!(
            out_len, 0,
            "the conversion happens inside the closure, so the failure path runs"
        );

        unsafe { demo_free_document(doc) };
    }

    #[test]
    fn an_empty_buffer_is_handed_over_as_a_non_null_pointer_with_zero_length() {
        let doc = demo_with_blob("empty.bin", b"");
        let id = CString::new("empty.bin").unwrap();

        let mut out_len: usize = 9;
        let data = unsafe { demo_blob(doc, id.as_ptr(), &mut out_len) };
        assert!(
            !data.is_null(),
            "an empty blob is a success, not an absent one"
        );
        assert_eq!(out_len, 0);

        unsafe {
            demo_free_bytes(data, out_len);
            demo_free_document(doc);
        }
    }
}

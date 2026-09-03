//! Shared plumbing for the `un*` document extraction family — [unpdf], [undoc], [unhwp].
//!
//! [unpdf]: https://crates.io/crates/unpdf
//! [undoc]: https://crates.io/crates/undoc
//! [unhwp]: https://crates.io/crates/unhwp
//!
//! # What this crate is for
//!
//! Those three libraries have the same shape — parse a container, build an intermediate
//! representation, render Markdown — and expose it the same way: a Rust API, a C ABI, and
//! C#, Python and WebAssembly bindings over that ABI. The *document* parts differ
//! entirely and belong in each library. The parts that do not differ live here: thread-local
//! last-error storage, catching panics before they cross `extern "C"`, and the integer space
//! the error classifications live in. Two more capabilities are planned as opt-in features
//! once their own migrations land: markdown shape-refinement and VLM/AI-assisted extraction
//! — see [Modules](#modules) below for what exists today.
//!
//! # Scope, and what is kept out
//!
//! - **No domain content.** No document model, no rendering, no format knowledge. If a
//!   type would mention a page, a paragraph or a spreadsheet cell, it belongs upstream.
//! - **The crate root has no dependencies.** Three published cdylibs link it statically, so
//!   a dependency here is a dependency in all of them. Everything in [`ffi`], [`kind`] and
//!   [`scaffold`] is `std`-only. Optional capabilities each bring their own dependency
//!   behind their own feature flag instead of widening this baseline.
//!
//! It is `std`-only at the root — [`ffi::LastErrorSlot`] holds a `CString` and lives in a
//! `thread_local!` — which suits every consumer including `wasm32-unknown-unknown`.
//!
//! # Modules
//!
//! - [`kind`] — the error-kind values the family already shares, and the bands that keep
//!   future ones from colliding. Read its docs before adding a kind anywhere.
//! - [`ffi`] — [`ffi::LastErrorSlot`], the [`ffi::catch`] panic guard, and the macros that
//!   export the ABI over them.
//! - [`scaffold`] — macros that assemble an entry point out of those primitives. Reach for
//!   these before writing the five steps by hand; the sentinel differs by return type and
//!   that is where hand-written entry points drift.
//!
//! # A minimal library
//!
//! ```
//! use std::ffi::c_int;
//! use unparser_shared::ffi::{self, FfiError, LastErrorSlot};
//! use unparser_shared::kind;
//!
//! // The slot lives here, not in unparser-shared — that is what keeps it per-library. See
//! // `ffi`'s module docs.
//! thread_local! {
//!     static LAST_ERROR: LastErrorSlot = const { LastErrorSlot::new() };
//! }
//!
//! unparser_shared::export_last_error_abi!(LAST_ERROR, mylib_last_error, mylib_last_error_kind);
//!
//! /// Returns the page count, or -1 on failure.
//! #[no_mangle]
//! pub extern "C" fn mylib_page_count(handle: *const u8) -> c_int {
//!     let result: Result<c_int, FfiError> = ffi::catch(|| {
//!         if handle.is_null() {
//!             return Err(ffi::invalid_argument("handle was null"));
//!         }
//!         Ok(7)
//!     });
//!
//!     match result {
//!         Ok(count) => {
//!             LAST_ERROR.with(|slot| slot.clear());
//!             count
//!         }
//!         Err(error) => {
//!             LAST_ERROR.with(|slot| slot.set_error(&error));
//!             -1
//!         }
//!     }
//! }
//!
//! assert_eq!(mylib_page_count(std::ptr::null()), -1);
//! assert_eq!(mylib_last_error_kind(), kind::INVALID_ARGUMENT);
//!
//! let data = 0u8;
//! assert_eq!(mylib_page_count(&data), 7);
//! assert_eq!(mylib_last_error_kind(), kind::NONE);
//! ```

#![doc(html_root_url = "https://docs.rs/unparser-shared/0.1.0")]

pub mod ffi;
pub mod kind;
pub mod scaffold;

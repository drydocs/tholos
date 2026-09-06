#![no_std]

//! Shared Tholos client import and error mapping for contracts that call into
//! Tholos rather than building their own dispute resolution logic
//! (`demo-consumer` and `asserter-consumer`). Exists so the two consumer
//! example crates don't each carry their own verbatim copy of this: if
//! `tholos::Error` ever gains a variant one of these should surface
//! distinctly, or the mapping logic needs a fix, there's exactly one place to
//! update instead of two that have to be kept in sync by hand.

use soroban_sdk::{contracterror, contractimport};

pub mod tholos {
    use super::*;
    contractimport!(file = "../../target/wasm32v1-none/release/tholos.wasm");
}

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
pub enum Error {
    /// `tholos_id` didn't resolve to an invokable Tholos instance (wrong
    /// address, wrong Wasm, a trap, or a return value this contract couldn't
    /// decode), or the call otherwise failed for a reason that doesn't map to
    /// one of the specific Tholos-side variants below.
    InvalidTholosId = 1,
    /// The Tholos instance at `tholos_id` has not been initialized yet.
    TholosNotInitialized = 2,
    /// The Tholos instance at `tholos_id` is currently paused.
    TholosPaused = 3,
    /// No assertion exists under the given id on the Tholos instance at
    /// `tholos_id`.
    AssertionNotFound = 4,
}

impl Error {
    /// Maps a failure surfaced from a `try_` call against the imported Tholos
    /// client into a consumer contract's own `Error`. Only the variants
    /// `assert_outcome`/`get_assertion_state` can actually return are named
    /// explicitly; anything else (a Tholos-side error a consumer doesn't
    /// expect from those two entry points, or a host-level invocation
    /// failure that never reached Tholos's own error handling at all)
    /// collapses to `InvalidTholosId`, since from a caller's perspective
    /// they all mean the same thing: the call to `tholos_id` didn't work as
    /// expected.
    pub fn from_tholos_call<T, C>(
        result: Result<Result<T, C>, Result<tholos::Error, soroban_sdk::InvokeError>>,
    ) -> Result<T, Error> {
        match result {
            Ok(Ok(value)) => Ok(value),
            Ok(Err(_conversion_error)) => Err(Error::InvalidTholosId),
            Err(Ok(tholos::Error::NotInitialized)) => Err(Error::TholosNotInitialized),
            Err(Ok(tholos::Error::Paused)) => Err(Error::TholosPaused),
            Err(Ok(tholos::Error::AssertionNotFound)) => Err(Error::AssertionNotFound),
            Err(Ok(_other_tholos_error)) => Err(Error::InvalidTholosId),
            Err(Err(_invoke_error)) => Err(Error::InvalidTholosId),
        }
    }
}

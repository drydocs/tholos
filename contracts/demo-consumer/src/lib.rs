#![no_std]

//! Minimal example of a contract that calls into Tholos rather than building its
//! own dispute resolution logic. Exists to validate the pattern documented in
//! INTEGRATION.md actually compiles and works, not as a production contract.

use soroban_sdk::{contract, contracterror, contractimpl, contractimport, Address, Env};

mod tholos {
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
    /// client into this contract's own `Error`. Only the variants
    /// `assert_outcome`/`get_assertion_state` can actually return are named
    /// explicitly; anything else (a Tholos-side error this contract doesn't
    /// expect from those two entry points, or a host-level invocation
    /// failure that never reached Tholos's own error handling at all)
    /// collapses to `InvalidTholosId`, since from this contract's
    /// perspective they all mean the same thing: the call to `tholos_id`
    /// didn't work as expected.
    fn from_tholos_call<T, C>(
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

#[contract]
pub struct DemoConsumer;

#[contractimpl]
impl DemoConsumer {
    /// Posts an assertion to a Tholos instance on behalf of `asserter`, an end
    /// user who signs for it directly. The bond is drawn from and returned to
    /// `asserter`, not this contract. This is the simple integration pattern:
    /// see INTEGRATION.md for what changes if this contract's own address
    /// should be the asserter instead.
    ///
    /// Returns `Error::TholosNotInitialized` or `Error::TholosPaused` if the
    /// Tholos instance at `tholos_id` can't currently accept an assertion, or
    /// `Error::InvalidTholosId` if `tholos_id` doesn't resolve to an
    /// invokable Tholos instance at all.
    pub fn create_assertion(
        env: Env,
        tholos_id: Address,
        asserter: Address,
        outcome: bool,
    ) -> Result<u64, Error> {
        let client = tholos::Client::new(&env, &tholos_id);
        Error::from_tholos_call(client.try_assert_outcome(&asserter, &outcome))
    }

    /// Forwards a read of an assertion's current state. See INTEGRATION.md for
    /// why `Assertion.outcome` is the *claimed* outcome, not necessarily the
    /// final one if the assertion was disputed and overturned.
    ///
    /// Returns `Error::AssertionNotFound` if no assertion exists under `id`
    /// on the Tholos instance at `tholos_id`, or `Error::InvalidTholosId` if
    /// `tholos_id` doesn't resolve to an invokable Tholos instance at all.
    pub fn get_status(env: Env, tholos_id: Address, id: u64) -> Result<tholos::Assertion, Error> {
        let client = tholos::Client::new(&env, &tholos_id);
        Error::from_tholos_call(client.try_get_assertion_state(&id))
    }
}

mod test;

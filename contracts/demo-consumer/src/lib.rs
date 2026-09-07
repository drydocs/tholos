#![no_std]

//! Minimal example of a contract that calls into Tholos rather than building its
//! own dispute resolution logic. Exists to validate the pattern documented in
//! INTEGRATION.md actually compiles and works, not as a production contract.

use soroban_sdk::{contract, contractimpl, Address, Env};
use tholos_client::{tholos, Error};

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

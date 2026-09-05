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
    /// The assertion ID does not exist in the referenced Tholos instance.
    AssertionNotFound = 1,
    /// The Tholos call failed for a reason not mapped above.
    TholosCallFailed = 2,
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
    pub fn create_assertion(
        env: Env,
        tholos_id: Address,
        asserter: Address,
        outcome: bool,
    ) -> Result<u64, Error> {
        let client = tholos::Client::new(&env, &tholos_id);
        client
            .try_assert_outcome(&asserter, &outcome)
            .map_err(|_| Error::TholosCallFailed)
    }

    /// Forwards a read of an assertion's current state. See INTEGRATION.md for
    /// why `Assertion.outcome` is the *claimed* outcome, not necessarily the
    /// final one if the assertion was disputed and overturned.
    pub fn get_status(
        env: Env,
        tholos_id: Address,
        id: u64,
    ) -> Result<tholos::Assertion, Error> {
        let client = tholos::Client::new(&env, &tholos_id);
        client
            .try_get_assertion_state(&id)
            .map_err(|_| Error::AssertionNotFound)
    }
}

mod test;

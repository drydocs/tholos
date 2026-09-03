#![no_std]
#![allow(clippy::too_many_arguments)]

//! Minimal example of a contract that calls into Tholos rather than building its
//! own dispute resolution logic. Exists to validate the pattern documented in
//! INTEGRATION.md actually compiles and works, not as a production contract.

use soroban_sdk::{contract, contractimpl, contractimport, Address, Env};

mod tholos {
    use super::*;
    contractimport!(file = "../../target/wasm32v1-none/release/tholos.wasm");
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
    pub fn create_assertion(env: Env, tholos_id: Address, asserter: Address, outcome: bool) -> u64 {
        let client = tholos::Client::new(&env, &tholos_id);
        client.assert_outcome(&asserter, &outcome)
    }

    /// Forwards a read of an assertion's current state. See INTEGRATION.md for
    /// why `Assertion.outcome` is the *claimed* outcome, not necessarily the
    /// final one if the assertion was disputed and overturned.
    pub fn get_status(env: Env, tholos_id: Address, id: u64) -> tholos::Assertion {
        let client = tholos::Client::new(&env, &tholos_id);
        client.get_assertion_state(&id)
    }
}

mod test;

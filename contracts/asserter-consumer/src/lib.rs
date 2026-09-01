#![no_std]

//! Second integration example: this contract's own address as the asserter,
//! demonstrating the "Your contract's own address as asserter" pattern from
//! INTEGRATION.md. See `demo-consumer` for the simpler, recommended default
//! (end user as asserter) instead.

use soroban_sdk::{
    auth::{ContractContext, InvokerContractAuthEntry, SubContractInvocation},
    contract, contracterror, contractimpl, contractimport, Address, Env, IntoVal, Symbol, Vec,
};

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
pub struct AsserterConsumer;

#[contractimpl]
impl AsserterConsumer {
    /// Posts an assertion with this contract's own address as the asserter, so
    /// the bond pools under this contract rather than an end user. `token_id`
    /// and `bond_amount` must match the Tholos instance's actual configuration
    /// at `tholos_id`: there's no way to query them from Tholos ahead of the
    /// call, so the caller (or this contract's own deployer) has to already
    /// know them, exactly as INTEGRATION.md describes.
    ///
    /// Soroban only auto-grants a contract's implicit self-authorization one
    /// call deep. This call chain is two deep (this contract -> Tholos ->
    /// the token's `transfer`), so the deeper call needs to be explicitly
    /// pre-authorized with `authorize_as_current_contract` before invoking
    /// Tholos, specifying the exact token contract, `transfer` args, and
    /// amount Tholos will end up calling.
    ///
    /// Returns `Error::TholosNotInitialized` or `Error::TholosPaused` if the
    /// Tholos instance at `tholos_id` can't currently accept an assertion, or
    /// `Error::InvalidTholosId` if `tholos_id` doesn't resolve to an
    /// invokable Tholos instance at all.
    pub fn create_assertion_as_self(
        env: Env,
        tholos_id: Address,
        token_id: Address,
        bond_amount: i128,
        outcome: bool,
    ) -> Result<u64, Error> {
        let curr_contract = env.current_contract_address();

        env.authorize_as_current_contract(Vec::from_array(
            &env,
            [InvokerContractAuthEntry::Contract(SubContractInvocation {
                context: ContractContext {
                    contract: token_id,
                    fn_name: Symbol::new(&env, "transfer"),
                    args: Vec::from_array(
                        &env,
                        [
                            curr_contract.into_val(&env),
                            tholos_id.into_val(&env),
                            bond_amount.into_val(&env),
                        ],
                    ),
                },
                sub_invocations: Vec::new(&env),
            })],
        ));

        let client = tholos::Client::new(&env, &tholos_id);
        Error::from_tholos_call(client.try_assert_outcome(&curr_contract, &outcome))
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

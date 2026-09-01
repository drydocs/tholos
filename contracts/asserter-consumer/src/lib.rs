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
    /// The cross-contract call to Tholos failed (e.g. invalid contract address,
    /// trap, or host invocation failure).
    TholosCallFailed = 1,
    /// Tholos has not been initialized.
    NotInitialized = 2,
    /// Tholos is currently paused.
    Paused = 3,
    /// The requested assertion was not found in Tholos.
    AssertionNotFound = 4,
    /// An underlying Tholos contract error was returned.
    TholosError = 5,
}

impl From<tholos::Error> for Error {
    fn from(err: tholos::Error) -> Self {
        match err {
            tholos::Error::NotInitialized => Error::NotInitialized,
            tholos::Error::Paused => Error::Paused,
            tholos::Error::AssertionNotFound => Error::AssertionNotFound,
            _ => Error::TholosError,
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
    /// Returns the new assertion ID, or an [`Error`] if the call or invocation fails
    /// (e.g. [`Error::Paused`], [`Error::NotInitialized`], or [`Error::TholosCallFailed`]).
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
        match client.try_assert_outcome(&curr_contract, &outcome) {
            Ok(Ok(id)) => Ok(id),
            Err(Ok(tholos_err)) => Err(tholos_err.into()),
            _ => Err(Error::TholosCallFailed),
        }
    }

    /// Forwards a read of an assertion's current state. See INTEGRATION.md for
    /// why `Assertion.outcome` is the *claimed* outcome, not necessarily the
    /// final one if the assertion was disputed and overturned.
    ///
    /// Returns the [`tholos::Assertion`], or an [`Error`] if the lookup fails
    /// (e.g. [`Error::AssertionNotFound`] or [`Error::TholosCallFailed`]).
    pub fn get_status(env: Env, tholos_id: Address, id: u64) -> Result<tholos::Assertion, Error> {
        let client = tholos::Client::new(&env, &tholos_id);
        match client.try_get_assertion_state(&id) {
            Ok(Ok(assertion)) => Ok(assertion),
            Err(Ok(tholos_err)) => Err(tholos_err.into()),
            _ => Err(Error::TholosCallFailed),
        }
    }
}

mod test;

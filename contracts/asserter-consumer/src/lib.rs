#![no_std]

//! Second integration example: this contract's own address as the asserter,
//! demonstrating the "Your contract's own address as asserter" pattern from
//! INTEGRATION.md. See `demo-consumer` for the simpler, recommended default
//! (end user as asserter) instead.

use soroban_sdk::{
    auth::{ContractContext, InvokerContractAuthEntry, SubContractInvocation},
    contract, contractimpl, contractimport, Address, Env, IntoVal, Symbol, Vec,
};

mod tholos {
    use super::*;
    contractimport!(file = "../../target/wasm32v1-none/release/tholos.wasm");
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
    pub fn create_assertion_as_self(
        env: Env,
        tholos_id: Address,
        token_id: Address,
        bond_amount: i128,
        outcome: bool,
    ) -> u64 {
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
        client.assert_outcome(&curr_contract, &outcome)
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

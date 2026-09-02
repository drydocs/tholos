#![no_std]

//! Second integration example: this contract's own address as the asserter,
//! demonstrating the "Your contract's own address as asserter" pattern from
//! INTEGRATION.md. See `demo-consumer` for the simpler, recommended default
//! (end user as asserter) instead.

use soroban_sdk::{
    auth::{ContractContext, InvokerContractAuthEntry, SubContractInvocation},
    contract, contracterror, contractimpl, contractimport, contracttype, Address, Env, IntoVal,
    Symbol, Vec,
};

mod tholos {
    use super::*;
    contractimport!(file = "../../target/wasm32v1-none/release/tholos.wasm");
}

#[contracttype]
pub enum DataKey {
    Admin,
    TholosId,
    TokenId,
}

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
pub enum Error {
    AlreadyInitialized = 1,
    NotInitialized = 2,
}

#[contract]
pub struct AsserterConsumer;

#[contractimpl]
impl AsserterConsumer {
    /// Initializes the contract with an admin address and pinned Tholos configuration.
    pub fn initialize(
        env: Env,
        admin: Address,
        tholos_id: Address,
        token_id: Address,
    ) -> Result<(), Error> {
        if env.storage().instance().has(&DataKey::Admin) {
            return Err(Error::AlreadyInitialized);
        }

        admin.require_auth();

        env.storage().instance().set(&DataKey::Admin, &admin);
        env.storage().instance().set(&DataKey::TholosId, &tholos_id);
        env.storage().instance().set(&DataKey::TokenId, &token_id);

        Ok(())
    }

    /// Posts an assertion with this contract's own address as the asserter, so
    /// the bond pools under this contract rather than an end user. Requires
    /// authorization from the configured admin.
    pub fn create_assertion_as_self(
        env: Env,
        bond_amount: i128,
        outcome: bool,
    ) -> Result<u64, Error> {
        let admin: Address = env
            .storage()
            .instance()
            .get(&DataKey::Admin)
            .ok_or(Error::NotInitialized)?;
        let tholos_id: Address = env
            .storage()
            .instance()
            .get(&DataKey::TholosId)
            .ok_or(Error::NotInitialized)?;
        let token_id: Address = env
            .storage()
            .instance()
            .get(&DataKey::TokenId)
            .ok_or(Error::NotInitialized)?;

        admin.require_auth();

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
        Ok(client.assert_outcome(&curr_contract, &outcome))
    }

    /// Forwards a read of an assertion's current state from the configured Tholos instance.
    pub fn get_status(env: Env, id: u64) -> Result<tholos::Assertion, Error> {
        let tholos_id: Address = env
            .storage()
            .instance()
            .get(&DataKey::TholosId)
            .ok_or(Error::NotInitialized)?;

        let client = tholos::Client::new(&env, &tholos_id);
        Ok(client.get_assertion_state(&id))
    }
}

mod test;

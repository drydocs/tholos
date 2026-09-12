#![no_std]

//! Second integration example: this contract's own address as the asserter,
//! demonstrating the "Your contract's own address as asserter" pattern from
//! INTEGRATION.md. See `demo-consumer` for the simpler, recommended default
//! (end user as asserter) instead.

use soroban_sdk::{
    auth::{ContractContext, InvokerContractAuthEntry, SubContractInvocation},
    contract, contractimpl, contracttype, Address, Env, IntoVal, Symbol, Vec,
};
use tholos_client::{tholos, Error};

#[contracttype]
pub enum DataKey {
    Admin,
    TholosId,
    TokenId,
}

#[contract]
pub struct AsserterConsumer;

#[contractimpl]
impl AsserterConsumer {
    /// One-time setup, pinning the trusted Tholos instance and token contract
    /// that `create_assertion_as_self` will interact with. Only callable when
    /// not yet initialized; fails with `AlreadyInitialized` on subsequent calls.
    /// The admin address is stored so `create_assertion_as_self` can be gated
    /// to authorized callers only.
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
    /// the bond pools under this contract rather than an end user. The trusted
    /// `tholos_id` and `token_id` are read from instance storage (set by
    /// `initialize`), not accepted as per-call arguments, preventing an
    /// attacker from draining this contract's balance in an arbitrary token.
    ///
    /// Only callable by the admin configured at `initialize`.
    ///
    /// Soroban only auto-grants a contract's implicit self-authorization one
    /// call deep. This call chain is two deep (this contract -> Tholos ->
    /// the token's `transfer`), so the deeper call needs to be explicitly
    /// pre-authorized with `authorize_as_current_contract` before invoking
    /// Tholos, specifying the exact token contract, `transfer` args, and
    /// amount Tholos will end up calling.
    ///
    /// Returns `Error::NotInitialized` if this contract has not been initialized yet,
    /// `Error::TholosNotInitialized` or `Error::TholosPaused` if the Tholos instance
    /// at `tholos_id` can't currently accept an assertion, or `Error::InvalidTholosId`
    /// if `tholos_id` doesn't resolve to an invokable Tholos instance at all.
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
        admin.require_auth();

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

    /// Forwards a read of an assertion's current state. Uses the trusted
    /// `tholos_id` stored at `initialize`. See INTEGRATION.md for why
    /// `Assertion.outcome` is the *claimed* outcome, not necessarily the
    /// final one if the assertion was disputed and overturned.
    ///
    /// Returns `Error::NotInitialized` if this contract has not been initialized yet,
    /// `Error::AssertionNotFound` if no assertion exists under `id` on the Tholos
    /// instance at `tholos_id`, or `Error::InvalidTholosId` if `tholos_id` doesn't
    /// resolve to an invokable Tholos instance at all.
    pub fn get_status(env: Env, id: u64) -> Result<tholos::Assertion, Error> {
        let tholos_id: Address = env
            .storage()
            .instance()
            .get(&DataKey::TholosId)
            .ok_or(Error::NotInitialized)?;
        let client = tholos::Client::new(&env, &tholos_id);
        Error::from_tholos_call(client.try_get_assertion_state(&id))
    }
}

mod test;

#![no_std]

//! Second integration example: this contract's own address as the asserter,
//! demonstrating the "Your contract's own address as asserter" pattern from
//! INTEGRATION.md. See `demo-consumer` for the simpler, recommended
//! default (end user as asserter) instead.

use soroban_sdk::{
    auth::{ContractContext, InvokerContractAuthEntry, SubContractInvocation},
    contract, contractimpl, contracttype, Address, Env, IntoVal, Symbol, Vec,
};
use tholos_client::{tholos, Error};

const DAY_IN_LEDGERS: u32 = 17280;
const INSTANCE_BUMP_AMOUNT: u32 = 30 * DAY_IN_LEDGERS;
const INSTANCE_LIFETIME_THRESHOLD: u32 = INSTANCE_BUMP_AMOUNT - DAY_IN_LEDGERS;

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
    /// Pins the admin atomically at deploy time.
    ///
    /// This is a `__constructor` rather than an `admin` argument to
    /// `initialize` on purpose. A caller-supplied admin in `initialize`
    /// is front-runnable: the first caller wins and chooses the admin, which
    /// for this contract would mean choosing who may spend its balance.
    /// `contracts/tholos` moved its own admin pinning into a constructor
    /// for exactly this reason; this follows that pattern.
    pub fn __constructor(env: Env, admin: Address) {
        admin.require_auth();

        env.storage().instance().set(&DataKey::Admin, &admin);
        Self::touch_instance_ttl(&env);
    }

    /// Stores the trusted Tholos instance and token addresses once, after
    /// which they can never be changed.
    ///
    /// Both are fixed for the life of the contract because they are what
    /// `create_assertion_as_self` pre-authorizes a transfer with. If either
    /// could be supplied per call, the caller could name a token this contract
    /// holds and a destination it controls, and the pre-authorization would
    /// satisfy the auth requirement for moving this contract's own balance.
    /// Keeping them in storage removes that input entirely rather than
    /// validating it.
    ///
    /// Requires the signature of the admin pinned at deploy time. Fails with
    /// `Error::AlreadyInitialized` if called twice.
    pub fn initialize(env: Env, tholos_id: Address, token_id: Address) -> Result<(), Error> {
        let admin = Self::admin(&env)?;
        admin.require_auth();

        if env.storage().instance().has(&DataKey::TholosId) {
            return Err(Error::AlreadyInitialized);
        }

        env.storage().instance().set(&DataKey::TholosId, &tholos_id);
        env.storage().instance().set(&DataKey::TokenId, &token_id);
        Self::touch_instance_ttl(&env);

        Ok(())
    }

    /// Posts an assertion with this contract's own address as the asserter, so
    /// the bond pools under this contract rather than an end user.
    ///
    /// Both the token being spent and the Tholos instance being paid are read
    /// from storage, never from arguments: the caller cannot influence the
    /// transfer this contract authorizes. `bond_amount` is the only
    /// remaining caller-supplied value, and it is bounded by this contract's
    /// own balance. The call is admin-gated, because it spends the contract's
    /// funds.
    ///
    /// Soroban only auto-grants a contract's implicit self-authorization one
    /// call deep. This call chain is two deep (this contract -> Tholos -> the
    /// token's `transfer`), so the deeper call is explicitly pre-authorized
    /// with `authorize_as_current_contract` before invoking Tholos,
    /// specifying the exact token contract, `transfer` args, and amount
    /// Tholos will end up calling.
    ///
    /// Returns `Error::NotInitialized` if `initialize` has not run,
    /// `Error::TholosNotInitialized` or `Error::TholosPaused` if the
    /// trusted Tholos instance cannot currently accept an assertion, or
    /// `Error::InvalidTholosId` if it does not resolve to an invokable
    /// Tholos instance at all.
    pub fn create_assertion_as_self(
        env: Env,
        bond_amount: i128,
        outcome: bool,
    ) -> Result<u64, Error> {
        let admin = Self::admin(&env)?;
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
                            tholos_id.clone().into_val(&env),
                            bond_amount.into_val(&env),
                        ],
                    ),
                },
                sub_invocations: Vec::new(&env),
            })],
        ));

        let client = tholos::Client::new(&env, &tholos_id);
        let result = Error::from_tholos_call(client.try_assert_outcome(&curr_contract, &outcome));

        if result.is_ok() {
            Self::touch_instance_ttl(&env);
        }

        result
    }

    /// Forwards a read of an assertion's current state. See INTEGRATION.md for
    /// why `Assertion.outcome` is the *claimed* outcome, not necessarily
    /// the final one if the assertion was disputed and overturned.
    ///
    /// This keeps taking `tholos_id` rather than reading the configured
    /// instance: it moves no funds, so an arbitrary instance here is a read the
    /// caller could already make directly, and narrowing it would widen this
    /// change without closing anything.
    ///
    /// Returns `Error::AssertionNotFound` if no assertion exists under
    /// `id` on the Tholos instance at `tholos_id`, or
    /// `Error::InvalidTholosId` if `tholos_id` does not resolve to an
    /// invokable Tholos instance at all.
    pub fn get_status(env: Env, tholos_id: Address, id: u64) -> Result<tholos::Assertion, Error> {
        let client = tholos::Client::new(&env, &tholos_id);
        Error::from_tholos_call(client.try_get_assertion_state(&id))
    }

    /// The admin pinned by the constructor.
    ///
    /// `NotInitialized` here is defensive rather than reachable in
    /// practice: every live instance has already run its constructor by the
    /// time any call reaches this, and the constructor is the only writer of
    /// this key.
    fn admin(env: &Env) -> Result<Address, Error> {
        env.storage()
            .instance()
            .get(&DataKey::Admin)
            .ok_or(Error::NotInitialized)
    }

    /// Renews instance storage TTL, so the trusted addresses and the admin
    /// cannot be archived out from under a contract that is still in use.
    fn touch_instance_ttl(env: &Env) {
        env.storage()
            .instance()
            .extend_ttl(INSTANCE_LIFETIME_THRESHOLD, INSTANCE_BUMP_AMOUNT);
    }
}

mod test;

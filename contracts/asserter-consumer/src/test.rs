#![cfg(test)]

use super::*;
use soroban_sdk::testutils::{Address as _, MockAuth, MockAuthInvoke};
use soroban_sdk::{token, IntoVal};

/// A Tholos instance, a token, and an AsserterConsumer whose admin is pinned
/// at deploy, all sharing one ledger.
struct Fixture {
    env: Env,
    consumer_admin: Address,
    token_admin: Address,
    tholos_id: Address,
    tholos_client: tholos::Client<'static>,
    token_id: Address,
    token_asset_client: token::StellarAssetClient<'static>,
    consumer_id: Address,
    consumer_client: AsserterConsumerClient<'static>,
    resolvers: Vec<Address>,
    bond_amount: i128,
}

impl Fixture {
    fn new() -> Self {
        let env = Env::default();

        // Covers both constructors' admin.require_auth(): each admin is now
        // pinned atomically at deploy, so it applies to registration itself,
        // before anything else runs. Registering Tholos from imported WASM
        // records its constructor's auth as non-root, so the non-root variant
        // is needed here specifically.
        env.mock_all_auths_allowing_non_root_auth();

        let tholos_admin = Address::generate(&env);
        let tholos_id = env.register(tholos::WASM, (tholos_admin,));
        let tholos_client = tholos::Client::new(&env, &tholos_id);

        let token_admin = Address::generate(&env);
        let token_id = env
            .register_stellar_asset_contract_v2(token_admin.clone())
            .address();
        let token_asset_client = token::StellarAssetClient::new(&env, &token_id);

        let resolvers = Vec::from_array(
            &env,
            [
                Address::generate(&env),
                Address::generate(&env),
                Address::generate(&env),
            ],
        );

        let consumer_admin = Address::generate(&env);
        let consumer_id = env.register(AsserterConsumer, (consumer_admin.clone(),));
        let consumer_client = AsserterConsumerClient::new(&env, &consumer_id);

        Fixture {
            env,
            consumer_admin,
            token_admin,
            tholos_id,
            tholos_client,
            token_id,
            token_asset_client,
            consumer_id,
            consumer_client,
            resolvers,
            bond_amount: 100,
        }
    }

    fn initialize_tholos(&self) {
        self.env.mock_all_auths();
        self.tholos_client.initialize(
            &self.token_id,
            &self.bond_amount,
            &3600,
            &self.resolvers,
            &0u32,
        );
    }

    /// Points the consumer at a specific instance and token. Used by the
    /// error-path tests, which need the consumer ready against something other
    /// than a healthy Tholos instance.
    fn initialize_consumer_at(&self, tholos_id: &Address, token_id: &Address) {
        self.env.mock_all_auths();
        self.consumer_client.initialize(tholos_id, token_id);
    }

    fn initialize_consumer(&self) {
        self.initialize_consumer_at(&self.tholos_id, &self.token_id);
    }

    fn fund_consumer(&self, amount: i128) {
        self.env.mock_all_auths();
        self.token_asset_client.mint(&self.consumer_id, &amount);
    }
}

/// End-to-end happy path, deliberately narrow-mocking auth past construction.
///
/// This test exists specifically to prove that `authorize_as_current_contract`
/// grants the real nested auth Tholos's `assert_outcome` needs for its token
/// transfer, without a blanket `mock_all_auths()` papering over a bug in that
/// mechanism. Only the mint and the consumer admin's own signature are mocked;
/// the transfer out of this contract is authorized by the pre-authorization.
#[test]
fn test_asserter_consumer_can_assert_as_itself_through_tholos() {
    let f = Fixture::new();
    f.initialize_tholos();
    f.initialize_consumer();

    // The bond comes from this contract's own balance, not an end user's.
    f.env.mock_auths(&[MockAuth {
        address: &f.token_admin,
        invoke: &MockAuthInvoke {
            contract: &f.token_id,
            fn_name: "mint",
            args: (f.consumer_id.clone(), 1_000i128).into_val(&f.env),
            sub_invokes: &[],
        },
    }]);
    f.token_asset_client.mint(&f.consumer_id, &1_000);

    // Only the configured admin signs. Nothing here names a destination.
    f.env.mock_auths(&[MockAuth {
        address: &f.consumer_admin,
        invoke: &MockAuthInvoke {
            contract: &f.consumer_id,
            fn_name: "create_assertion_as_self",
            args: (f.bond_amount, true).into_val(&f.env),
            sub_invokes: &[],
        },
    }]);

    let id = f
        .consumer_client
        .create_assertion_as_self(&f.bond_amount, &true);

    let state = f.consumer_client.get_status(&f.tholos_id, &id);
    assert!(state.outcome);
    assert_eq!(state.asserter, f.consumer_id);

    let token = token::Client::new(&f.env, &f.token_id);
    assert_eq!(token.balance(&f.consumer_id), 900);
    // The bond landed on the *configured* Tholos instance, and it is the only
    // destination the pre-authorization can name: there is no argument left
    // that could redirect it, which is what closes the drain.
    assert_eq!(token.balance(&f.tholos_id), f.bond_amount);
}

#[test]
fn test_create_assertion_as_self_before_initialize_is_rejected() {
    let f = Fixture::new();
    f.initialize_tholos();

    // Consumer never initialized: there is no trusted instance or token yet.
    assert_eq!(
        f.consumer_client
            .try_create_assertion_as_self(&f.bond_amount, &true),
        Err(Ok(Error::NotInitialized))
    );
}

#[test]
fn test_initialize_cannot_run_twice() {
    let f = Fixture::new();
    f.initialize_consumer();

    // The trusted addresses are write-once, so a later caller cannot re-point
    // the contract at an instance or token of their choosing.
    let other = Address::generate(&f.env);
    assert_eq!(
        f.consumer_client.try_initialize(&other, &other),
        Err(Ok(Error::AlreadyInitialized))
    );
}

/// With no auth mocked at all, the pinned admin's signature is the only thing
/// that could satisfy `initialize`.
#[test]
#[should_panic]
fn test_initialize_requires_the_configured_admin() {
    let f = Fixture::new();
    f.env.set_auths(&[]);

    f.consumer_client.initialize(&f.tholos_id, &f.token_id);
}

/// The same for the entrypoint that spends the contract's balance: an address
/// that is not the configured admin cannot trigger the self-authorized
/// transfer at all.
#[test]
#[should_panic]
fn test_create_assertion_as_self_requires_the_configured_admin() {
    let f = Fixture::new();
    f.initialize_tholos();
    f.initialize_consumer();
    f.fund_consumer(1_000);

    f.env.set_auths(&[]);

    f.consumer_client
        .create_assertion_as_self(&f.bond_amount, &true);
}

#[test]
fn test_create_assertion_as_self_fails_against_uninitialized_tholos() {
    let f = Fixture::new();

    // Consumer ready, but its Tholos instance never initialized. Tholos
    // rejects with NotInitialized before ever reaching the token transfer.
    f.initialize_consumer();

    assert_eq!(
        f.consumer_client
            .try_create_assertion_as_self(&f.bond_amount, &true),
        Err(Ok(Error::TholosNotInitialized))
    );
}

#[test]
fn test_create_assertion_as_self_fails_when_tholos_paused() {
    let f = Fixture::new();
    f.initialize_tholos();
    f.initialize_consumer();

    f.env.mock_all_auths();
    f.tholos_client.set_paused(&true);

    assert_eq!(
        f.consumer_client
            .try_create_assertion_as_self(&f.bond_amount, &true),
        Err(Ok(Error::TholosPaused))
    );
}

#[test]
fn test_create_assertion_as_self_fails_for_invalid_tholos_id() {
    let f = Fixture::new();

    // An address with no contract registered at all: the call cannot even
    // reach Tholos's own error handling, so this exercises the
    // Err(Err(InvokeError)) -> InvalidTholosId path, not Err(Ok(_)).
    let not_a_tholos_instance = Address::generate(&f.env);
    f.initialize_consumer_at(&not_a_tholos_instance, &f.token_id);

    assert_eq!(
        f.consumer_client
            .try_create_assertion_as_self(&f.bond_amount, &true),
        Err(Ok(Error::InvalidTholosId))
    );
}

#[test]
fn test_get_status_fails_for_nonexistent_assertion() {
    let f = Fixture::new();
    f.initialize_tholos();

    assert_eq!(
        f.consumer_client.try_get_status(&f.tholos_id, &999),
        Err(Ok(Error::AssertionNotFound))
    );
}

#[test]
fn test_get_status_fails_for_invalid_tholos_id() {
    let f = Fixture::new();
    let not_a_tholos_instance = Address::generate(&f.env);

    let result = f.consumer_client.try_get_status(&not_a_tholos_instance, &0);
    assert_eq!(result, Err(Ok(Error::InvalidTholosId)));
}

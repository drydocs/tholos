#![cfg(test)]

use super::*;
use soroban_sdk::testutils::{Address as _, MockAuth, MockAuthInvoke};
use soroban_sdk::{token, IntoVal};

#[test]
fn test_asserter_consumer_can_assert_as_itself_through_tholos() {
    let env = Env::default();

    // Deliberately not using mock_all_auths(): this test exists specifically to
    // prove authorize_as_current_contract grants the real nested auth Tholos's
    // assert_outcome needs for its token transfer, without blanket auth mocking
    // papering over a bug in that mechanism. Only the admin's initialize call
    // (a genuine top-level signature this test can't otherwise provide) is
    // mocked, and only for that one call.
    let tholos_id = env.register(tholos::WASM, ());
    let tholos_client = tholos::Client::new(&env, &tholos_id);

    let token_admin = Address::generate(&env);
    let token_contract = env.register_stellar_asset_contract_v2(token_admin.clone());
    let token_id = token_contract.address();
    let token_asset_client = token::StellarAssetClient::new(&env, &token_id);

    let admin = Address::generate(&env);
    let resolvers = Vec::from_array(
        &env,
        [
            Address::generate(&env),
            Address::generate(&env),
            Address::generate(&env),
        ],
    );
    let bond_amount: i128 = 100;

    env.mock_auths(&[MockAuth {
        address: &admin,
        invoke: &MockAuthInvoke {
            contract: &tholos_id,
            fn_name: "initialize",
            args: (
                admin.clone(),
                token_id.clone(),
                bond_amount,
                3600u64,
                resolvers.clone(),
                0u32,
            )
                .into_val(&env),
            sub_invokes: &[],
        },
    }]);
    tholos_client.initialize(&admin, &token_id, &bond_amount, &3600, &resolvers, &0u32);

    let consumer_id = env.register(AsserterConsumer, ());
    let consumer_client = AsserterConsumerClient::new(&env, &consumer_id);

    // The bond comes from this contract's own balance, not an end user's.
    env.mock_auths(&[MockAuth {
        address: &token_admin,
        invoke: &MockAuthInvoke {
            contract: &token_id,
            fn_name: "mint",
            args: (consumer_id.clone(), 1_000i128).into_val(&env),
            sub_invokes: &[],
        },
    }]);
    token_asset_client.mint(&consumer_id, &1_000);

    let id = consumer_client.create_assertion_as_self(&tholos_id, &token_id, &bond_amount, &true);

    let state = consumer_client.get_status(&tholos_id, &id);
    assert!(state.outcome);
    assert_eq!(state.asserter, consumer_id);
    assert_eq!(
        token::Client::new(&env, &token_id).balance(&consumer_id),
        900
    );
}

/// A registered but uninitialized token, resolver committee, and Tholos
/// instance, plus an AsserterConsumer pointed at it. Shared setup for the
/// error-path tests below, which don't need the full funded/authorized flow
/// the happy-path test above exercises.
struct Fixture {
    env: Env,
    tholos_id: Address,
    tholos_client: tholos::Client<'static>,
    token_id: Address,
    consumer_client: AsserterConsumerClient<'static>,
    admin: Address,
    resolvers: Vec<Address>,
    bond_amount: i128,
}

impl Fixture {
    fn new() -> Self {
        let env = Env::default();

        let tholos_id = env.register(tholos::WASM, ());
        let tholos_client = tholos::Client::new(&env, &tholos_id);

        let token_admin = Address::generate(&env);
        let token_id = env
            .register_stellar_asset_contract_v2(token_admin)
            .address();

        let admin = Address::generate(&env);
        let resolvers = Vec::from_array(
            &env,
            [
                Address::generate(&env),
                Address::generate(&env),
                Address::generate(&env),
            ],
        );

        let consumer_id = env.register(AsserterConsumer, ());
        let consumer_client = AsserterConsumerClient::new(&env, &consumer_id);

        Fixture {
            env,
            tholos_id,
            tholos_client,
            token_id,
            consumer_client,
            admin,
            resolvers,
            bond_amount: 100,
        }
    }

    fn initialize_tholos(&self) {
        self.env.mock_all_auths();
        self.tholos_client.initialize(
            &self.admin,
            &self.token_id,
            &self.bond_amount,
            &3600,
            &self.resolvers,
            &0u32,
        );
    }
}

#[test]
fn test_create_assertion_as_self_fails_against_uninitialized_tholos() {
    let f = Fixture::new();

    // No initialize() call: Tholos rejects with NotInitialized before ever
    // reaching the token transfer, so this doesn't need mocked auths either.
    assert_eq!(
        f.consumer_client.try_create_assertion_as_self(
            &f.tholos_id,
            &f.token_id,
            &f.bond_amount,
            &true
        ),
        Err(Ok(Error::TholosNotInitialized))
    );
}

#[test]
fn test_create_assertion_as_self_fails_when_tholos_paused() {
    let f = Fixture::new();
    f.initialize_tholos();

    f.env.mock_all_auths();
    f.tholos_client.set_paused(&true);

    assert_eq!(
        f.consumer_client.try_create_assertion_as_self(
            &f.tholos_id,
            &f.token_id,
            &f.bond_amount,
            &true
        ),
        Err(Ok(Error::TholosPaused))
    );
}

#[test]
fn test_create_assertion_as_self_fails_for_invalid_tholos_id() {
    let f = Fixture::new();

    // An address with no contract registered at all: the call can't even
    // reach Tholos's own error handling, so this exercises the
    // Err(Err(InvokeError)) -> InvalidTholosId path, not Err(Ok(_)).
    let not_a_tholos_instance = Address::generate(&f.env);

    let result = f.consumer_client.try_create_assertion_as_self(
        &not_a_tholos_instance,
        &f.token_id,
        &f.bond_amount,
        &true,
    );
    assert_eq!(result, Err(Ok(Error::InvalidTholosId)));
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

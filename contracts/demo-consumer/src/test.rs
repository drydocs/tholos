#![cfg(test)]

use super::*;
use soroban_sdk::testutils::Address as _;
use soroban_sdk::{token, Vec};

#[test]
fn test_demo_consumer_can_assert_and_read_status_through_tholos() {
    let env = Env::default();
    // The asserter signs indirectly (as an argument to `create_assertion`
    // rather than the top-level call), so this needs non-root auth mocking.
    // See INTEGRATION.md for what this implies for real deployments.
    env.mock_all_auths_allowing_non_root_auth();

    // Deploy the real Tholos contract from its compiled wasm, not a mock, so this
    // actually validates the cross-contract call pattern from INTEGRATION.md.
    let tholos_id = env.register(tholos::WASM, ());
    let tholos_client = tholos::Client::new(&env, &tholos_id);

    let token_admin = Address::generate(&env);
    let token_contract = env.register_stellar_asset_contract_v2(token_admin);
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
    tholos_client.initialize(&admin, &token_id, &100, &3600, &resolvers, &0u32);

    let consumer_id = env.register(DemoConsumer, ());
    let consumer_client = DemoConsumerClient::new(&env, &consumer_id);

    // An end user signs and funds the bond directly; the demo contract just
    // relays the call.
    let asserter = Address::generate(&env);
    token_asset_client.mint(&asserter, &1_000);

    let id = consumer_client.create_assertion(&tholos_id, &asserter, &true);

    let state = consumer_client.get_status(&tholos_id, &id);
    assert!(state.outcome);
    assert_eq!(state.asserter, asserter);
    assert_eq!(token::Client::new(&env, &token_id).balance(&asserter), 900);
}

/// A registered but uninitialized token, resolver committee, and Tholos
/// instance, plus a DemoConsumer pointed at it. Shared setup for the
/// error-path tests below, which don't need the funded end-user flow the
/// happy-path test above exercises.
struct Fixture {
    env: Env,
    tholos_id: Address,
    tholos_client: tholos::Client<'static>,
    token_id: Address,
    consumer_client: DemoConsumerClient<'static>,
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

        let consumer_id = env.register(DemoConsumer, ());
        let consumer_client = DemoConsumerClient::new(&env, &consumer_id);

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
fn test_create_assertion_fails_against_uninitialized_tholos() {
    let f = Fixture::new();
    let asserter = Address::generate(&f.env);

    // No initialize() call: Tholos rejects with NotInitialized before ever
    // reaching the token transfer, so this doesn't need mocked auths either.
    assert_eq!(
        f.consumer_client
            .try_create_assertion(&f.tholos_id, &asserter, &true),
        Err(Ok(Error::TholosNotInitialized))
    );
}

#[test]
fn test_create_assertion_fails_when_tholos_paused() {
    let f = Fixture::new();
    f.initialize_tholos();

    f.env.mock_all_auths();
    f.tholos_client.set_paused(&true);

    let asserter = Address::generate(&f.env);
    assert_eq!(
        f.consumer_client
            .try_create_assertion(&f.tholos_id, &asserter, &true),
        Err(Ok(Error::TholosPaused))
    );
}

#[test]
fn test_create_assertion_fails_for_invalid_tholos_id() {
    let f = Fixture::new();
    let asserter = Address::generate(&f.env);

    // An address with no contract registered at all: the call can't even
    // reach Tholos's own error handling, so this exercises the
    // Err(Err(InvokeError)) -> InvalidTholosId path, not Err(Ok(_)).
    let not_a_tholos_instance = Address::generate(&f.env);

    let result = f
        .consumer_client
        .try_create_assertion(&not_a_tholos_instance, &asserter, &true);
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

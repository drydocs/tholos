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

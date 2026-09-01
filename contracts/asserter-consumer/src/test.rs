#![cfg(test)]

use super::*;
use soroban_sdk::testutils::{Address as _, MockAuth, MockAuthInvoke};
use soroban_sdk::{token, IntoVal};

struct Fixture {
    env: Env,
    tholos_id: Address,
    token_id: Address,
    admin: Address,
    bond_amount: i128,
    consumer_id: Address,
}

fn setup() -> Fixture {
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

    Fixture {
        env,
        tholos_id,
        token_id,
        admin,
        bond_amount,
        consumer_id,
    }
}

#[test]
fn test_asserter_consumer_can_assert_as_itself_through_tholos() {
    let f = setup();
    let consumer_client = AsserterConsumerClient::new(&f.env, &f.consumer_id);

    let id =
        consumer_client.create_assertion_as_self(&f.tholos_id, &f.token_id, &f.bond_amount, &true);

    let state = consumer_client.get_status(&f.tholos_id, &id);
    assert!(state.outcome);
    assert_eq!(state.asserter, f.consumer_id);
    assert_eq!(
        token::Client::new(&f.env, &f.token_id).balance(&f.consumer_id),
        900
    );
}

#[test]
fn test_asserter_consumer_fails_when_tholos_is_paused() {
    let f = setup();
    let tholos_client = tholos::Client::new(&f.env, &f.tholos_id);
    let consumer_client = AsserterConsumerClient::new(&f.env, &f.consumer_id);

    f.env.mock_auths(&[MockAuth {
        address: &f.admin,
        invoke: &MockAuthInvoke {
            contract: &f.tholos_id,
            fn_name: "set_paused",
            args: (true,).into_val(&f.env),
            sub_invokes: &[],
        },
    }]);
    tholos_client.set_paused(&true);

    let result = consumer_client.try_create_assertion_as_self(
        &f.tholos_id,
        &f.token_id,
        &f.bond_amount,
        &true,
    );
    assert_eq!(result, Err(Ok(Error::Paused)));
}

#[test]
fn test_asserter_consumer_fails_when_tholos_not_initialized() {
    let f = setup();
    let uninit_tholos_id = f.env.register(tholos::WASM, ());
    let consumer_client = AsserterConsumerClient::new(&f.env, &f.consumer_id);

    let result = consumer_client.try_create_assertion_as_self(
        &uninit_tholos_id,
        &f.token_id,
        &f.bond_amount,
        &true,
    );
    assert_eq!(result, Err(Ok(Error::NotInitialized)));
}

#[test]
fn test_asserter_consumer_fails_when_tholos_id_invalid() {
    let f = setup();
    let invalid_tholos_id = Address::generate(&f.env);
    let consumer_client = AsserterConsumerClient::new(&f.env, &f.consumer_id);

    let result = consumer_client.try_create_assertion_as_self(
        &invalid_tholos_id,
        &f.token_id,
        &f.bond_amount,
        &true,
    );
    assert_eq!(result, Err(Ok(Error::TholosCallFailed)));
}

#[test]
fn test_asserter_consumer_fails_when_token_transfer_fails() {
    let f = setup();
    // Register an unfunded consumer (0 balance) so token transfer fails inside Tholos
    let unfunded_consumer_id = f.env.register(AsserterConsumer, ());
    let unfunded_consumer_client = AsserterConsumerClient::new(&f.env, &unfunded_consumer_id);

    let result = unfunded_consumer_client.try_create_assertion_as_self(
        &f.tholos_id,
        &f.token_id,
        &f.bond_amount,
        &true,
    );
    assert_eq!(result, Err(Ok(Error::TholosError)));
}

#[test]
fn test_asserter_consumer_get_status_assertion_not_found() {
    let f = setup();
    let consumer_client = AsserterConsumerClient::new(&f.env, &f.consumer_id);

    let result = consumer_client.try_get_status(&f.tholos_id, &999u64);
    assert_eq!(result, Err(Ok(Error::AssertionNotFound)));
}

#[test]
fn test_asserter_consumer_get_status_fails_on_invalid_tholos_id() {
    let f = setup();
    let invalid_tholos_id = Address::generate(&f.env);
    let consumer_client = AsserterConsumerClient::new(&f.env, &f.consumer_id);

    let result = consumer_client.try_get_status(&invalid_tholos_id, &0u64);
    assert_eq!(result, Err(Ok(Error::TholosCallFailed)));
}

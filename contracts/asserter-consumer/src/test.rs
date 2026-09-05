#![cfg(test)]

use super::*;
use soroban_sdk::testutils::{Address as _, MockAuth, MockAuthInvoke};
use soroban_sdk::{token, IntoVal};

#[test]
fn test_asserter_consumer_can_assert_as_itself_through_tholos() {
    let env = Env::default();

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

    // Initialize the consumer contract, pinning tholos_id and token_id.
    env.mock_auths(&[MockAuth {
        address: &admin,
        invoke: &MockAuthInvoke {
            contract: &consumer_id,
            fn_name: "initialize",
            args: (admin.clone(), tholos_id.clone(), token_id.clone()).into_val(&env),
            sub_invokes: &[],
        },
    }]);
    consumer_client.initialize(&admin, &tholos_id, &token_id);

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

    // Only admin can call create_assertion_as_self.
    env.mock_auths(&[MockAuth {
        address: &admin,
        invoke: &MockAuthInvoke {
            contract: &consumer_id,
            fn_name: "create_assertion_as_self",
            args: (bond_amount, true).into_val(&env),
            sub_invokes: &[],
        },
    }]);
    let id = consumer_client.create_assertion_as_self(&bond_amount, &true);

    let state = consumer_client.get_status(&id);
    assert!(state.outcome);
    assert_eq!(state.asserter, consumer_id);
    assert_eq!(
        token::Client::new(&env, &token_id).balance(&consumer_id),
        900
    );
}

#[test]
fn test_create_assertion_as_self_rejects_before_initialize() {
    let env = Env::default();
    let consumer_id = env.register(AsserterConsumer, ());
    let consumer_client = AsserterConsumerClient::new(&env, &consumer_id);

    let result = consumer_client.try_create_assertion_as_self(&100, &true);
    assert_eq!(result, Err(Ok(Error::NotInitialized)));
}

#[test]
fn test_initialize_rejects_second_call() {
    let env = Env::default();
    let admin = Address::generate(&env);
    let tholos_id = Address::generate(&env);
    let token_id = Address::generate(&env);

    let consumer_id = env.register(AsserterConsumer, ());
    let consumer_client = AsserterConsumerClient::new(&env, &consumer_id);

    env.mock_auths(&[MockAuth {
        address: &admin,
        invoke: &MockAuthInvoke {
            contract: &consumer_id,
            fn_name: "initialize",
            args: (admin.clone(), tholos_id.clone(), token_id.clone()).into_val(&env),
            sub_invokes: &[],
        },
    }]);
    consumer_client.initialize(&admin, &tholos_id, &token_id);

    let result = consumer_client.try_initialize(&admin, &tholos_id, &token_id);
    assert_eq!(result, Err(Ok(Error::AlreadyInitialized)));
}

#[test]
fn test_create_assertion_as_self_rejects_unauthorized_caller() {
    let env = Env::default();
    let admin = Address::generate(&env);
    let attacker = Address::generate(&env);
    let tholos_id = Address::generate(&env);
    let token_id = Address::generate(&env);

    let consumer_id = env.register(AsserterConsumer, ());
    let consumer_client = AsserterConsumerClient::new(&env, &consumer_id);

    env.mock_auths(&[MockAuth {
        address: &admin,
        invoke: &MockAuthInvoke {
            contract: &consumer_id,
            fn_name: "initialize",
            args: (admin.clone(), tholos_id.clone(), token_id.clone()).into_val(&env),
            sub_invokes: &[],
        },
    }]);
    consumer_client.initialize(&admin, &tholos_id, &token_id);

    // Attacker tries to call create_assertion_as_self.
    env.mock_auths(&[MockAuth {
        address: &attacker,
        invoke: &MockAuthInvoke {
            contract: &consumer_id,
            fn_name: "create_assertion_as_self",
            args: (100i128, true).into_val(&env),
            sub_invokes: &[],
        },
    }]);
    let result = consumer_client.try_create_assertion_as_self(&100, &true);
    // Should fail because attacker is not the admin.
    assert!(result.is_err());
}

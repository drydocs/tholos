#![cfg(test)]

use super::*;
use soroban_sdk::testutils::{Address as _, Ledger};

fn setup(env: &Env) -> (Address, Address, token::Client<'static>, Vec<Address>) {
    let admin = Address::generate(env);
    let token_admin = Address::generate(env);
    let token_contract = env.register_stellar_asset_contract_v2(token_admin.clone());
    let token = token::Client::new(env, &token_contract.address());
    let token_asset_client = token::StellarAssetClient::new(env, &token_contract.address());

    let resolvers = Vec::from_array(
        env,
        [
            Address::generate(env),
            Address::generate(env),
            Address::generate(env),
        ],
    );

    let _ = admin;
    let _ = token_asset_client;
    (token_admin, token_contract.address(), token, resolvers)
}

#[test]
fn test_uncontested_assertion_finalizes() {
    let env = Env::default();
    env.mock_all_auths();

    let (_token_admin, token_id, token, resolvers) = setup(&env);
    let token_asset_client = token::StellarAssetClient::new(&env, &token_id);

    let contract_id = env.register(Tholos, ());
    let client = TholosClient::new(&env, &contract_id);

    let admin = Address::generate(&env);
    let asserter = Address::generate(&env);
    token_asset_client.mint(&asserter, &1_000);

    client.initialize(&admin, &token_id, &100, &3600, &resolvers);

    let id = client.assert_outcome(&asserter, &true);
    assert_eq!(token.balance(&asserter), 900);

    env.ledger().with_mut(|l| l.timestamp += 3601);

    let outcome = client.finalize(&id);
    assert!(outcome);
    assert_eq!(token.balance(&asserter), 1_000);
}

#[test]
fn test_disputed_assertion_pays_winner() {
    let env = Env::default();
    env.mock_all_auths();

    let (_token_admin, token_id, token, resolvers) = setup(&env);
    let token_asset_client = token::StellarAssetClient::new(&env, &token_id);

    let contract_id = env.register(Tholos, ());
    let client = TholosClient::new(&env, &contract_id);

    let admin = Address::generate(&env);
    let asserter = Address::generate(&env);
    let disputer = Address::generate(&env);
    token_asset_client.mint(&asserter, &1_000);
    token_asset_client.mint(&disputer, &1_000);

    client.initialize(&admin, &token_id, &100, &3600, &resolvers);

    let id = client.assert_outcome(&asserter, &true);
    client.dispute(&disputer, &id);
    assert_eq!(token.balance(&asserter), 900);
    assert_eq!(token.balance(&disputer), 900);

    client.resolve(&resolvers.get(0).unwrap(), &id, &false);
    client.resolve(&resolvers.get(1).unwrap(), &id, &false);

    assert_eq!(token.balance(&disputer), 1_100);
    assert_eq!(token.balance(&asserter), 900);
}

#[test]
fn test_cannot_initialize_with_even_resolver_count() {
    let env = Env::default();
    env.mock_all_auths();

    let (_token_admin, token_id, _token, _resolvers) = setup(&env);
    let contract_id = env.register(Tholos, ());
    let client = TholosClient::new(&env, &contract_id);

    let admin = Address::generate(&env);
    let even_resolvers = Vec::from_array(&env, [Address::generate(&env), Address::generate(&env)]);

    let result = client.try_initialize(&admin, &token_id, &100, &3600, &even_resolvers);
    assert!(result.is_err());
}

#[test]
fn test_cannot_initialize_twice() {
    let env = Env::default();
    env.mock_all_auths();

    let (_token_admin, token_id, _token, resolvers) = setup(&env);
    let contract_id = env.register(Tholos, ());
    let client = TholosClient::new(&env, &contract_id);

    let admin = Address::generate(&env);
    client.initialize(&admin, &token_id, &100, &3600, &resolvers);

    let result = client.try_initialize(&admin, &token_id, &100, &3600, &resolvers);
    assert_eq!(result, Err(Ok(Error::AlreadyInitialized)));
}

#[test]
fn test_cannot_finalize_before_window_closes() {
    let env = Env::default();
    env.mock_all_auths();

    let (_token_admin, token_id, _token, resolvers) = setup(&env);
    let token_asset_client = token::StellarAssetClient::new(&env, &token_id);
    let contract_id = env.register(Tholos, ());
    let client = TholosClient::new(&env, &contract_id);

    let admin = Address::generate(&env);
    let asserter = Address::generate(&env);
    token_asset_client.mint(&asserter, &1_000);
    client.initialize(&admin, &token_id, &100, &3600, &resolvers);

    let id = client.assert_outcome(&asserter, &true);

    let result = client.try_finalize(&id);
    assert_eq!(result, Err(Ok(Error::ChallengeWindowOpen)));
}

#[test]
fn test_cannot_dispute_after_window_closes() {
    let env = Env::default();
    env.mock_all_auths();

    let (_token_admin, token_id, _token, resolvers) = setup(&env);
    let token_asset_client = token::StellarAssetClient::new(&env, &token_id);
    let contract_id = env.register(Tholos, ());
    let client = TholosClient::new(&env, &contract_id);

    let admin = Address::generate(&env);
    let asserter = Address::generate(&env);
    let disputer = Address::generate(&env);
    token_asset_client.mint(&asserter, &1_000);
    token_asset_client.mint(&disputer, &1_000);
    client.initialize(&admin, &token_id, &100, &3600, &resolvers);

    let id = client.assert_outcome(&asserter, &true);
    env.ledger().with_mut(|l| l.timestamp += 3601);

    let result = client.try_dispute(&disputer, &id);
    assert_eq!(result, Err(Ok(Error::ChallengeWindowClosed)));
}

#[test]
fn test_cannot_dispute_an_already_disputed_assertion() {
    let env = Env::default();
    env.mock_all_auths();

    let (_token_admin, token_id, _token, resolvers) = setup(&env);
    let token_asset_client = token::StellarAssetClient::new(&env, &token_id);
    let contract_id = env.register(Tholos, ());
    let client = TholosClient::new(&env, &contract_id);

    let admin = Address::generate(&env);
    let asserter = Address::generate(&env);
    let disputer = Address::generate(&env);
    let second_disputer = Address::generate(&env);
    token_asset_client.mint(&asserter, &1_000);
    token_asset_client.mint(&disputer, &1_000);
    token_asset_client.mint(&second_disputer, &1_000);
    client.initialize(&admin, &token_id, &100, &3600, &resolvers);

    let id = client.assert_outcome(&asserter, &true);
    client.dispute(&disputer, &id);

    let result = client.try_dispute(&second_disputer, &id);
    assert_eq!(result, Err(Ok(Error::NotPending)));
}

#[test]
fn test_non_resolver_cannot_vote() {
    let env = Env::default();
    env.mock_all_auths();

    let (_token_admin, token_id, _token, resolvers) = setup(&env);
    let token_asset_client = token::StellarAssetClient::new(&env, &token_id);
    let contract_id = env.register(Tholos, ());
    let client = TholosClient::new(&env, &contract_id);

    let admin = Address::generate(&env);
    let asserter = Address::generate(&env);
    let disputer = Address::generate(&env);
    let outsider = Address::generate(&env);
    token_asset_client.mint(&asserter, &1_000);
    token_asset_client.mint(&disputer, &1_000);
    client.initialize(&admin, &token_id, &100, &3600, &resolvers);

    let id = client.assert_outcome(&asserter, &true);
    client.dispute(&disputer, &id);

    let result = client.try_resolve(&outsider, &id, &true);
    assert_eq!(result, Err(Ok(Error::NotAResolver)));
}

#[test]
fn test_resolver_cannot_vote_twice() {
    let env = Env::default();
    env.mock_all_auths();

    let (_token_admin, token_id, _token, resolvers) = setup(&env);
    let token_asset_client = token::StellarAssetClient::new(&env, &token_id);
    let contract_id = env.register(Tholos, ());
    let client = TholosClient::new(&env, &contract_id);

    let admin = Address::generate(&env);
    let asserter = Address::generate(&env);
    let disputer = Address::generate(&env);
    token_asset_client.mint(&asserter, &1_000);
    token_asset_client.mint(&disputer, &1_000);
    client.initialize(&admin, &token_id, &100, &3600, &resolvers);

    let id = client.assert_outcome(&asserter, &true);
    client.dispute(&disputer, &id);

    let resolver = resolvers.get(0).unwrap();
    client.resolve(&resolver, &id, &true);

    let result = client.try_resolve(&resolver, &id, &true);
    assert_eq!(result, Err(Ok(Error::AlreadyVoted)));
}

#[test]
fn test_cannot_resolve_a_non_disputed_assertion() {
    let env = Env::default();
    env.mock_all_auths();

    let (_token_admin, token_id, _token, resolvers) = setup(&env);
    let token_asset_client = token::StellarAssetClient::new(&env, &token_id);
    let contract_id = env.register(Tholos, ());
    let client = TholosClient::new(&env, &contract_id);

    let admin = Address::generate(&env);
    let asserter = Address::generate(&env);
    token_asset_client.mint(&asserter, &1_000);
    client.initialize(&admin, &token_id, &100, &3600, &resolvers);

    let id = client.assert_outcome(&asserter, &true);

    let result = client.try_resolve(&resolvers.get(0).unwrap(), &id, &true);
    assert_eq!(result, Err(Ok(Error::NotDisputed)));
}

#[test]
fn test_split_resolver_vote_does_not_finalize() {
    let env = Env::default();
    env.mock_all_auths();

    let (_token_admin, token_id, token, resolvers) = setup(&env);
    let token_asset_client = token::StellarAssetClient::new(&env, &token_id);
    let contract_id = env.register(Tholos, ());
    let client = TholosClient::new(&env, &contract_id);

    let admin = Address::generate(&env);
    let asserter = Address::generate(&env);
    let disputer = Address::generate(&env);
    token_asset_client.mint(&asserter, &1_000);
    token_asset_client.mint(&disputer, &1_000);
    client.initialize(&admin, &token_id, &100, &3600, &resolvers);

    let id = client.assert_outcome(&asserter, &true);
    client.dispute(&disputer, &id);

    let outcome = client.resolve(&resolvers.get(0).unwrap(), &id, &true);
    assert_eq!(outcome, None);
    assert_eq!(token.balance(&asserter), 900);
    assert_eq!(token.balance(&disputer), 900);
}

#[test]
fn test_operations_on_unknown_assertion_fail() {
    let env = Env::default();
    env.mock_all_auths();

    let (_token_admin, token_id, _token, resolvers) = setup(&env);
    let contract_id = env.register(Tholos, ());
    let client = TholosClient::new(&env, &contract_id);

    let admin = Address::generate(&env);
    let disputer = Address::generate(&env);
    client.initialize(&admin, &token_id, &100, &3600, &resolvers);

    assert_eq!(
        client.try_dispute(&disputer, &42),
        Err(Ok(Error::AssertionNotFound))
    );
    assert_eq!(client.try_finalize(&42), Err(Ok(Error::AssertionNotFound)));
    assert_eq!(
        client.try_resolve(&resolvers.get(0).unwrap(), &42, &true),
        Err(Ok(Error::AssertionNotFound))
    );
    assert_eq!(
        client.try_get_assertion_state(&42),
        Err(Ok(Error::AssertionNotFound))
    );
}

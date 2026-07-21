#![cfg(test)]

use super::*;
use soroban_sdk::testutils::storage::Persistent as _;
use soroban_sdk::testutils::{Address as _, Ledger};

const DEFAULT_BOND: i128 = 100;
const DEFAULT_WINDOW: u64 = 3600;
const DEFAULT_MINT: i128 = 1_000;

/// A registered but uninitialized token and resolver committee, for the
/// handful of tests that need to call `initialize` themselves (to test bad
/// init parameters, or that it can't be called twice).
fn setup(env: &Env) -> (Address, Vec<Address>) {
    let token_admin = Address::generate(env);
    let token_id = env
        .register_stellar_asset_contract_v2(token_admin)
        .address();
    let resolvers = Vec::from_array(
        env,
        [
            Address::generate(env),
            Address::generate(env),
            Address::generate(env),
        ],
    );
    (token_id, resolvers)
}

/// A ready-to-use, already-initialized Tholos instance with a 3-member
/// resolver committee and its backing token (bond 100, window 3600), used by
/// most tests. Tests that need an *uninitialized* contract, or non-default
/// init parameters, use `setup` directly instead.
struct Fixture {
    env: Env,
    client: TholosClient<'static>,
    token: token::Client<'static>,
    token_id: Address,
    resolvers: Vec<Address>,
}

impl Fixture {
    fn new() -> Self {
        let env = Env::default();
        env.mock_all_auths();

        let (token_id, resolvers) = setup(&env);
        let token = token::Client::new(&env, &token_id);

        let contract_id = env.register(Tholos, ());
        let client = TholosClient::new(&env, &contract_id);

        let admin = Address::generate(&env);
        client.initialize(
            &admin,
            &token_id,
            &DEFAULT_BOND,
            &DEFAULT_WINDOW,
            &resolvers,
            &0u32,
        );

        Fixture {
            env,
            client,
            token,
            token_id,
            resolvers,
        }
    }

    fn generate(&self) -> Address {
        Address::generate(&self.env)
    }

    /// Generates a fresh address and mints it the default test balance.
    fn funded_address(&self) -> Address {
        let addr = self.generate();
        self.mint(&addr, DEFAULT_MINT);
        addr
    }

    fn mint(&self, addr: &Address, amount: i128) {
        token::StellarAssetClient::new(&self.env, &self.token_id).mint(addr, &amount);
    }

    fn advance_past_window(&self) {
        self.env
            .ledger()
            .with_mut(|l| l.timestamp += DEFAULT_WINDOW + 1);
    }
}

#[test]
fn test_uncontested_assertion_finalizes() {
    let f = Fixture::new();
    let asserter = f.funded_address();
    let caller = f.generate();

    let id = f.client.assert_outcome(&asserter, &true);
    assert_eq!(f.token.balance(&asserter), 900);

    f.advance_past_window();

    // Zero reward bps (the default): full bond back to asserter, caller gets
    // nothing. Auth is still required unconditionally so the recorded
    // finalizer is always a verified address.
    let outcome = f.client.finalize(&caller, &id);
    assert!(outcome);
    assert_eq!(f.token.balance(&asserter), 1_000);
    assert_eq!(f.token.balance(&caller), 0);

    // Finalizer is always recorded now — caller required auth unconditionally.
    let state = f.client.get_assertion_state(&id);
    assert_eq!(state.finalizer, Some(caller));
}

#[test]
fn test_assertion_storage_ttl_is_extended_on_every_write() {
    let f = Fixture::new();
    let asserter = f.funded_address();
    let disputer = f.funded_address();

    let ttl_of = |id: u64| {
        f.env.as_contract(&f.client.address, || {
            f.env
                .storage()
                .persistent()
                .get_ttl(&DataKey::Assertion(id))
        })
    };

    let id = f.client.assert_outcome(&asserter, &true);
    assert_eq!(ttl_of(id), ASSERTION_BUMP_AMOUNT);

    // Advance close to expiry, then confirm disputing (a write) bumps the
    // TTL back up rather than leaving the entry to lapse.
    f.env
        .ledger()
        .with_mut(|l| l.sequence_number += ASSERTION_BUMP_AMOUNT - 10);
    f.client.dispute(&disputer, &id);
    assert_eq!(ttl_of(id), ASSERTION_BUMP_AMOUNT);
}

#[test]
fn test_disputed_assertion_pays_winner() {
    let f = Fixture::new();
    let asserter = f.funded_address();
    let disputer = f.funded_address();

    let id = f.client.assert_outcome(&asserter, &true);
    f.client.dispute(&disputer, &id);
    assert_eq!(f.token.balance(&asserter), 900);
    assert_eq!(f.token.balance(&disputer), 900);

    f.client.resolve(&f.resolvers.get(0).unwrap(), &id, &false);
    f.client.resolve(&f.resolvers.get(1).unwrap(), &id, &false);

    assert_eq!(f.token.balance(&disputer), 1_100);
    assert_eq!(f.token.balance(&asserter), 900);
}

#[test]
fn test_cannot_initialize_with_even_resolver_count() {
    let env = Env::default();
    env.mock_all_auths();

    let (token_id, _resolvers) = setup(&env);
    let contract_id = env.register(Tholos, ());
    let client = TholosClient::new(&env, &contract_id);

    let admin = Address::generate(&env);
    let even_resolvers = Vec::from_array(&env, [Address::generate(&env), Address::generate(&env)]);

    let result = client.try_initialize(
        &admin,
        &token_id,
        &DEFAULT_BOND,
        &DEFAULT_WINDOW,
        &even_resolvers,
        &0u32,
    );
    assert!(result.is_err());
}

#[test]
fn test_cannot_initialize_with_too_many_resolvers() {
    let env = Env::default();
    env.mock_all_auths();

    let (token_id, _resolvers) = setup(&env);
    let contract_id = env.register(Tholos, ());
    let client = TholosClient::new(&env, &contract_id);

    let admin = Address::generate(&env);
    // +2, not +1: must stay odd (MAX_RESOLVERS is odd) so this isolates the
    // TooManyResolvers check rather than tripping InvalidResolverCount first.
    let mut too_many = Vec::new(&env);
    for _ in 0..(MAX_RESOLVERS + 2) {
        too_many.push_back(Address::generate(&env));
    }

    let result = client.try_initialize(
        &admin,
        &token_id,
        &DEFAULT_BOND,
        &DEFAULT_WINDOW,
        &too_many,
        &0u32,
    );
    assert_eq!(result, Err(Ok(Error::TooManyResolvers)));
}

#[test]
fn test_cannot_initialize_with_zero_bond_amount() {
    let env = Env::default();
    env.mock_all_auths();

    let (token_id, resolvers) = setup(&env);
    let contract_id = env.register(Tholos, ());
    let client = TholosClient::new(&env, &contract_id);

    let admin = Address::generate(&env);
    let result = client.try_initialize(&admin, &token_id, &0, &DEFAULT_WINDOW, &resolvers, &0u32);
    assert_eq!(result, Err(Ok(Error::InvalidBondAmount)));
}

#[test]
fn test_cannot_initialize_with_negative_bond_amount() {
    let env = Env::default();
    env.mock_all_auths();

    let (token_id, resolvers) = setup(&env);
    let contract_id = env.register(Tholos, ());
    let client = TholosClient::new(&env, &contract_id);

    let admin = Address::generate(&env);
    let result = client.try_initialize(&admin, &token_id, &-1, &DEFAULT_WINDOW, &resolvers, &0u32);
    assert_eq!(result, Err(Ok(Error::InvalidBondAmount)));
}

#[test]
fn test_cannot_initialize_with_zero_challenge_window() {
    let env = Env::default();
    env.mock_all_auths();

    let (token_id, resolvers) = setup(&env);
    let contract_id = env.register(Tholos, ());
    let client = TholosClient::new(&env, &contract_id);

    let admin = Address::generate(&env);
    let result = client.try_initialize(&admin, &token_id, &DEFAULT_BOND, &0, &resolvers, &0u32);
    assert_eq!(result, Err(Ok(Error::InvalidChallengeWindow)));
}

#[test]
fn test_cannot_initialize_with_challenge_window_too_large() {
    let env = Env::default();
    env.mock_all_auths();

    let (token_id, resolvers) = setup(&env);
    let contract_id = env.register(Tholos, ());
    let client = TholosClient::new(&env, &contract_id);

    let admin = Address::generate(&env);
    let result = client.try_initialize(
        &admin,
        &token_id,
        &DEFAULT_BOND,
        &(MAX_CHALLENGE_WINDOW_SECS + 1),
        &resolvers,
        &0u32,
    );
    assert_eq!(result, Err(Ok(Error::InvalidChallengeWindow)));
}

#[test]
fn test_cannot_initialize_twice() {
    let f = Fixture::new();

    let admin = f.generate();
    let result = f.client.try_initialize(
        &admin,
        &f.token_id,
        &DEFAULT_BOND,
        &DEFAULT_WINDOW,
        &f.resolvers,
        &0u32,
    );
    assert_eq!(result, Err(Ok(Error::AlreadyInitialized)));
}

#[test]
fn test_cannot_finalize_before_window_closes() {
    let f = Fixture::new();
    let asserter = f.funded_address();
    let caller = f.generate();

    let id = f.client.assert_outcome(&asserter, &true);

    let result = f.client.try_finalize(&caller, &id);
    assert_eq!(result, Err(Ok(Error::ChallengeWindowOpen)));
}

#[test]
fn test_cannot_dispute_after_window_closes() {
    let f = Fixture::new();
    let asserter = f.funded_address();
    let disputer = f.funded_address();

    let id = f.client.assert_outcome(&asserter, &true);
    f.advance_past_window();

    let result = f.client.try_dispute(&disputer, &id);
    assert_eq!(result, Err(Ok(Error::ChallengeWindowClosed)));
}

#[test]
fn test_cannot_dispute_an_already_disputed_assertion() {
    let f = Fixture::new();
    let asserter = f.funded_address();
    let disputer = f.funded_address();
    let second_disputer = f.funded_address();

    let id = f.client.assert_outcome(&asserter, &true);
    f.client.dispute(&disputer, &id);

    let result = f.client.try_dispute(&second_disputer, &id);
    assert_eq!(result, Err(Ok(Error::NotPending)));
}

#[test]
fn test_non_resolver_cannot_vote() {
    let f = Fixture::new();
    let asserter = f.funded_address();
    let disputer = f.funded_address();
    let outsider = f.generate();

    let id = f.client.assert_outcome(&asserter, &true);
    f.client.dispute(&disputer, &id);

    let result = f.client.try_resolve(&outsider, &id, &true);
    assert_eq!(result, Err(Ok(Error::NotAResolver)));
}

#[test]
fn test_resolver_cannot_vote_twice() {
    let f = Fixture::new();
    let asserter = f.funded_address();
    let disputer = f.funded_address();

    let id = f.client.assert_outcome(&asserter, &true);
    f.client.dispute(&disputer, &id);

    let resolver = f.resolvers.get(0).unwrap();
    f.client.resolve(&resolver, &id, &true);

    let result = f.client.try_resolve(&resolver, &id, &true);
    assert_eq!(result, Err(Ok(Error::AlreadyVoted)));
}

#[test]
fn test_cannot_resolve_a_non_disputed_assertion() {
    let f = Fixture::new();
    let asserter = f.funded_address();

    let id = f.client.assert_outcome(&asserter, &true);

    let result = f
        .client
        .try_resolve(&f.resolvers.get(0).unwrap(), &id, &true);
    assert_eq!(result, Err(Ok(Error::NotDisputed)));
}

#[test]
fn test_split_resolver_vote_does_not_finalize() {
    let f = Fixture::new();
    let asserter = f.funded_address();
    let disputer = f.funded_address();

    let id = f.client.assert_outcome(&asserter, &true);
    f.client.dispute(&disputer, &id);

    let outcome = f.client.resolve(&f.resolvers.get(0).unwrap(), &id, &true);
    assert_eq!(outcome, None);
    assert_eq!(f.token.balance(&asserter), 900);
    assert_eq!(f.token.balance(&disputer), 900);
}

#[test]
fn test_admin_can_update_resolvers() {
    let f = Fixture::new();
    let asserter = f.funded_address();
    let disputer = f.funded_address();

    let new_resolvers = Vec::from_array(&f.env, [f.generate(), f.generate(), f.generate()]);
    f.client.update_resolvers(&new_resolvers);

    // The old committee can no longer vote.
    let id = f.client.assert_outcome(&asserter, &true);
    f.client.dispute(&disputer, &id);
    let result = f
        .client
        .try_resolve(&f.resolvers.get(0).unwrap(), &id, &true);
    assert_eq!(result, Err(Ok(Error::NotAResolver)));

    // The new committee can.
    f.client
        .resolve(&new_resolvers.get(0).unwrap(), &id, &false);
    f.client
        .resolve(&new_resolvers.get(1).unwrap(), &id, &false);
    assert_eq!(f.token.balance(&disputer), 1_100);
}

#[test]
fn test_resolvers_updated_mid_dispute_do_not_affect_it() {
    let f = Fixture::new();
    let asserter = f.funded_address();
    let disputer = f.funded_address();

    // The dispute is opened, snapshotting the original committee, before the
    // committee changes.
    let id = f.client.assert_outcome(&asserter, &true);
    f.client.dispute(&disputer, &id);

    let new_resolvers = Vec::from_array(&f.env, [f.generate(), f.generate(), f.generate()]);
    f.client.update_resolvers(&new_resolvers);

    // A member of the new (current) committee cannot vote on this dispute:
    // it was snapshotted to the old committee before they joined.
    assert_eq!(
        f.client
            .try_resolve(&new_resolvers.get(0).unwrap(), &id, &true),
        Err(Ok(Error::NotAResolver))
    );

    // The original committee, though no longer the live committee, can
    // still decide this dispute, since it was snapshotted at dispute time.
    f.client.resolve(&f.resolvers.get(0).unwrap(), &id, &false);
    f.client.resolve(&f.resolvers.get(1).unwrap(), &id, &false);
    assert_eq!(f.token.balance(&disputer), 1_100);
}

#[test]
fn test_paused_blocks_assert_dispute_and_resolve_but_not_finalize() {
    let f = Fixture::new();
    let asserter = f.funded_address();
    let disputer = f.funded_address();

    // An assertion posted before the pause can still finalize normally.
    let pending_id = f.client.assert_outcome(&asserter, &true);

    f.client.set_paused(&true);

    assert_eq!(
        f.client.try_assert_outcome(&asserter, &true),
        Err(Ok(Error::Paused))
    );
    assert_eq!(
        f.client.try_dispute(&disputer, &pending_id),
        Err(Ok(Error::Paused))
    );

    f.advance_past_window();
    let outcome = f.client.finalize(&asserter, &pending_id);
    assert!(outcome);
    assert_eq!(f.token.balance(&asserter), 1_000);

    f.client.set_paused(&false);
    let id = f.client.assert_outcome(&asserter, &true);
    f.client.dispute(&disputer, &id);
    assert_eq!(
        f.client
            .try_resolve(&f.resolvers.get(0).unwrap(), &id, &true),
        Ok(Ok(None))
    );
}

#[test]
fn test_cannot_pause_before_initialization() {
    let env = Env::default();
    env.mock_all_auths();

    let contract_id = env.register(Tholos, ());
    let client = TholosClient::new(&env, &contract_id);

    assert_eq!(client.try_set_paused(&true), Err(Ok(Error::NotInitialized)));
}

#[test]
fn test_cannot_update_resolvers_to_even_count() {
    let f = Fixture::new();

    let even_resolvers = Vec::from_array(&f.env, [f.generate(), f.generate()]);
    let result = f.client.try_update_resolvers(&even_resolvers);
    assert_eq!(result, Err(Ok(Error::InvalidResolverCount)));
}

#[test]
fn test_cannot_update_resolvers_to_too_many() {
    let f = Fixture::new();

    let mut too_many = Vec::new(&f.env);
    for _ in 0..(MAX_RESOLVERS + 2) {
        too_many.push_back(f.generate());
    }

    let result = f.client.try_update_resolvers(&too_many);
    assert_eq!(result, Err(Ok(Error::TooManyResolvers)));
}

#[test]
fn test_cannot_update_resolvers_before_initialization() {
    let env = Env::default();
    env.mock_all_auths();

    let contract_id = env.register(Tholos, ());
    let client = TholosClient::new(&env, &contract_id);

    let resolvers = Vec::from_array(
        &env,
        [
            Address::generate(&env),
            Address::generate(&env),
            Address::generate(&env),
        ],
    );
    let result = client.try_update_resolvers(&resolvers);
    assert_eq!(result, Err(Ok(Error::NotInitialized)));
}

#[test]
fn test_cannot_initialize_with_duplicate_resolvers() {
    let env = Env::default();
    env.mock_all_auths();

    let (token_id, _resolvers) = setup(&env);
    let contract_id = env.register(Tholos, ());
    let client = TholosClient::new(&env, &contract_id);

    let admin = Address::generate(&env);
    // The same address twice, plus a third: odd length and within
    // MAX_RESOLVERS, so this isolates the duplicate check.
    let a = Address::generate(&env);
    let b = Address::generate(&env);
    let duplicated = Vec::from_array(&env, [a.clone(), a.clone(), b]);

    let result = client.try_initialize(
        &admin,
        &token_id,
        &DEFAULT_BOND,
        &DEFAULT_WINDOW,
        &duplicated,
        &0u32,
    );
    assert_eq!(result, Err(Ok(Error::DuplicateResolvers)));
}

#[test]
fn test_initialize_accepts_distinct_committee() {
    // Sanity check that the duplicate rejection doesn't reject the happy path:
    // a fully distinct committee still initializes successfully.
    let env = Env::default();
    env.mock_all_auths();

    let (token_id, resolvers) = setup(&env);
    let contract_id = env.register(Tholos, ());
    let client = TholosClient::new(&env, &contract_id);

    let admin = Address::generate(&env);
    let result = client.try_initialize(
        &admin,
        &token_id,
        &DEFAULT_BOND,
        &DEFAULT_WINDOW,
        &resolvers,
        &0u32,
    );
    assert_eq!(result, Ok(Ok(())));
}

#[test]
fn test_initialize_rejects_duplicate_at_end_of_vector() {
    // A duplicate at the very end of an otherwise-distinct, odd-length,
    // within-bounds committee, to prove the inner scan runs to the end.
    let env = Env::default();
    env.mock_all_auths();

    let (token_id, _resolvers) = setup(&env);
    let contract_id = env.register(Tholos, ());
    let client = TholosClient::new(&env, &contract_id);

    let admin = Address::generate(&env);
    let d = Address::generate(&env);
    let resolvers = Vec::from_array(
        &env,
        [
            Address::generate(&env),
            Address::generate(&env),
            Address::generate(&env),
            d.clone(),
            d.clone(),
        ],
    );

    let result = client.try_initialize(
        &admin,
        &token_id,
        &DEFAULT_BOND,
        &DEFAULT_WINDOW,
        &resolvers,
        &0u32,
    );
    assert_eq!(result, Err(Ok(Error::DuplicateResolvers)));
}

#[test]
fn test_initialize_reports_invalid_count_before_duplicates() {
    // `[A, A]` is both even-length and duplicate-heavy. The even-length check
    // runs first, so InvalidResolverCount is reported, not DuplicateResolvers.
    // Documents the precedence of the cheaper check over the O(n²) scan.
    let env = Env::default();
    env.mock_all_auths();

    let (token_id, _resolvers) = setup(&env);
    let contract_id = env.register(Tholos, ());
    let client = TholosClient::new(&env, &contract_id);

    let admin = Address::generate(&env);
    let a = Address::generate(&env);
    let even_and_duplicated = Vec::from_array(&env, [a.clone(), a.clone()]);

    let result = client.try_initialize(
        &admin,
        &token_id,
        &DEFAULT_BOND,
        &DEFAULT_WINDOW,
        &even_and_duplicated,
        &0u32,
    );
    assert_eq!(result, Err(Ok(Error::InvalidResolverCount)));
}

#[test]
fn test_cannot_update_resolvers_to_duplicates() {
    let f = Fixture::new();

    let a = f.generate();
    let d = f.generate();
    let duplicated = Vec::from_array(&f.env, [a.clone(), a.clone(), d]);

    let result = f.client.try_update_resolvers(&duplicated);
    assert_eq!(result, Err(Ok(Error::DuplicateResolvers)));

    // The rejected update must not have overwritten the stored committee: a
    // member of the original committee can still be looked up as a resolver.
    let asserter = f.funded_address();
    let disputer = f.funded_address();
    let id = f.client.assert_outcome(&asserter, &true);
    f.client.dispute(&disputer, &id);
    f.client.resolve(&f.resolvers.get(0).unwrap(), &id, &false);
    f.client.resolve(&f.resolvers.get(1).unwrap(), &id, &false);
    assert_eq!(f.token.balance(&disputer), 1_100);
}

#[test]
fn test_operations_on_unknown_assertion_fail() {
    let f = Fixture::new();
    let disputer = f.generate();

    assert_eq!(
        f.client.try_dispute(&disputer, &42),
        Err(Ok(Error::AssertionNotFound))
    );
    assert_eq!(
        f.client.try_finalize(&disputer, &42),
        Err(Ok(Error::AssertionNotFound))
    );
    assert_eq!(
        f.client
            .try_resolve(&f.resolvers.get(0).unwrap(), &42, &true),
        Err(Ok(Error::AssertionNotFound))
    );
    assert_eq!(
        f.client.try_get_assertion_state(&42),
        Err(Ok(Error::AssertionNotFound))
    );
}

// ---------------------------------------------------------------------------
// Finalize reward tests
// ---------------------------------------------------------------------------

/// Helper: build a Tholos instance configured with the given reward bps.
fn fixture_with_reward(bps: u32) -> (Fixture, Address) {
    let env = Env::default();
    env.mock_all_auths();

    let (token_id, resolvers) = setup(&env);
    let token = token::Client::new(&env, &token_id);
    let contract_id = env.register(Tholos, ());
    let client = TholosClient::new(&env, &contract_id);
    let admin = Address::generate(&env);
    client.initialize(
        &admin,
        &token_id,
        &DEFAULT_BOND,
        &DEFAULT_WINDOW,
        &resolvers,
        &bps,
    );
    let f = Fixture {
        env,
        client,
        token,
        token_id,
        resolvers,
    };
    let contract_addr = f.client.address.clone();
    (f, contract_addr)
}

#[test]
fn test_finalize_with_reward_pays_caller_and_asserter() {
    // 500 bps = 5 % of bond (100) = 5 tokens to caller; 95 back to asserter.
    let (f, _) = fixture_with_reward(500);
    let asserter = f.funded_address();
    let caller = f.generate(); // no tokens yet

    let id = f.client.assert_outcome(&asserter, &true);
    assert_eq!(f.token.balance(&asserter), 900); // bond deducted

    f.advance_past_window();
    let outcome = f.client.finalize(&caller, &id);

    assert!(outcome);
    assert_eq!(f.token.balance(&caller), 5); // 500 bps of 100
    assert_eq!(f.token.balance(&asserter), 995); // 900 + 95

    // State reflects finalizer.
    let state = f.client.get_assertion_state(&id);
    assert_eq!(state.finalizer, Some(caller));
    assert_eq!(state.status, Status::Resolved);
}

#[test]
fn test_finalize_zero_reward_full_bond_returned() {
    // Explicit zero bps: full bond back to asserter, caller gets nothing.
    // Auth is still required; the finalizer is recorded.
    let (f, _) = fixture_with_reward(0);
    let asserter = f.funded_address();
    let caller = f.generate();

    let id = f.client.assert_outcome(&asserter, &true);
    f.advance_past_window();
    f.client.finalize(&caller, &id);

    assert_eq!(f.token.balance(&asserter), 1_000);
    // finalizer is now always recorded — caller must authorize unconditionally.
    let state = f.client.get_assertion_state(&id);
    assert_eq!(state.finalizer, Some(caller));
}

#[test]
fn test_finalize_max_reward_bps() {
    // 1000 bps = 10 % of bond (100) = 10 tokens to caller; 90 to asserter.
    let (f, _) = fixture_with_reward(MAX_FINALIZE_REWARD_BPS);
    let asserter = f.funded_address();
    let caller = f.generate();

    let id = f.client.assert_outcome(&asserter, &true);
    f.advance_past_window();
    f.client.finalize(&caller, &id);

    assert_eq!(f.token.balance(&caller), 10);
    assert_eq!(f.token.balance(&asserter), 990);
}

#[test]
fn test_cannot_initialize_with_reward_bps_over_max() {
    let env = Env::default();
    env.mock_all_auths();

    let (token_id, resolvers) = setup(&env);
    let contract_id = env.register(Tholos, ());
    let client = TholosClient::new(&env, &contract_id);
    let admin = Address::generate(&env);

    let result = client.try_initialize(
        &admin,
        &token_id,
        &DEFAULT_BOND,
        &DEFAULT_WINDOW,
        &resolvers,
        &(MAX_FINALIZE_REWARD_BPS + 1),
    );
    assert_eq!(result, Err(Ok(Error::InvalidFinalizeReward)));
}

#[test]
fn test_finalize_requires_auth_when_reward_bps_is_zero() {
    // The core fix for the flagged review comment: finalize requires
    // caller.require_auth() unconditionally, even when finalize_reward_bps is
    // 0 (no reward configured). Verify via env.auths() that the auth was
    // actually invoked for the caller's address.
    let (f, _) = fixture_with_reward(0);
    let asserter = f.funded_address();
    let caller = f.generate();

    let id = f.client.assert_outcome(&asserter, &true);
    f.advance_past_window();
    f.client.finalize(&caller, &id);

    // env.auths() returns every require_auth invocation that occurred during
    // the last contract call. The caller's auth must appear in this list,
    // proving it was checked even with zero reward bps.
    let auths = f.env.auths();
    let caller_was_authed = auths.iter().any(|(addr, _)| *addr == caller);
    assert!(
        caller_was_authed,
        "caller's require_auth was not invoked during finalize with 0 bps"
    );

    // Confirm finalizer is recorded (not None) since auth was verified.
    let state = f.client.get_assertion_state(&id);
    assert_eq!(state.finalizer, Some(caller));
}

#[test]
fn test_finalize_with_reward_works_while_paused() {
    // Finalize is deliberately exempt from the pause; reward payout must also
    // work when the contract is paused.
    let (f, _) = fixture_with_reward(200); // 2 % = 2 tokens
    let asserter = f.funded_address();
    let caller = f.generate();

    let id = f.client.assert_outcome(&asserter, &true);
    f.client.set_paused(&true);
    f.advance_past_window();

    let outcome = f.client.finalize(&caller, &id);
    assert!(outcome);
    assert_eq!(f.token.balance(&caller), 2);
    assert_eq!(f.token.balance(&asserter), 998);
}

/// A minimal token that reenters a Tholos call from inside its own
/// `transfer`, before doing its own balance bookkeeping. Models a malicious
/// or merely non-standard (e.g. hook-bearing) SEP-41 token, to prove state is
/// written before the external transfer rather than after it.
///
/// The evil-token tests initialize Tholos with `finalize_reward_bps = 0`, so
/// `finalize` pays no reward in this context. Because `finalize` requires
/// `caller.require_auth()` unconditionally, a reentrant token attempting to
/// call `finalize` from inside its own `transfer` is rejected by Soroban's
/// auth model (the same first-layer protection that applies to `assert_outcome`,
/// `dispute`, and `resolve`). The state-before-transfer ordering is a second
/// layer of defense in case a colluding, pre-authorized signer ever got one
/// through.
mod evil_token {
    use soroban_sdk::{contract, contractimpl, contracttype, Address, Env, Map};

    /// Which Tholos call to attempt reentrantly, and with what arguments.
    /// `transfer` disarms this (sets it back to `None`) before acting on it,
    /// so a reentrant call that itself triggers another `transfer` (e.g. a
    /// successful reentrant `assert_outcome`) doesn't recurse indefinitely.
    #[contracttype]
    #[derive(Clone)]
    pub enum Reentry {
        None,
        AssertOutcome(Address, bool),
        Dispute(Address, u64),
        Resolve(Address, u64, bool),
        Finalize(Address, u64),
    }

    /// Storage keys for `EvilToken`, mirroring the `DataKey` pattern used by
    /// the real Tholos contract instead of ad hoc `symbol_short!` strings.
    #[contracttype]
    pub enum DataKey {
        Tholos,
        Reentry,
        Balances,
    }

    #[contract]
    pub struct EvilToken;

    #[contractimpl]
    impl EvilToken {
        pub fn configure(env: Env, tholos_id: Address, reentry: Reentry) {
            env.storage().instance().set(&DataKey::Tholos, &tholos_id);
            env.storage().instance().set(&DataKey::Reentry, &reentry);
        }

        pub fn credit(env: Env, addr: Address, amount: i128) {
            let mut balances = Self::balances(&env);
            let current = balances.get(addr.clone()).unwrap_or(0);
            balances.set(addr, current + amount);
            env.storage().instance().set(&DataKey::Balances, &balances);
        }

        pub fn balance(env: Env, addr: Address) -> i128 {
            Self::balances(&env).get(addr).unwrap_or(0)
        }

        pub fn transfer(env: Env, from: Address, to: Address, amount: i128) {
            if let Some(tholos_id) = env.storage().instance().get::<_, Address>(&DataKey::Tholos) {
                let reentry: Reentry = env
                    .storage()
                    .instance()
                    .get(&DataKey::Reentry)
                    .unwrap_or(Reentry::None);
                env.storage()
                    .instance()
                    .set(&DataKey::Reentry, &Reentry::None);

                let client = super::TholosClient::new(&env, &tholos_id);
                // A well-behaved caller would fail cleanly here if Tholos has
                // already written its state; that's exactly what these tests
                // verify. Ignore the result either way.
                match reentry {
                    Reentry::None => {}
                    Reentry::AssertOutcome(asserter, outcome) => {
                        let _ = client.try_assert_outcome(&asserter, &outcome);
                    }
                    Reentry::Dispute(disputer, id) => {
                        let _ = client.try_dispute(&disputer, &id);
                    }
                    Reentry::Resolve(resolver, id, agrees_with_asserter) => {
                        let _ = client.try_resolve(&resolver, &id, &agrees_with_asserter);
                    }
                    Reentry::Finalize(caller, id) => {
                        let _ = client.try_finalize(&caller, &id);
                    }
                }
            }

            let mut balances = Self::balances(&env);
            let from_bal = balances.get(from.clone()).unwrap_or(0);
            let to_bal = balances.get(to.clone()).unwrap_or(0);
            balances.set(from, from_bal - amount);
            balances.set(to, to_bal + amount);
            env.storage().instance().set(&DataKey::Balances, &balances);
        }

        fn balances(env: &Env) -> Map<Address, i128> {
            env.storage()
                .instance()
                .get(&DataKey::Balances)
                .unwrap_or(Map::new(env))
        }
    }
}

/// Shared setup for the reentrancy tests below: a Tholos instance backed by
/// `EvilToken` instead of a real SAC, so each test can arm a specific
/// reentrant call and verify Tholos's state-before-transfer ordering holds
/// for it.
fn evil_fixture(
    env: &Env,
) -> (
    evil_token::EvilTokenClient<'static>,
    TholosClient<'static>,
    Address,
    Vec<Address>,
) {
    use evil_token::EvilToken;

    let evil_token_id = env.register(EvilToken, ());
    let evil_token = evil_token::EvilTokenClient::new(env, &evil_token_id);

    let resolvers = Vec::from_array(
        env,
        [
            Address::generate(env),
            Address::generate(env),
            Address::generate(env),
        ],
    );
    let contract_id = env.register(Tholos, ());
    let client = TholosClient::new(env, &contract_id);

    let admin = Address::generate(env);
    client.initialize(
        &admin,
        &evil_token_id,
        &DEFAULT_BOND,
        &DEFAULT_WINDOW,
        &resolvers,
        &0u32,
    );

    (evil_token, client, contract_id, resolvers)
}

#[test]
fn test_assert_outcome_is_not_reentrant() {
    use evil_token::Reentry;

    let env = Env::default();
    env.mock_all_auths();
    let (evil_token, client, contract_id, _resolvers) = evil_fixture(&env);

    let asserter = Address::generate(&env);
    let reentrant_asserter = Address::generate(&env);
    evil_token.credit(&asserter, &1_000);
    evil_token.credit(&reentrant_asserter, &1_000);

    // Arm the trap before the only externally triggered assert_outcome call:
    // EvilToken.transfer will try to reenter assert_outcome with a different
    // asserter, before this call's own transfer even returns. Soroban's auth
    // model itself rejects a dynamically-triggered nested `require_auth`
    // like this one, regardless of which address it's for, so the reentrant
    // call never gets far enough to matter. This still guards against a
    // regression: if it ever did get through (e.g. via a colluding signer
    // who pre-authorized the whole call tree), the id-reservation-before-
    // transfer ordering in `assert_outcome` is what would stop it from
    // colliding with the outer call's id.
    evil_token.configure(
        &contract_id,
        &Reentry::AssertOutcome(reentrant_asserter.clone(), true),
    );

    let id = client.assert_outcome(&asserter, &true);

    // No second assertion was created, and the reentrant asserter was never
    // charged.
    let original = client.get_assertion_state(&id);
    assert_eq!(original.asserter, asserter);
    assert_eq!(evil_token.balance(&asserter), 900);
    assert_eq!(evil_token.balance(&reentrant_asserter), 1_000);
}

#[test]
fn test_dispute_is_not_reentrant() {
    use evil_token::Reentry;

    let env = Env::default();
    env.mock_all_auths();
    let (evil_token, client, contract_id, _resolvers) = evil_fixture(&env);

    let asserter = Address::generate(&env);
    let disputer = Address::generate(&env);
    let second_disputer = Address::generate(&env);
    evil_token.credit(&asserter, &1_000);
    evil_token.credit(&disputer, &1_000);
    evil_token.credit(&second_disputer, &1_000);

    let id = client.assert_outcome(&asserter, &true);

    // Arm the trap: EvilToken.transfer will try to reenter dispute(id) with
    // a different disputer, before this dispute call's own transfer returns.
    // As with assert_outcome, Soroban's auth model rejects this nested
    // require_auth on its own; this guards against a regression in the
    // state-before-transfer ordering for the case where it didn't.
    evil_token.configure(&contract_id, &Reentry::Dispute(second_disputer.clone(), id));

    client.dispute(&disputer, &id);

    // The reentrant dispute did not happen: the second disputer was never
    // charged, and the assertion still records the original disputer.
    assert_eq!(evil_token.balance(&disputer), 900);
    assert_eq!(evil_token.balance(&second_disputer), 1_000);
    let state = client.get_assertion_state(&id);
    assert_eq!(state.disputer, Some(disputer));
}

#[test]
fn test_resolve_is_not_reentrant() {
    use evil_token::Reentry;

    let env = Env::default();
    env.mock_all_auths();
    let (evil_token, client, contract_id, resolvers) = evil_fixture(&env);

    let asserter = Address::generate(&env);
    let disputer = Address::generate(&env);
    evil_token.credit(&asserter, &1_000);
    evil_token.credit(&disputer, &1_000);

    let id = client.assert_outcome(&asserter, &true);
    client.dispute(&disputer, &id);
    client.resolve(&resolvers.get(0).unwrap(), &id, &false);

    // Arm the trap right before the majority-triggering vote: EvilToken.transfer
    // will try to reenter resolve() with the third, not-yet-voted resolver,
    // during the payout transfer of this second, majority-triggering vote.
    // As with the other auth-gated functions, Soroban's auth model rejects
    // this nested require_auth on its own; this guards against a regression
    // in the state-before-transfer ordering for the case where it didn't.
    evil_token.configure(
        &contract_id,
        &Reentry::Resolve(resolvers.get(2).unwrap(), id, false),
    );

    client.resolve(&resolvers.get(1).unwrap(), &id, &false);

    // Exactly one payout (both bonds) went to the disputer, not two.
    assert_eq!(evil_token.balance(&disputer), 1_100);
}

#[test]
fn test_finalize_is_not_reentrant() {
    use evil_token::Reentry;

    let env = Env::default();
    env.mock_all_auths();
    let (evil_token, client, contract_id, _resolvers) = evil_fixture(&env);

    let asserter = Address::generate(&env);
    let caller = Address::generate(&env);
    evil_token.credit(&asserter, &1_000);

    // The reentrancy trap isn't armed yet, so this assert_outcome call's own
    // transfer doesn't try to reenter anything.
    let id = client.assert_outcome(&asserter, &true);
    assert_eq!(evil_token.balance(&asserter), 900);

    env.ledger().with_mut(|l| l.timestamp += DEFAULT_WINDOW + 1);

    // Arm the trap: EvilToken.transfer will now try to reenter finalize(id)
    // on itself, before finalize's own transfer call even returns. Because
    // finalize requires caller.require_auth() unconditionally, Soroban's auth
    // model rejects the reentrant nested require_auth; the state-before-
    // transfer ordering is a second layer of defense.
    evil_token.configure(&contract_id, &Reentry::Finalize(caller.clone(), id));

    let outcome = client.finalize(&caller, &id);
    assert!(outcome);

    // Exactly one bond's worth was returned, not two. If Tholos wrote state
    // after the transfer instead of before, the reentrant finalize call
    // would have seen the assertion as still `Pending` and paid out again.
    assert_eq!(evil_token.balance(&asserter), 1_000);
}

// ---------------------------------------------------------------------------
// Property-based tests for resolver vote counting and majority logic
// ---------------------------------------------------------------------------
//
// These tests complement the hand-written scenarios above by generating random
// odd-length resolver committees and random vote sequences, then asserting the
// invariant: resolution happens if and only if one side has reached a strict
// majority at that step. This guards against off-by-one errors in the
// `(resolvers.len() / 2) + 1` majority formula across all valid committee sizes.
//
// Because Soroban's `Env` and `Address::generate` are not `Send`, the proptest
// tests run in-process (no forking).  `proptest!` is configured with
// `fork = false` for that reason.

mod proptest_vote_counting {
    use super::*;
    use proptest::prelude::*;

    // Use the standard-library vec for test-side bookkeeping to avoid
    // confusion with soroban_sdk::Vec (which is in scope from `super::*`
    // via the wildcard import of the contract types).
    extern crate alloc;
    use alloc::vec::Vec as StdVec;

    // Odd committee sizes from 1 to MAX_RESOLVERS (1, 3, 5, … 21).
    fn odd_committee_size() -> impl Strategy<Value = usize> {
        (0u32..=(MAX_RESOLVERS / 2)).prop_map(|n| (2 * n + 1) as usize)
    }

    // A sequence of boolean votes, length 0 to `max_len`.
    fn vote_sequence(max_len: usize) -> impl Strategy<Value = StdVec<bool>> {
        proptest::collection::vec(any::<bool>(), 0..=max_len)
    }

    /// Core fixture builder that accepts an arbitrary committee size rather
    /// than the default three resolvers.  Returns a tuple of
    /// `(Fixture, resolvers)` where `resolvers` is a plain `StdVec<Address>`
    /// for easy indexed access inside proptest closures.
    fn fixture_with_committee(committee_size: usize) -> (Fixture, StdVec<Address>) {
        let env = Env::default();
        env.mock_all_auths();

        let token_admin = Address::generate(&env);
        let token_id = env
            .register_stellar_asset_contract_v2(token_admin)
            .address();

        // Build both a Soroban Vec (for the contract call) and a plain
        // std Vec (for indexed access in tests).
        let mut resolvers_sdk = soroban_sdk::Vec::new(&env);
        let mut resolvers_std: StdVec<Address> = StdVec::new();
        for _ in 0..committee_size {
            let addr = Address::generate(&env);
            resolvers_sdk.push_back(addr.clone());
            resolvers_std.push(addr);
        }

        let contract_id = env.register(Tholos, ());
        let client = TholosClient::new(&env, &contract_id);
        let token = token::Client::new(&env, &token_id);
        let admin = Address::generate(&env);

        client.initialize(
            &admin,
            &token_id,
            &DEFAULT_BOND,
            &DEFAULT_WINDOW,
            &resolvers_sdk,
            &0u32,
        );

        let fixture = Fixture {
            env,
            client,
            token,
            token_id,
            resolvers: resolvers_sdk,
        };

        (fixture, resolvers_std)
    }

    proptest! {
        // Don't fork: Soroban's Env internals are not Send.
        #![proptest_config(ProptestConfig {
            fork: false,
            cases: 256,
            ..ProptestConfig::default()
        })]

        /// For every odd committee size and every vote sequence at most as
        /// long as the committee, the contract's return value after each cast
        /// vote matches a manually computed majority check.
        ///
        /// Votes are consumed one at a time.  After each vote the test checks
        /// whether the contract returned `Some(outcome)` (resolution reached)
        /// or `None` (no majority yet), comparing to the reference.  Once the
        /// contract resolves (returns `Some`) the assertion is in `Resolved`
        /// state and no further votes are valid or tested.
        #[test]
        fn prop_resolve_iff_majority_reached(
            committee_size in odd_committee_size(),
            // Generate up to MAX_RESOLVERS booleans; the test trims to
            // committee_size so we never exceed the number of resolvers.
            all_votes in vote_sequence(MAX_RESOLVERS as usize),
        ) {
            // Trim to at most committee_size votes (can't exceed # resolvers).
            let votes: StdVec<bool> = all_votes
                .into_iter()
                .take(committee_size)
                .collect();

            let (f, resolvers) = fixture_with_committee(committee_size);

            let asserter = f.funded_address();
            let disputer = f.funded_address();
            let id = f.client.assert_outcome(&asserter, &true);
            f.client.dispute(&disputer, &id);

            let majority = (committee_size / 2) + 1;
            let mut for_count: usize = 0;
            let mut against_count: usize = 0;
            let mut already_resolved = false;

            for (step, &agrees_with_asserter) in votes.iter().enumerate() {
                // Once resolved the assertion is closed; stop.
                if already_resolved {
                    break;
                }

                let result = f.client.resolve(&resolvers[step], &id, &agrees_with_asserter);

                if agrees_with_asserter {
                    for_count += 1;
                } else {
                    against_count += 1;
                }

                // Reference: has either side reached a strict majority?
                let expected: Option<bool> = if for_count >= majority {
                    // Asserter wins; contract emits the asserted outcome (true).
                    Some(true)
                } else if against_count >= majority {
                    // Disputer wins; contract emits the negation (!true == false).
                    Some(false)
                } else {
                    None
                };

                prop_assert_eq!(
                    result,
                    expected,
                    "step {}, committee {}, for {}, against {}, majority {}",
                    step, committee_size, for_count, against_count, majority
                );

                if expected.is_some() {
                    already_resolved = true;
                }
            }
        }

        /// Resolution never occurs with fewer votes than the strict majority
        /// threshold, regardless of which side they favour.
        ///
        /// For every odd committee size N cast exactly `majority - 1` votes
        /// all for the same side and verify the contract has not resolved.
        #[test]
        fn prop_no_resolution_below_majority(
            committee_size in odd_committee_size(),
            all_for in any::<bool>(),
        ) {
            let majority = (committee_size / 2) + 1;
            // `majority - 1` votes must never resolve; for size 1 that is 0
            // votes, so there is nothing to cast and the test trivially passes.
            let votes_to_cast = majority.saturating_sub(1);

            let (f, resolvers) = fixture_with_committee(committee_size);

            let asserter = f.funded_address();
            let disputer = f.funded_address();
            let id = f.client.assert_outcome(&asserter, &true);
            f.client.dispute(&disputer, &id);

            for (i, resolver) in resolvers.iter().enumerate().take(votes_to_cast) {
                let result = f.client.resolve(resolver, &id, &all_for);
                prop_assert_eq!(
                    result,
                    None,
                    "committee {}, majority {}, after {} of {} pre-majority votes",
                    committee_size, majority, i + 1, votes_to_cast
                );
            }
        }

        /// The majority-triggering vote always resolves the assertion.
        ///
        /// For every odd committee size N cast exactly `majority` votes all
        /// for the same side and verify the final vote returns `Some`.
        #[test]
        fn prop_resolution_at_exact_majority(
            committee_size in odd_committee_size(),
            all_for in any::<bool>(),
        ) {
            let majority = (committee_size / 2) + 1;

            let (f, resolvers) = fixture_with_committee(committee_size);

            let asserter = f.funded_address();
            let disputer = f.funded_address();
            let id = f.client.assert_outcome(&asserter, &true);
            f.client.dispute(&disputer, &id);

            // Cast majority - 1 votes: none must trigger resolution.
            for (i, resolver) in resolvers.iter().enumerate().take(majority - 1) {
                let result = f.client.resolve(resolver, &id, &all_for);
                prop_assert_eq!(
                    result,
                    None,
                    "committee {}, pre-majority vote {} returned Some unexpectedly",
                    committee_size, i
                );
            }

            // The majority-th vote must resolve.
            let final_result = f.client.resolve(&resolvers[majority - 1], &id, &all_for);
            prop_assert!(
                final_result.is_some(),
                "committee {}, majority {}: the {}-th vote must resolve the assertion",
                committee_size, majority, majority
            );
        }
    }
}

// ---------------------------------------------------------------------------
// Property-based tests for `initialize`'s bond_amount / challenge_window_secs
// boundaries
// ---------------------------------------------------------------------------
//
// The hand-written tests above (`test_cannot_initialize_with_zero_bond_amount`
// and friends) only cover a handful of picked values (0, -1, exactly the max,
// max+1). These tests fuzz the full `i128` and `u64` domains for those two
// parameters, with a fixed valid resolver committee, asserting `initialize`
// never panics (it is called via `try_initialize`, so a panic would surface
// as a test failure rather than a silently-passed `Result`) and always
// returns exactly the `Result` predicted by the validation order in
// `initialize`: `bond_amount` is checked before `challenge_window_secs`, so
// an invalid bond always yields `InvalidBondAmount` regardless of the window.
//
// Same in-process rationale as `proptest_vote_counting`: `fork = false`
// because Soroban's `Env` is not `Send`.
mod proptest_initialize_bounds {
    use super::*;
    use proptest::prelude::*;

    /// Reference implementation of `initialize`'s bond/window validation,
    /// mirroring the checks in `Tholos::initialize` (resolver count is held
    /// fixed and valid by every call site here, so it is not modeled).
    fn expected_result(bond_amount: i128, challenge_window_secs: u64) -> Result<(), Error> {
        if bond_amount <= 0 {
            return Err(Error::InvalidBondAmount);
        }
        if challenge_window_secs == 0 || challenge_window_secs > MAX_CHALLENGE_WINDOW_SECS {
            return Err(Error::InvalidChallengeWindow);
        }
        Ok(())
    }

    proptest! {
        #![proptest_config(ProptestConfig {
            fork: false,
            cases: 512,
            ..ProptestConfig::default()
        })]

        /// For any `bond_amount` and `challenge_window_secs` drawn from their
        /// full domains, `initialize` returns exactly what the reference
        /// validation predicts and never panics.
        #[test]
        fn prop_initialize_matches_reference_validation(
            bond_amount in any::<i128>(),
            challenge_window_secs in any::<u64>(),
        ) {
            let env = Env::default();
            env.mock_all_auths();

            let (token_id, resolvers) = setup(&env);
            let contract_id = env.register(Tholos, ());
            let client = TholosClient::new(&env, &contract_id);
            let admin = Address::generate(&env);

            let result = client.try_initialize(
                &admin,
                &token_id,
                &bond_amount,
                &challenge_window_secs,
                &resolvers,
                &0u32,
            );

            match expected_result(bond_amount, challenge_window_secs) {
                Ok(()) => prop_assert!(
                    result.is_ok(),
                    "bond {}, window {}: expected success, got {:?}",
                    bond_amount, challenge_window_secs, result
                ),
                Err(expected_err) => prop_assert_eq!(
                    result,
                    Err(Ok(expected_err)),
                    "bond {}, window {}",
                    bond_amount, challenge_window_secs
                ),
            }
        }

        /// Values right around the `challenge_window_secs` boundary
        /// (`MAX_CHALLENGE_WINDOW_SECS` +/- a small delta), combined with a
        /// fuzzed bond amount, to weight coverage toward the edge the
        /// hand-written tests already probe at single points.
        #[test]
        fn prop_initialize_near_challenge_window_boundary(
            bond_amount in any::<i128>(),
            delta in -5i64..=5i64,
        ) {
            let challenge_window_secs = MAX_CHALLENGE_WINDOW_SECS.saturating_add_signed(delta);

            let env = Env::default();
            env.mock_all_auths();

            let (token_id, resolvers) = setup(&env);
            let contract_id = env.register(Tholos, ());
            let client = TholosClient::new(&env, &contract_id);
            let admin = Address::generate(&env);

            let result = client.try_initialize(
                &admin,
                &token_id,
                &bond_amount,
                &challenge_window_secs,
                &resolvers,
                &0u32,
            );

            match expected_result(bond_amount, challenge_window_secs) {
                Ok(()) => prop_assert!(
                    result.is_ok(),
                    "bond {}, window {}: expected success, got {:?}",
                    bond_amount, challenge_window_secs, result
                ),
                Err(expected_err) => prop_assert_eq!(
                    result,
                    Err(Ok(expected_err)),
                    "bond {}, window {}",
                    bond_amount, challenge_window_secs
                ),
            }
        }
    }
}

#![cfg(test)]

use super::*;
use soroban_sdk::testutils::storage::Persistent as _;
use soroban_sdk::testutils::{Address as _, Ledger};

const DEFAULT_BOND: i128 = 100;
const DEFAULT_CHALLENGE_WINDOW: u64 = 3600;
const DEFAULT_FINALIZE_REWARD_BPS: u32 = 0;
const DEFAULT_REGISTRATION_SECS: u64 = 3600;
const DEFAULT_ANTI_SNIPE_EXT_SECS: u64 = 300;
const DEFAULT_ANTI_SNIPE_HARD_MAX_SECS: u64 = 1800;
const DEFAULT_REVEAL_SECS: u64 = 3600;
const DEFAULT_MAX_POSITION: i128 = 1_000_000;
const DEFAULT_MAX_TOTAL_WEIGHT: i128 = 10_000_000;
const DEFAULT_MINT: i128 = 1_000;

fn setup(env: &Env) -> Address {
    let token_admin = Address::generate(env);
    env.register_stellar_asset_contract_v2(token_admin)
        .address()
}

/// A ready-to-use, already-initialized TholosV2 instance (bond 100, 1-hour
/// challenge window, no finalize reward) with a real backing SAC token.
/// Tests that need an *uninitialized* contract, or non-default init
/// parameters, call `initialize` directly instead.
struct Fixture {
    env: Env,
    client: TholosV2Client<'static>,
    token: token::Client<'static>,
    admin: Address,
}

#[allow(clippy::too_many_arguments)]
fn init(
    client: &TholosV2Client,
    admin: &Address,
    token_id: &Address,
    base_bond: i128,
    challenge_window_secs: u64,
    finalize_reward_bps: u32,
) -> Result<Result<(), soroban_sdk::ConversionError>, Result<Error, soroban_sdk::InvokeError>> {
    client.try_initialize(
        admin,
        token_id,
        &base_bond,
        &challenge_window_secs,
        &finalize_reward_bps,
        &DEFAULT_REGISTRATION_SECS,
        &DEFAULT_ANTI_SNIPE_EXT_SECS,
        &DEFAULT_ANTI_SNIPE_HARD_MAX_SECS,
        &DEFAULT_REVEAL_SECS,
        &DEFAULT_MAX_POSITION,
        &DEFAULT_MAX_TOTAL_WEIGHT,
    )
}

/// For tests exercising registration/reveal/anti-snipe/position-bound
/// validation specifically, where the `init` helper's fixed defaults for
/// those fields would get in the way.
#[allow(clippy::too_many_arguments)]
fn init_full(
    client: &TholosV2Client,
    admin: &Address,
    token_id: &Address,
    registration_duration_secs: u64,
    anti_snipe_extension_secs: u64,
    anti_snipe_hard_max_secs: u64,
    reveal_duration_secs: u64,
    max_position: i128,
    max_total_weight: i128,
) -> Result<Result<(), soroban_sdk::ConversionError>, Result<Error, soroban_sdk::InvokeError>> {
    client.try_initialize(
        admin,
        token_id,
        &DEFAULT_BOND,
        &DEFAULT_CHALLENGE_WINDOW,
        &DEFAULT_FINALIZE_REWARD_BPS,
        &registration_duration_secs,
        &anti_snipe_extension_secs,
        &anti_snipe_hard_max_secs,
        &reveal_duration_secs,
        &max_position,
        &max_total_weight,
    )
}

impl Fixture {
    fn new() -> Self {
        let env = Env::default();
        env.mock_all_auths();

        let token_id = setup(&env);
        let token = token::Client::new(&env, &token_id);

        let contract_id = env.register(TholosV2, ());
        let client = TholosV2Client::new(&env, &contract_id);

        let admin = Address::generate(&env);
        init(
            &client,
            &admin,
            &token_id,
            DEFAULT_BOND,
            DEFAULT_CHALLENGE_WINDOW,
            DEFAULT_FINALIZE_REWARD_BPS,
        )
        .unwrap()
        .unwrap();

        Fixture {
            env,
            client,
            token,
            admin,
        }
    }

    fn generate(&self) -> Address {
        Address::generate(&self.env)
    }

    fn funded_address(&self) -> Address {
        let addr = self.generate();
        self.mint(&addr, DEFAULT_MINT);
        addr
    }

    fn mint(&self, addr: &Address, amount: i128) {
        token::StellarAssetClient::new(&self.env, &self.token.address).mint(addr, &amount);
    }

    fn advance_past_window(&self) {
        self.env
            .ledger()
            .with_mut(|l| l.timestamp += DEFAULT_CHALLENGE_WINDOW + 1);
    }
}

#[test]
fn test_initialize_pins_expected_policy() {
    let f = Fixture::new();

    let policy = f.client.get_policy();

    assert_eq!(policy.token, f.token.address);
    assert_eq!(policy.base_bond, DEFAULT_BOND);
    assert_eq!(policy.challenge_window_secs, DEFAULT_CHALLENGE_WINDOW);
    assert_eq!(policy.finalize_reward_bps, DEFAULT_FINALIZE_REWARD_BPS);
    // Decided in #64: minimum external resolution bond always equals the
    // base bond, so a third party can't break a tie for less than the
    // asserter/disputer risked.
    assert_eq!(policy.min_resolution_bond, DEFAULT_BOND);
    assert_eq!(policy.registration_duration_secs, DEFAULT_REGISTRATION_SECS);
    assert_eq!(
        policy.anti_snipe_extension_secs,
        DEFAULT_ANTI_SNIPE_EXT_SECS
    );
    assert_eq!(
        policy.anti_snipe_hard_max_secs,
        DEFAULT_ANTI_SNIPE_HARD_MAX_SECS
    );
    assert_eq!(policy.reveal_duration_secs, DEFAULT_REVEAL_SECS);
    assert_eq!(policy.weight_rule, WeightRuleVersion::LinearStakeV1);
    assert_eq!(
        policy.timeout_default,
        TimeoutDefaultRule::AssertedOutcomeStands
    );
    assert_eq!(policy.payout_rule, PayoutRuleVersion::ProRataV1);
    assert_eq!(policy.max_position, DEFAULT_MAX_POSITION);
    assert_eq!(policy.max_total_weight, DEFAULT_MAX_TOTAL_WEIGHT);
}

#[test]
fn test_initialize_twice_fails() {
    let f = Fixture::new();

    let result = init(
        &f.client,
        &f.admin,
        &f.token.address,
        DEFAULT_BOND,
        DEFAULT_CHALLENGE_WINDOW,
        DEFAULT_FINALIZE_REWARD_BPS,
    );

    assert_eq!(result, Err(Ok(Error::AlreadyInitialized)));
}

#[test]
fn test_get_policy_before_initialize_fails() {
    let env = Env::default();
    let contract_id = env.register(TholosV2, ());
    let client = TholosV2Client::new(&env, &contract_id);

    let result = client.try_get_policy();

    assert_eq!(result, Err(Ok(Error::NotInitialized)));
}

#[test]
fn test_initialize_rejects_zero_bond() {
    let env = Env::default();
    env.mock_all_auths();
    let token_id = setup(&env);
    let contract_id = env.register(TholosV2, ());
    let client = TholosV2Client::new(&env, &contract_id);
    let admin = Address::generate(&env);

    let result = init(
        &client,
        &admin,
        &token_id,
        0,
        DEFAULT_CHALLENGE_WINDOW,
        DEFAULT_FINALIZE_REWARD_BPS,
    );
    assert_eq!(result, Err(Ok(Error::InvalidBondAmount)));
}

#[test]
fn test_initialize_rejects_negative_bond() {
    let env = Env::default();
    env.mock_all_auths();
    let token_id = setup(&env);
    let contract_id = env.register(TholosV2, ());
    let client = TholosV2Client::new(&env, &contract_id);
    let admin = Address::generate(&env);

    let result = init(
        &client,
        &admin,
        &token_id,
        -1,
        DEFAULT_CHALLENGE_WINDOW,
        DEFAULT_FINALIZE_REWARD_BPS,
    );
    assert_eq!(result, Err(Ok(Error::InvalidBondAmount)));
}

#[test]
fn test_initialize_rejects_bond_over_max() {
    let env = Env::default();
    env.mock_all_auths();
    let token_id = setup(&env);
    let contract_id = env.register(TholosV2, ());
    let client = TholosV2Client::new(&env, &contract_id);
    let admin = Address::generate(&env);

    let result = init(
        &client,
        &admin,
        &token_id,
        MAX_BOND_AMOUNT + 1,
        DEFAULT_CHALLENGE_WINDOW,
        DEFAULT_FINALIZE_REWARD_BPS,
    );
    assert_eq!(result, Err(Ok(Error::InvalidBondAmount)));
}

#[test]
fn test_initialize_accepts_bond_at_max() {
    let env = Env::default();
    env.mock_all_auths();
    let token_id = setup(&env);
    let contract_id = env.register(TholosV2, ());
    let client = TholosV2Client::new(&env, &contract_id);
    let admin = Address::generate(&env);

    let result = init(
        &client,
        &admin,
        &token_id,
        MAX_BOND_AMOUNT,
        DEFAULT_CHALLENGE_WINDOW,
        DEFAULT_FINALIZE_REWARD_BPS,
    );
    assert_eq!(result, Ok(Ok(())));
}

#[test]
fn test_initialize_rejects_zero_registration_duration() {
    let env = Env::default();
    env.mock_all_auths();
    let token_id = setup(&env);
    let contract_id = env.register(TholosV2, ());
    let client = TholosV2Client::new(&env, &contract_id);
    let admin = Address::generate(&env);

    let result = init_full(
        &client,
        &admin,
        &token_id,
        0,
        DEFAULT_ANTI_SNIPE_EXT_SECS,
        DEFAULT_ANTI_SNIPE_HARD_MAX_SECS,
        DEFAULT_REVEAL_SECS,
        DEFAULT_MAX_POSITION,
        DEFAULT_MAX_TOTAL_WEIGHT,
    );
    assert_eq!(result, Err(Ok(Error::InvalidRegistrationDuration)));
}

#[test]
fn test_initialize_rejects_registration_duration_over_max() {
    let env = Env::default();
    env.mock_all_auths();
    let token_id = setup(&env);
    let contract_id = env.register(TholosV2, ());
    let client = TholosV2Client::new(&env, &contract_id);
    let admin = Address::generate(&env);

    let result = init_full(
        &client,
        &admin,
        &token_id,
        MAX_REGISTRATION_DURATION_SECS + 1,
        DEFAULT_ANTI_SNIPE_EXT_SECS,
        DEFAULT_ANTI_SNIPE_HARD_MAX_SECS,
        DEFAULT_REVEAL_SECS,
        DEFAULT_MAX_POSITION,
        DEFAULT_MAX_TOTAL_WEIGHT,
    );
    assert_eq!(result, Err(Ok(Error::InvalidRegistrationDuration)));
}

#[test]
fn test_initialize_rejects_zero_reveal_duration() {
    let env = Env::default();
    env.mock_all_auths();
    let token_id = setup(&env);
    let contract_id = env.register(TholosV2, ());
    let client = TholosV2Client::new(&env, &contract_id);
    let admin = Address::generate(&env);

    let result = init_full(
        &client,
        &admin,
        &token_id,
        DEFAULT_REGISTRATION_SECS,
        DEFAULT_ANTI_SNIPE_EXT_SECS,
        DEFAULT_ANTI_SNIPE_HARD_MAX_SECS,
        0,
        DEFAULT_MAX_POSITION,
        DEFAULT_MAX_TOTAL_WEIGHT,
    );
    assert_eq!(result, Err(Ok(Error::InvalidRevealDuration)));
}

#[test]
fn test_initialize_rejects_anti_snipe_extension_over_hard_max() {
    let env = Env::default();
    env.mock_all_auths();
    let token_id = setup(&env);
    let contract_id = env.register(TholosV2, ());
    let client = TholosV2Client::new(&env, &contract_id);
    let admin = Address::generate(&env);

    let result = init_full(
        &client,
        &admin,
        &token_id,
        DEFAULT_REGISTRATION_SECS,
        // Extension bigger than its own hard max: a single qualifying
        // deposit could blow past the deployment's stated cap in one step.
        2000,
        1800,
        DEFAULT_REVEAL_SECS,
        DEFAULT_MAX_POSITION,
        DEFAULT_MAX_TOTAL_WEIGHT,
    );
    assert_eq!(result, Err(Ok(Error::InvalidAntiSnipeParams)));
}

#[test]
fn test_initialize_accepts_anti_snipe_extension_equal_to_hard_max() {
    let env = Env::default();
    env.mock_all_auths();
    let token_id = setup(&env);
    let contract_id = env.register(TholosV2, ());
    let client = TholosV2Client::new(&env, &contract_id);
    let admin = Address::generate(&env);

    let result = init_full(
        &client,
        &admin,
        &token_id,
        DEFAULT_REGISTRATION_SECS,
        1800,
        1800,
        DEFAULT_REVEAL_SECS,
        DEFAULT_MAX_POSITION,
        DEFAULT_MAX_TOTAL_WEIGHT,
    );
    assert_eq!(result, Ok(Ok(())));
}

#[test]
fn test_initialize_rejects_zero_max_position() {
    let env = Env::default();
    env.mock_all_auths();
    let token_id = setup(&env);
    let contract_id = env.register(TholosV2, ());
    let client = TholosV2Client::new(&env, &contract_id);
    let admin = Address::generate(&env);

    let result = init_full(
        &client,
        &admin,
        &token_id,
        DEFAULT_REGISTRATION_SECS,
        DEFAULT_ANTI_SNIPE_EXT_SECS,
        DEFAULT_ANTI_SNIPE_HARD_MAX_SECS,
        DEFAULT_REVEAL_SECS,
        0,
        DEFAULT_MAX_TOTAL_WEIGHT,
    );
    assert_eq!(result, Err(Ok(Error::InvalidMaxPosition)));
}

#[test]
fn test_initialize_rejects_max_position_over_max_total_weight() {
    let env = Env::default();
    env.mock_all_auths();
    let token_id = setup(&env);
    let contract_id = env.register(TholosV2, ());
    let client = TholosV2Client::new(&env, &contract_id);
    let admin = Address::generate(&env);

    let result = init_full(
        &client,
        &admin,
        &token_id,
        DEFAULT_REGISTRATION_SECS,
        DEFAULT_ANTI_SNIPE_EXT_SECS,
        DEFAULT_ANTI_SNIPE_HARD_MAX_SECS,
        DEFAULT_REVEAL_SECS,
        DEFAULT_MAX_TOTAL_WEIGHT + 1,
        DEFAULT_MAX_TOTAL_WEIGHT,
    );
    assert_eq!(result, Err(Ok(Error::InvalidMaxPosition)));
}

#[test]
fn test_initialize_rejects_zero_max_total_weight() {
    let env = Env::default();
    env.mock_all_auths();
    let token_id = setup(&env);
    let contract_id = env.register(TholosV2, ());
    let client = TholosV2Client::new(&env, &contract_id);
    let admin = Address::generate(&env);

    let result = init_full(
        &client,
        &admin,
        &token_id,
        DEFAULT_REGISTRATION_SECS,
        DEFAULT_ANTI_SNIPE_EXT_SECS,
        DEFAULT_ANTI_SNIPE_HARD_MAX_SECS,
        DEFAULT_REVEAL_SECS,
        DEFAULT_MAX_POSITION,
        0,
    );
    assert_eq!(result, Err(Ok(Error::InvalidMaxTotalWeight)));
}

#[test]
fn test_initialize_rejects_max_total_weight_over_max_bond() {
    let env = Env::default();
    env.mock_all_auths();
    let token_id = setup(&env);
    let contract_id = env.register(TholosV2, ());
    let client = TholosV2Client::new(&env, &contract_id);
    let admin = Address::generate(&env);

    let result = init_full(
        &client,
        &admin,
        &token_id,
        DEFAULT_REGISTRATION_SECS,
        DEFAULT_ANTI_SNIPE_EXT_SECS,
        DEFAULT_ANTI_SNIPE_HARD_MAX_SECS,
        DEFAULT_REVEAL_SECS,
        MAX_BOND_AMOUNT,
        MAX_BOND_AMOUNT + 1,
    );
    assert_eq!(result, Err(Ok(Error::InvalidMaxTotalWeight)));
}

#[test]
fn test_initialize_rejects_zero_challenge_window() {
    let env = Env::default();
    env.mock_all_auths();
    let token_id = setup(&env);
    let contract_id = env.register(TholosV2, ());
    let client = TholosV2Client::new(&env, &contract_id);
    let admin = Address::generate(&env);

    let result = init(
        &client,
        &admin,
        &token_id,
        DEFAULT_BOND,
        0,
        DEFAULT_FINALIZE_REWARD_BPS,
    );
    assert_eq!(result, Err(Ok(Error::InvalidChallengeWindow)));
}

#[test]
fn test_initialize_rejects_challenge_window_over_max() {
    let env = Env::default();
    env.mock_all_auths();
    let token_id = setup(&env);
    let contract_id = env.register(TholosV2, ());
    let client = TholosV2Client::new(&env, &contract_id);
    let admin = Address::generate(&env);

    let result = init(
        &client,
        &admin,
        &token_id,
        DEFAULT_BOND,
        MAX_CHALLENGE_WINDOW_SECS + 1,
        DEFAULT_FINALIZE_REWARD_BPS,
    );
    assert_eq!(result, Err(Ok(Error::InvalidChallengeWindow)));
}

#[test]
fn test_initialize_rejects_finalize_reward_over_max() {
    let env = Env::default();
    env.mock_all_auths();
    let token_id = setup(&env);
    let contract_id = env.register(TholosV2, ());
    let client = TholosV2Client::new(&env, &contract_id);
    let admin = Address::generate(&env);

    let result = init(
        &client,
        &admin,
        &token_id,
        DEFAULT_BOND,
        DEFAULT_CHALLENGE_WINDOW,
        MAX_FINALIZE_REWARD_BPS + 1,
    );
    assert_eq!(result, Err(Ok(Error::InvalidFinalizeReward)));
}

#[test]
fn test_initialize_accepts_finalize_reward_at_max() {
    let env = Env::default();
    env.mock_all_auths();
    let token_id = setup(&env);
    let contract_id = env.register(TholosV2, ());
    let client = TholosV2Client::new(&env, &contract_id);
    let admin = Address::generate(&env);

    let result = init(
        &client,
        &admin,
        &token_id,
        DEFAULT_BOND,
        DEFAULT_CHALLENGE_WINDOW,
        MAX_FINALIZE_REWARD_BPS,
    );
    assert_eq!(result, Ok(Ok(())));
}

#[test]
fn test_assert_outcome_transfers_bond_and_pins_policy() {
    let f = Fixture::new();
    let asserter = f.funded_address();

    let id = f.client.assert_outcome(&asserter, &true);

    assert_eq!(id, 0);
    assert_eq!(f.token.balance(&asserter), DEFAULT_MINT - DEFAULT_BOND);
    assert_eq!(f.token.balance(&f.client.address), DEFAULT_BOND);

    let assertion = f.client.get_assertion(&id);
    assert_eq!(assertion.id, 0);
    assert_eq!(assertion.asserter, asserter);
    assert_eq!(assertion.disputer, None);
    assert!(assertion.outcome);
    assert_eq!(assertion.phase, PhaseV2::Pending);
    assert_eq!(assertion.policy.base_bond, DEFAULT_BOND);
    assert_eq!(assertion.terminal_cause, TerminalCause::NotYetDecided);
    assert_eq!(assertion.final_outcome, None);
    assert_eq!(assertion.finalizer, None);
}

#[test]
fn test_assert_outcome_ids_increment() {
    let f = Fixture::new();
    let asserter = f.funded_address();

    let first = f.client.assert_outcome(&asserter, &true);
    let second = f.client.assert_outcome(&asserter, &false);

    assert_eq!(first, 0);
    assert_eq!(second, 1);
}

#[test]
fn test_get_assertion_not_found() {
    let f = Fixture::new();

    let result = f.client.try_get_assertion(&0);

    assert_eq!(result, Err(Ok(Error::AssertionNotFound)));
}

#[test]
fn test_policy_hash_is_deterministic_for_identical_policy() {
    let f = Fixture::new();
    let asserter = f.funded_address();

    let id_a = f.client.assert_outcome(&asserter, &true);
    let id_b = f.client.assert_outcome(&asserter, &false);

    let a = f.client.get_assertion(&id_a);
    let b = f.client.get_assertion(&id_b);

    assert_eq!(a.policy_hash, b.policy_hash);
}

#[test]
fn test_finalize_before_window_elapses_fails() {
    let f = Fixture::new();
    let asserter = f.funded_address();
    let caller = f.generate();

    let id = f.client.assert_outcome(&asserter, &true);

    let result = f.client.try_finalize(&caller, &id);
    assert_eq!(result, Err(Ok(Error::ChallengeWindowOpen)));
}

#[test]
fn test_finalize_uncontested_with_zero_reward() {
    let f = Fixture::new();
    let asserter = f.funded_address();
    let caller = f.generate();

    let id = f.client.assert_outcome(&asserter, &true);
    assert_eq!(f.token.balance(&asserter), DEFAULT_MINT - DEFAULT_BOND);

    f.advance_past_window();

    let outcome = f.client.finalize(&caller, &id);

    assert!(outcome);
    // Zero reward bps: full bond back to the asserter, caller gets nothing.
    assert_eq!(f.token.balance(&asserter), DEFAULT_MINT);
    assert_eq!(f.token.balance(&caller), 0);

    let assertion = f.client.get_assertion(&id);
    assert_eq!(assertion.phase, PhaseV2::Resolved);
    assert_eq!(assertion.terminal_cause, TerminalCause::UncontestedFinalize);
    assert_eq!(assertion.final_outcome, Some(true));
    assert_eq!(assertion.finalizer, Some(caller));
}

#[test]
fn test_finalize_uncontested_with_nonzero_reward() {
    let env = Env::default();
    env.mock_all_auths();
    let token_id = setup(&env);
    let token = token::Client::new(&env, &token_id);
    let contract_id = env.register(TholosV2, ());
    let client = TholosV2Client::new(&env, &contract_id);
    let admin = Address::generate(&env);

    // 10% reward: bond 100 -> 10 to the finalizer, 90 to the asserter.
    init(
        &client,
        &admin,
        &token_id,
        100,
        DEFAULT_CHALLENGE_WINDOW,
        1_000,
    )
    .unwrap()
    .unwrap();

    let asserter = Address::generate(&env);
    token::StellarAssetClient::new(&env, &token_id).mint(&asserter, &DEFAULT_MINT);
    let caller = Address::generate(&env);

    let id = client.assert_outcome(&asserter, &true);
    env.ledger()
        .with_mut(|l| l.timestamp += DEFAULT_CHALLENGE_WINDOW + 1);

    client.finalize(&caller, &id);

    assert_eq!(token.balance(&caller), 10);
    assert_eq!(token.balance(&asserter), DEFAULT_MINT - 100 + 90);
}

#[test]
fn test_finalize_twice_fails() {
    let f = Fixture::new();
    let asserter = f.funded_address();
    let caller = f.generate();

    let id = f.client.assert_outcome(&asserter, &true);
    f.advance_past_window();
    f.client.finalize(&caller, &id);

    let result = f.client.try_finalize(&caller, &id);
    assert_eq!(result, Err(Ok(Error::NotPending)));
}

#[test]
fn test_finalize_nonexistent_assertion_fails() {
    let f = Fixture::new();
    let caller = f.generate();

    let result = f.client.try_finalize(&caller, &0);
    assert_eq!(result, Err(Ok(Error::AssertionNotFound)));
}

#[test]
fn test_assertion_storage_ttl_is_extended_on_finalize() {
    let f = Fixture::new();
    let asserter = f.funded_address();
    let caller = f.generate();

    let ttl_of = |id: u64| {
        f.env.as_contract(&f.client.address, || {
            f.env
                .storage()
                .persistent()
                .get_ttl(&DataKey::AssertionV2(id))
        })
    };

    let id = f.client.assert_outcome(&asserter, &true);
    assert_eq!(ttl_of(id), INSTANCE_BUMP_AMOUNT);

    f.env
        .ledger()
        .with_mut(|l| l.sequence_number += INSTANCE_BUMP_AMOUNT - 10);
    f.advance_past_window();
    f.client.finalize(&caller, &id);
    assert_eq!(ttl_of(id), INSTANCE_BUMP_AMOUNT);
}

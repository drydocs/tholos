#![cfg(test)]

use super::*;
use soroban_sdk::testutils::storage::Persistent as _;
use soroban_sdk::testutils::{Address as _, Ledger};

const DEFAULT_BOND: i128 = 100;
const DEFAULT_CHALLENGE_WINDOW: u64 = 3600;
const DEFAULT_FINALIZE_REWARD_BPS: u32 = 0;
const DEFAULT_REGISTRATION_SECS: u64 = 3600;
const DEFAULT_ANTI_SNIPE_EXT_SECS: u64 = 300;
// Must be >= DEFAULT_REGISTRATION_SECS: registration_hard_deadline is
// registration_opened_at + anti_snipe_hard_max_secs, an absolute duration
// independent of registration_duration_secs, so it can never be shorter
// than the base registration window itself.
const DEFAULT_ANTI_SNIPE_HARD_MAX_SECS: u64 = 3900;
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

    /// Posts an assertion, advances past its challenge window is NOT done
    /// here (dispute must happen within the window); returns the id so the
    /// caller can dispute it immediately.
    fn asserted(&self, asserter: &Address) -> u64 {
        self.client.assert_outcome(asserter, &true)
    }
}

/// An opaque, deterministic 32-byte commitment for tests. Reveal (#67) will
/// verify these are actually `H(canonical_encode(...))`; until then, any
/// distinct 32-byte value is enough to exercise registration's aggregation
/// and mismatch-detection rules.
fn commitment(env: &Env, seed: u8) -> BytesN<32> {
    BytesN::from_array(env, &[seed; 32])
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
        // hard_max (3700) still satisfies >= registration_duration_secs
        // (3600), so this exercises only the extension-vs-hard-max check.
        4000,
        3700,
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
        DEFAULT_REGISTRATION_SECS,
        DEFAULT_REGISTRATION_SECS,
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

#[test]
fn test_initialize_rejects_anti_snipe_hard_max_below_registration_duration() {
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
        // Hard max shorter than the base registration window itself: the
        // hard deadline would fall before the ordinary soft deadline even
        // with zero extensions ever granted.
        DEFAULT_REGISTRATION_SECS - 1,
        DEFAULT_REVEAL_SECS,
        DEFAULT_MAX_POSITION,
        DEFAULT_MAX_TOTAL_WEIGHT,
    );
    assert_eq!(result, Err(Ok(Error::InvalidAntiSnipeParams)));
}

#[test]
fn test_dispute_opens_registration_and_creates_fixed_positions() {
    let f = Fixture::new();
    let asserter = f.funded_address();
    let disputer = f.funded_address();

    let id = f.asserted(&asserter);
    f.client.dispute(&disputer, &id);

    let assertion = f.client.get_assertion(&id);
    assert_eq!(assertion.phase, PhaseV2::Registration);
    assert_eq!(assertion.disputer, Some(disputer.clone()));

    let asserter_position = f.client.get_position(&id, &asserter);
    assert_eq!(asserter_position.amount, DEFAULT_BOND);
    assert_eq!(asserter_position.kind, PositionKind::Fixed(true));

    let disputer_position = f.client.get_position(&id, &disputer);
    assert_eq!(disputer_position.amount, DEFAULT_BOND);
    assert_eq!(disputer_position.kind, PositionKind::Fixed(false));

    let resolution = f.client.get_resolution(&id);
    assert_eq!(resolution.eligible_total, DEFAULT_BOND * 2);
    assert_eq!(
        resolution.registration_deadline,
        resolution.registration_opened_at + DEFAULT_REGISTRATION_SECS
    );
    assert_eq!(
        resolution.registration_hard_deadline,
        resolution.registration_opened_at + DEFAULT_ANTI_SNIPE_HARD_MAX_SECS
    );
}

#[test]
fn test_dispute_transfers_bond() {
    let f = Fixture::new();
    let asserter = f.funded_address();
    let disputer = f.funded_address();

    let id = f.asserted(&asserter);
    assert_eq!(f.token.balance(&disputer), DEFAULT_MINT);

    f.client.dispute(&disputer, &id);

    assert_eq!(f.token.balance(&disputer), DEFAULT_MINT - DEFAULT_BOND);
    assert_eq!(f.token.balance(&f.client.address), DEFAULT_BOND * 2);
}

#[test]
fn test_dispute_by_asserter_fails() {
    let f = Fixture::new();
    let asserter = f.funded_address();

    let id = f.asserted(&asserter);

    let result = f.client.try_dispute(&asserter, &id);
    assert_eq!(result, Err(Ok(Error::DisputerIsAsserter)));
}

#[test]
fn test_dispute_non_pending_fails() {
    let f = Fixture::new();
    let asserter = f.funded_address();
    let disputer = f.funded_address();
    let second_disputer = f.funded_address();

    let id = f.asserted(&asserter);
    f.client.dispute(&disputer, &id);

    let result = f.client.try_dispute(&second_disputer, &id);
    assert_eq!(result, Err(Ok(Error::NotPending)));
}

#[test]
fn test_dispute_nonexistent_assertion_fails() {
    let f = Fixture::new();
    let disputer = f.funded_address();

    let result = f.client.try_dispute(&disputer, &0);
    assert_eq!(result, Err(Ok(Error::AssertionNotFound)));
}

#[test]
fn test_register_creates_external_position_and_transfers() {
    let f = Fixture::new();
    let asserter = f.funded_address();
    let disputer = f.funded_address();
    let voter = f.funded_address();

    let id = f.asserted(&asserter);
    f.client.dispute(&disputer, &id);

    let c = commitment(&f.env, 1);
    f.client.register(&voter, &id, &DEFAULT_BOND, &c);

    assert_eq!(f.token.balance(&voter), DEFAULT_MINT - DEFAULT_BOND);

    let position = f.client.get_position(&id, &voter);
    assert_eq!(position.amount, DEFAULT_BOND);
    assert_eq!(position.kind, PositionKind::External(c));

    let resolution = f.client.get_resolution(&id);
    assert_eq!(resolution.eligible_total, DEFAULT_BOND * 3);
}

#[test]
fn test_register_before_dispute_fails() {
    let f = Fixture::new();
    let asserter = f.funded_address();
    let voter = f.funded_address();

    let id = f.asserted(&asserter);

    let result = f
        .client
        .try_register(&voter, &id, &DEFAULT_BOND, &commitment(&f.env, 1));
    assert_eq!(result, Err(Ok(Error::NotRegistration)));
}

#[test]
fn test_register_by_asserter_fails() {
    let f = Fixture::new();
    let asserter = f.funded_address();
    let disputer = f.funded_address();

    let id = f.asserted(&asserter);
    f.client.dispute(&disputer, &id);

    let result = f
        .client
        .try_register(&asserter, &id, &DEFAULT_BOND, &commitment(&f.env, 1));
    assert_eq!(result, Err(Ok(Error::CannotRegisterAsFixedParty)));
}

#[test]
fn test_register_by_disputer_fails() {
    let f = Fixture::new();
    let asserter = f.funded_address();
    let disputer = f.funded_address();

    let id = f.asserted(&asserter);
    f.client.dispute(&disputer, &id);

    let result = f
        .client
        .try_register(&disputer, &id, &DEFAULT_BOND, &commitment(&f.env, 1));
    assert_eq!(result, Err(Ok(Error::CannotRegisterAsFixedParty)));
}

#[test]
fn test_register_zero_amount_fails() {
    let f = Fixture::new();
    let asserter = f.funded_address();
    let disputer = f.funded_address();
    let voter = f.funded_address();

    let id = f.asserted(&asserter);
    f.client.dispute(&disputer, &id);

    let result = f
        .client
        .try_register(&voter, &id, &0, &commitment(&f.env, 1));
    assert_eq!(result, Err(Ok(Error::InvalidPositionAmount)));
}

#[test]
fn test_register_below_minimum_fails() {
    let f = Fixture::new();
    let asserter = f.funded_address();
    let disputer = f.funded_address();
    let voter = f.funded_address();

    let id = f.asserted(&asserter);
    f.client.dispute(&disputer, &id);

    // min_resolution_bond equals base_bond (DEFAULT_BOND); one unit under
    // that is below the minimum for a brand-new position.
    let result = f
        .client
        .try_register(&voter, &id, &(DEFAULT_BOND - 1), &commitment(&f.env, 1));
    assert_eq!(result, Err(Ok(Error::BelowMinimumResolutionBond)));
}

#[test]
fn test_register_top_up_aggregates() {
    let f = Fixture::new();
    let asserter = f.funded_address();
    let disputer = f.funded_address();
    let voter = f.funded_address();
    f.mint(&voter, DEFAULT_MINT);

    let id = f.asserted(&asserter);
    f.client.dispute(&disputer, &id);

    let c = commitment(&f.env, 1);
    f.client.register(&voter, &id, &DEFAULT_BOND, &c);
    f.client.register(&voter, &id, &50, &c);

    let position = f.client.get_position(&id, &voter);
    assert_eq!(position.amount, DEFAULT_BOND + 50);

    let resolution = f.client.get_resolution(&id);
    assert_eq!(
        resolution.eligible_total,
        DEFAULT_BOND * 2 + DEFAULT_BOND + 50
    );
}

#[test]
fn test_register_top_up_with_different_commitment_fails() {
    let f = Fixture::new();
    let asserter = f.funded_address();
    let disputer = f.funded_address();
    let voter = f.funded_address();
    f.mint(&voter, DEFAULT_MINT);

    let id = f.asserted(&asserter);
    f.client.dispute(&disputer, &id);

    f.client
        .register(&voter, &id, &DEFAULT_BOND, &commitment(&f.env, 1));

    let result = f
        .client
        .try_register(&voter, &id, &50, &commitment(&f.env, 2));
    assert_eq!(result, Err(Ok(Error::CommitmentMismatch)));
}

#[test]
fn test_register_exceeds_max_position_fails() {
    let env = Env::default();
    env.mock_all_auths();
    let token_id = setup(&env);
    let contract_id = env.register(TholosV2, ());
    let client = TholosV2Client::new(&env, &contract_id);
    let admin = Address::generate(&env);

    // max_position tight enough that one deposit right at the bond floor is
    // fine, but a second one pushes the same position over the top.
    init_full(
        &client,
        &admin,
        &token_id,
        DEFAULT_REGISTRATION_SECS,
        DEFAULT_ANTI_SNIPE_EXT_SECS,
        DEFAULT_ANTI_SNIPE_HARD_MAX_SECS,
        DEFAULT_REVEAL_SECS,
        DEFAULT_BOND + 50,
        DEFAULT_MAX_TOTAL_WEIGHT,
    )
    .unwrap()
    .unwrap();

    let asserter = Address::generate(&env);
    let disputer = Address::generate(&env);
    let voter = Address::generate(&env);
    token::StellarAssetClient::new(&env, &token_id).mint(&asserter, &DEFAULT_MINT);
    token::StellarAssetClient::new(&env, &token_id).mint(&disputer, &DEFAULT_MINT);
    token::StellarAssetClient::new(&env, &token_id).mint(&voter, &DEFAULT_MINT);

    let id = client.assert_outcome(&asserter, &true);
    client.dispute(&disputer, &id);

    let result = client.try_register(&voter, &id, &(DEFAULT_BOND + 100), &commitment(&env, 1));
    assert_eq!(result, Err(Ok(Error::PositionExceedsMax)));
}

#[test]
fn test_register_exceeds_max_total_weight_fails() {
    let env = Env::default();
    env.mock_all_auths();
    let token_id = setup(&env);
    let contract_id = env.register(TholosV2, ());
    let client = TholosV2Client::new(&env, &contract_id);
    let admin = Address::generate(&env);

    // max_total_weight tight enough that the two fixed positions (2 *
    // DEFAULT_BOND) already consume nearly all of it. max_position must
    // stay <= max_total_weight for initialize to accept it.
    init_full(
        &client,
        &admin,
        &token_id,
        DEFAULT_REGISTRATION_SECS,
        DEFAULT_ANTI_SNIPE_EXT_SECS,
        DEFAULT_ANTI_SNIPE_HARD_MAX_SECS,
        DEFAULT_REVEAL_SECS,
        DEFAULT_BOND * 2 + 50,
        DEFAULT_BOND * 2 + 50,
    )
    .unwrap()
    .unwrap();

    let asserter = Address::generate(&env);
    let disputer = Address::generate(&env);
    let voter = Address::generate(&env);
    token::StellarAssetClient::new(&env, &token_id).mint(&asserter, &DEFAULT_MINT);
    token::StellarAssetClient::new(&env, &token_id).mint(&disputer, &DEFAULT_MINT);
    token::StellarAssetClient::new(&env, &token_id).mint(&voter, &DEFAULT_MINT);

    let id = client.assert_outcome(&asserter, &true);
    client.dispute(&disputer, &id);

    let result = client.try_register(&voter, &id, &(DEFAULT_BOND + 100), &commitment(&env, 1));
    assert_eq!(result, Err(Ok(Error::EligibleTotalExceedsMax)));
}

#[test]
fn test_register_after_deadline_fails() {
    let f = Fixture::new();
    let asserter = f.funded_address();
    let disputer = f.funded_address();
    let voter = f.funded_address();

    let id = f.asserted(&asserter);
    f.client.dispute(&disputer, &id);

    f.env
        .ledger()
        .with_mut(|l| l.timestamp += DEFAULT_REGISTRATION_SECS + 1);

    let result = f
        .client
        .try_register(&voter, &id, &DEFAULT_BOND, &commitment(&f.env, 1));
    assert_eq!(result, Err(Ok(Error::RegistrationClosed)));
}

#[test]
fn test_register_extends_deadline_on_late_qualifying_deposit() {
    let f = Fixture::new();
    let asserter = f.funded_address();
    let disputer = f.funded_address();
    let voter = f.funded_address();

    let id = f.asserted(&asserter);
    f.client.dispute(&disputer, &id);

    let before = f.client.get_resolution(&id);

    // Land within the last anti_snipe_extension_secs of the deadline.
    f.env
        .ledger()
        .with_mut(|l| l.timestamp = before.registration_deadline - DEFAULT_ANTI_SNIPE_EXT_SECS + 1);
    f.client
        .register(&voter, &id, &DEFAULT_BOND, &commitment(&f.env, 1));

    let after = f.client.get_resolution(&id);
    assert!(after.registration_deadline > before.registration_deadline);
}

#[test]
fn test_register_extension_capped_at_hard_deadline() {
    let f = Fixture::new();
    let asserter = f.funded_address();
    let disputer = f.funded_address();

    let id = f.asserted(&asserter);
    f.client.dispute(&disputer, &id);

    let opened = f.client.get_resolution(&id).registration_opened_at;
    let hard_deadline = opened + DEFAULT_ANTI_SNIPE_HARD_MAX_SECS;

    // Repeated late qualifying deposits (a fresh voter each time, so no
    // top-up/commitment-mismatch concern) keep pushing the soft deadline
    // out, but it must never cross the hard deadline fixed at dispute()
    // time.
    for i in 0..20u8 {
        let current = f.client.get_resolution(&id).registration_deadline;
        if current >= hard_deadline {
            break;
        }
        f.env
            .ledger()
            .with_mut(|l| l.timestamp = current.saturating_sub(DEFAULT_ANTI_SNIPE_EXT_SECS - 1));
        let voter = f.funded_address();
        f.client
            .register(&voter, &id, &DEFAULT_BOND, &commitment(&f.env, i));
    }

    let resolution = f.client.get_resolution(&id);
    assert!(resolution.registration_deadline <= hard_deadline);
}

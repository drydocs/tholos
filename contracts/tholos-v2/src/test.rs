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

    fn advance_past_registration_deadline(&self, id: u64) {
        let deadline = self.client.get_resolution(&id).registration_deadline;
        self.env.ledger().with_mut(|l| l.timestamp = deadline + 1);
    }

    fn advance_past_reveal_deadline(&self, id: u64) {
        let deadline = self.client.get_resolution(&id).reveal_deadline;
        self.env.ledger().with_mut(|l| l.timestamp = deadline + 1);
    }
}

/// An opaque, deterministic 32-byte commitment for tests that only exercise
/// registration (aggregation, mismatch-detection), never `reveal`, which is
/// the only place a commitment's actual hash content is checked.
fn commitment(env: &Env, seed: u8) -> BytesN<32> {
    BytesN::from_array(env, &[seed; 32])
}

fn salt(env: &Env, seed: u8) -> BytesN<32> {
    BytesN::from_array(env, &[seed; 32])
}

/// Mirrors `reveal`'s own preimage construction exactly, so tests can build
/// a commitment that will actually verify successfully.
fn compute_commitment(
    env: &Env,
    contract_id: &Address,
    policy_hash: &BytesN<32>,
    assertion_id: u64,
    voter: &Address,
    choice: bool,
    salt: &BytesN<32>,
) -> BytesN<32> {
    let preimage = VoteCommitmentPreimage {
        domain: Symbol::new(env, "THOLOS_V2_VOTE"),
        network_id: env.ledger().network_id(),
        contract_address: contract_id.clone(),
        policy_hash: policy_hash.clone(),
        assertion_id,
        round: ROUND,
        voter: voter.clone(),
        choice,
        salt: salt.clone(),
    };
    env.crypto().sha256(&preimage.to_xdr(env)).into()
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

#[test]
fn test_reveal_opens_phase_counts_fixed_positions_and_verifies_commitment() {
    let f = Fixture::new();
    let asserter = f.funded_address();
    let disputer = f.funded_address();
    let voter = f.funded_address();
    // A second, never-revealing voter keeps eligible_total above what this
    // test's one reveal accounts for, so the assertion stays Reveal instead
    // of closing to Resolved: this test is about the phase-opening/tally/
    // commitment mechanics, not the closure behavior covered separately by
    // test_reveal_last_outstanding_weight_closes_assertion.
    let other_voter = f.funded_address();

    let id = f.asserted(&asserter);
    f.client.dispute(&disputer, &id);
    let policy_hash = f.client.get_assertion(&id).policy_hash;
    let s = salt(&f.env, 1);
    let c = compute_commitment(
        &f.env,
        &f.client.address,
        &policy_hash,
        id,
        &voter,
        true,
        &s,
    );
    f.client.register(&voter, &id, &DEFAULT_BOND, &c);
    let other_commitment = commitment(&f.env, 9);
    f.client
        .register(&other_voter, &id, &DEFAULT_BOND, &other_commitment);

    f.advance_past_registration_deadline(id);
    f.client.reveal(&voter, &id, &true, &s);

    let assertion = f.client.get_assertion(&id);
    assert_eq!(assertion.phase, PhaseV2::Reveal);

    let resolution = f.client.get_resolution(&id);
    // Asserter's fixed position (agrees) + this voter's position (agrees).
    assert_eq!(resolution.agree_weight, DEFAULT_BOND * 2);
    // Disputer's fixed position (disagrees).
    assert_eq!(resolution.disagree_weight, DEFAULT_BOND);
    assert_eq!(resolution.revealed_weight(), DEFAULT_BOND * 3);

    let asserter_position = f.client.get_position(&id, &asserter);
    assert!(asserter_position.revealed);
    let disputer_position = f.client.get_position(&id, &disputer);
    assert!(disputer_position.revealed);
    let voter_position = f.client.get_position(&id, &voter);
    assert!(voter_position.revealed);
}

#[test]
fn test_reveal_disagreeing_choice_counts_disagree_weight() {
    let f = Fixture::new();
    let asserter = f.funded_address();
    let disputer = f.funded_address();
    let voter = f.funded_address();

    let id = f.asserted(&asserter);
    f.client.dispute(&disputer, &id);
    let s = salt(&f.env, 1);
    let policy_hash = f.client.get_assertion(&id).policy_hash;
    let c = compute_commitment(
        &f.env,
        &f.client.address,
        &policy_hash,
        id,
        &voter,
        false,
        &s,
    );
    f.client.register(&voter, &id, &DEFAULT_BOND, &c);

    f.advance_past_registration_deadline(id);
    f.client.reveal(&voter, &id, &false, &s);

    let resolution = f.client.get_resolution(&id);
    assert_eq!(resolution.agree_weight, DEFAULT_BOND);
    assert_eq!(resolution.disagree_weight, DEFAULT_BOND * 2);
}

#[test]
fn test_reveal_wrong_salt_fails() {
    let f = Fixture::new();
    let asserter = f.funded_address();
    let disputer = f.funded_address();
    let voter = f.funded_address();

    let id = f.asserted(&asserter);
    f.client.dispute(&disputer, &id);
    let s = salt(&f.env, 1);
    let policy_hash = f.client.get_assertion(&id).policy_hash;
    let c = compute_commitment(
        &f.env,
        &f.client.address,
        &policy_hash,
        id,
        &voter,
        true,
        &s,
    );
    f.client.register(&voter, &id, &DEFAULT_BOND, &c);

    f.advance_past_registration_deadline(id);
    let wrong_salt = salt(&f.env, 2);
    let result = f.client.try_reveal(&voter, &id, &true, &wrong_salt);
    assert_eq!(result, Err(Ok(Error::CommitmentVerificationFailed)));
}

#[test]
fn test_reveal_wrong_choice_fails() {
    let f = Fixture::new();
    let asserter = f.funded_address();
    let disputer = f.funded_address();
    let voter = f.funded_address();

    let id = f.asserted(&asserter);
    f.client.dispute(&disputer, &id);
    let s = salt(&f.env, 1);
    let policy_hash = f.client.get_assertion(&id).policy_hash;
    let c = compute_commitment(
        &f.env,
        &f.client.address,
        &policy_hash,
        id,
        &voter,
        true,
        &s,
    );
    f.client.register(&voter, &id, &DEFAULT_BOND, &c);

    f.advance_past_registration_deadline(id);
    let result = f.client.try_reveal(&voter, &id, &false, &s);
    assert_eq!(result, Err(Ok(Error::CommitmentVerificationFailed)));
}

#[test]
fn test_reveal_twice_fails() {
    let f = Fixture::new();
    let asserter = f.funded_address();
    let disputer = f.funded_address();
    let voter = f.funded_address();
    // A second, never-revealing voter keeps the assertion in Reveal after
    // the first reveal below, so a repeat call exercises the per-position
    // AlreadyRevealed guard rather than the (also correct, but different)
    // NotReveal an already-Resolved assertion would return instead.
    let other_voter = f.funded_address();

    let id = f.asserted(&asserter);
    f.client.dispute(&disputer, &id);
    let policy_hash = f.client.get_assertion(&id).policy_hash;
    let s = salt(&f.env, 1);
    let c = compute_commitment(
        &f.env,
        &f.client.address,
        &policy_hash,
        id,
        &voter,
        true,
        &s,
    );
    f.client.register(&voter, &id, &DEFAULT_BOND, &c);
    let other_commitment = commitment(&f.env, 9);
    f.client
        .register(&other_voter, &id, &DEFAULT_BOND, &other_commitment);

    f.advance_past_registration_deadline(id);
    f.client.reveal(&voter, &id, &true, &s);

    let result = f.client.try_reveal(&voter, &id, &true, &s);
    assert_eq!(result, Err(Ok(Error::AlreadyRevealed)));
}

#[test]
fn test_fixed_voter_cannot_reveal() {
    let f = Fixture::new();
    let asserter = f.funded_address();
    let disputer = f.funded_address();

    let id = f.asserted(&asserter);
    f.client.dispute(&disputer, &id);

    f.advance_past_registration_deadline(id);
    // Opens the reveal phase (which auto-reveals the fixed positions), then
    // the asserter tries to reveal something they never committed to.
    let result = f.client.try_reveal(&asserter, &id, &true, &salt(&f.env, 1));
    assert_eq!(result, Err(Ok(Error::AlreadyRevealed)));
}

#[test]
fn test_reveal_before_registration_closes_fails() {
    let f = Fixture::new();
    let asserter = f.funded_address();
    let disputer = f.funded_address();
    let voter = f.funded_address();

    let id = f.asserted(&asserter);
    f.client.dispute(&disputer, &id);
    let s = salt(&f.env, 1);
    let policy_hash = f.client.get_assertion(&id).policy_hash;
    let c = compute_commitment(
        &f.env,
        &f.client.address,
        &policy_hash,
        id,
        &voter,
        true,
        &s,
    );
    f.client.register(&voter, &id, &DEFAULT_BOND, &c);

    let result = f.client.try_reveal(&voter, &id, &true, &s);
    assert_eq!(result, Err(Ok(Error::RegistrationNotClosed)));
}

#[test]
fn test_reveal_on_pending_assertion_fails() {
    let f = Fixture::new();
    let asserter = f.funded_address();

    let id = f.asserted(&asserter);

    let result = f.client.try_reveal(&asserter, &id, &true, &salt(&f.env, 1));
    assert_eq!(result, Err(Ok(Error::NotReveal)));
}

#[test]
fn test_reveal_on_resolved_assertion_fails() {
    let f = Fixture::new();
    let asserter = f.funded_address();
    let caller = f.generate();

    let id = f.asserted(&asserter);
    f.advance_past_window();
    f.client.finalize(&caller, &id);

    let result = f.client.try_reveal(&asserter, &id, &true, &salt(&f.env, 1));
    assert_eq!(result, Err(Ok(Error::NotReveal)));
}

#[test]
fn test_reveal_nonexistent_position_fails() {
    let f = Fixture::new();
    let asserter = f.funded_address();
    let disputer = f.funded_address();
    let stranger = f.funded_address();

    let id = f.asserted(&asserter);
    f.client.dispute(&disputer, &id);
    f.advance_past_registration_deadline(id);

    let result = f.client.try_reveal(&stranger, &id, &true, &salt(&f.env, 1));
    assert_eq!(result, Err(Ok(Error::AssertionNotFound)));
}

#[test]
fn test_reveal_after_reveal_deadline_fails() {
    let f = Fixture::new();
    let asserter = f.funded_address();
    let disputer = f.funded_address();
    let first_voter = f.funded_address();
    let second_voter = f.funded_address();

    let id = f.asserted(&asserter);
    f.client.dispute(&disputer, &id);

    let policy_hash = f.client.get_assertion(&id).policy_hash;
    let first_salt = salt(&f.env, 1);
    let first_commitment = compute_commitment(
        &f.env,
        &f.client.address,
        &policy_hash,
        id,
        &first_voter,
        true,
        &first_salt,
    );
    f.client
        .register(&first_voter, &id, &DEFAULT_BOND, &first_commitment);

    let second_salt = salt(&f.env, 2);
    let second_commitment = compute_commitment(
        &f.env,
        &f.client.address,
        &policy_hash,
        id,
        &second_voter,
        true,
        &second_salt,
    );
    f.client
        .register(&second_voter, &id, &DEFAULT_BOND, &second_commitment);

    // The first voter's reveal opens the phase.
    f.advance_past_registration_deadline(id);
    f.client.reveal(&first_voter, &id, &true, &first_salt);

    // Time passes the reveal window before the second voter gets to reveal.
    let reveal_deadline = f.client.get_resolution(&id).reveal_deadline;
    f.env
        .ledger()
        .with_mut(|l| l.timestamp = reveal_deadline + 1);

    let result = f.client.try_reveal(&second_voter, &id, &true, &second_salt);
    assert_eq!(result, Err(Ok(Error::RevealClosed)));
}

#[test]
fn test_strict_majority_locks_outcome_early_for_while_staying_in_reveal() {
    let f = Fixture::new();
    let asserter = f.funded_address();
    let disputer = f.funded_address();
    let big_agree_voter = f.funded_address();
    // Registered but never revealed: keeps eligible_total above what the
    // big voter's reveal alone accounts for, so the assertion is still
    // Reveal (not yet closed to Resolved) once the lock fires, letting this
    // test check that reveals keep being accepted after locking.
    let unrevealed_voter = f.funded_address();

    let id = f.asserted(&asserter);
    f.client.dispute(&disputer, &id);
    let policy_hash = f.client.get_assertion(&id).policy_hash;

    let big_salt = salt(&f.env, 1);
    let big_commitment = compute_commitment(
        &f.env,
        &f.client.address,
        &policy_hash,
        id,
        &big_agree_voter,
        true,
        &big_salt,
    );
    f.client
        .register(&big_agree_voter, &id, &300, &big_commitment);
    let unrevealed_salt = salt(&f.env, 2);
    let unrevealed_commitment = compute_commitment(
        &f.env,
        &f.client.address,
        &policy_hash,
        id,
        &unrevealed_voter,
        false,
        &unrevealed_salt,
    );
    f.client.register(
        &unrevealed_voter,
        &id,
        &DEFAULT_BOND,
        &unrevealed_commitment,
    );

    f.advance_past_registration_deadline(id);
    f.client.reveal(&big_agree_voter, &id, &true, &big_salt);

    // eligible_total = 100 (asserter) + 100 (disputer) + 300 + 100 = 600.
    // agree_weight = 100 + 300 = 400 > 600 - 400 = 200: strict majority for.
    let assertion = f.client.get_assertion(&id);
    assert_eq!(assertion.phase, PhaseV2::Reveal);
    assert_eq!(assertion.terminal_cause, TerminalCause::StrictMajorityFor);
    assert_eq!(assertion.final_outcome, Some(true));

    let resolution = f.client.get_resolution(&id);
    assert_eq!(resolution.eligible_total, 600);
    assert!(resolution.revealed_weight() < resolution.eligible_total);

    // The still-unrevealed voter can still reveal after the lock; this is
    // the last outstanding weight, so it also closes the assertion out.
    // Its disagreeing choice doesn't change the already-locked outcome.
    f.client
        .reveal(&unrevealed_voter, &id, &false, &unrevealed_salt);

    let closed = f.client.get_assertion(&id);
    assert_eq!(closed.phase, PhaseV2::Resolved);
    assert_eq!(closed.terminal_cause, TerminalCause::StrictMajorityFor);
    assert_eq!(closed.final_outcome, Some(true));
}

#[test]
fn test_strict_majority_locks_outcome_early_against() {
    let f = Fixture::new();
    let asserter = f.funded_address();
    let disputer = f.funded_address();
    let big_disagree_voter = f.funded_address();
    let unrevealed_voter = f.funded_address();

    let id = f.asserted(&asserter);
    f.client.dispute(&disputer, &id);
    let policy_hash = f.client.get_assertion(&id).policy_hash;

    let big_salt = salt(&f.env, 1);
    let big_commitment = compute_commitment(
        &f.env,
        &f.client.address,
        &policy_hash,
        id,
        &big_disagree_voter,
        false,
        &big_salt,
    );
    f.client
        .register(&big_disagree_voter, &id, &300, &big_commitment);
    f.client.register(
        &unrevealed_voter,
        &id,
        &DEFAULT_BOND,
        &commitment(&f.env, 9),
    );

    f.advance_past_registration_deadline(id);
    f.client.reveal(&big_disagree_voter, &id, &false, &big_salt);

    // eligible_total = 600. disagree_weight = 100 + 300 = 400 > 200: strict
    // majority against, so the resolved outcome flips from the asserted one.
    let assertion = f.client.get_assertion(&id);
    assert_eq!(assertion.phase, PhaseV2::Reveal);
    assert_eq!(
        assertion.terminal_cause,
        TerminalCause::StrictMajorityAgainst
    );
    assert_eq!(assertion.final_outcome, Some(false));
}

#[test]
fn test_strict_majority_boundary_requires_more_than_half() {
    let f = Fixture::new();
    let asserter = f.funded_address();
    let disputer = f.funded_address();
    let voter = f.funded_address();

    let id = f.asserted(&asserter);
    f.client.dispute(&disputer, &id);
    let policy_hash = f.client.get_assertion(&id).policy_hash;

    // A top-up (not a first-time deposit) isn't held to min_resolution_bond,
    // so this voter can land on an odd eligible_total: 100 (first deposit)
    // + 1 (top-up) = 101, for eligible_total = 100 + 100 + 101 = 301.
    // agree_weight ends up 100 (asserter) + 101 (voter) = 201, which is
    // checked against `301 - 201 = 100`, exercising the subtraction form
    // against an odd total rather than an even one like every other test
    // here uses.
    let s = salt(&f.env, 1);
    let c = compute_commitment(
        &f.env,
        &f.client.address,
        &policy_hash,
        id,
        &voter,
        true,
        &s,
    );
    f.client.register(&voter, &id, &DEFAULT_BOND, &c);
    f.client.register(&voter, &id, &1, &c);

    f.advance_past_registration_deadline(id);
    f.client.reveal(&voter, &id, &true, &s);

    let assertion = f.client.get_assertion(&id);
    assert_eq!(assertion.terminal_cause, TerminalCause::StrictMajorityFor);
}

#[test]
fn test_resolve_outcome_closes_zero_third_party_dispute_as_optimistic_timeout() {
    let f = Fixture::new();
    let asserter = f.funded_address();
    let disputer = f.funded_address();

    let id = f.asserted(&asserter);
    f.client.dispute(&disputer, &id);

    f.advance_past_registration_deadline(id);

    // Nobody ever registered a third-party position, so no one but the
    // asserter/disputer has anything to call reveal() with, and their
    // fixed positions are already marked revealed automatically; reveal()
    // itself can never be called successfully here. resolve_outcome is the
    // only way this dispute can ever leave Registration.
    let cause = f.client.resolve_outcome(&id);
    assert_eq!(cause, TerminalCause::OptimisticTimeout);

    let assertion = f.client.get_assertion(&id);
    assert_eq!(assertion.phase, PhaseV2::Resolved);
    assert_eq!(assertion.terminal_cause, TerminalCause::OptimisticTimeout);
    // Exact 100/100 tie: neither side exceeds half of eligible_total (200),
    // so the asserted outcome (true) stands, per the timeout default.
    assert_eq!(assertion.final_outcome, Some(true));

    // Idempotent: calling it again just returns the already-decided cause.
    let cause_again = f.client.resolve_outcome(&id);
    assert_eq!(cause_again, TerminalCause::OptimisticTimeout);
}

#[test]
fn test_resolve_outcome_before_registration_deadline_fails() {
    let f = Fixture::new();
    let asserter = f.funded_address();
    let disputer = f.funded_address();

    let id = f.asserted(&asserter);
    f.client.dispute(&disputer, &id);

    let result = f.client.try_resolve_outcome(&id);
    assert_eq!(result, Err(Ok(Error::RegistrationNotClosed)));
}

#[test]
fn test_resolve_outcome_before_reveal_deadline_with_incomplete_reveal_fails() {
    let f = Fixture::new();
    let asserter = f.funded_address();
    let disputer = f.funded_address();
    let first_voter = f.funded_address();
    let second_voter = f.funded_address();

    let id = f.asserted(&asserter);
    f.client.dispute(&disputer, &id);
    let policy_hash = f.client.get_assertion(&id).policy_hash;

    let first_salt = salt(&f.env, 1);
    let first_commitment = compute_commitment(
        &f.env,
        &f.client.address,
        &policy_hash,
        id,
        &first_voter,
        true,
        &first_salt,
    );
    f.client
        .register(&first_voter, &id, &DEFAULT_BOND, &first_commitment);
    f.client
        .register(&second_voter, &id, &DEFAULT_BOND, &commitment(&f.env, 9));

    f.advance_past_registration_deadline(id);
    f.client.reveal(&first_voter, &id, &true, &first_salt);

    // second_voter never reveals, and reveal_deadline hasn't passed yet.
    let result = f.client.try_resolve_outcome(&id);
    assert_eq!(result, Err(Ok(Error::RevealNotClosed)));
}

#[test]
fn test_resolve_outcome_on_pending_assertion_fails() {
    let f = Fixture::new();
    let asserter = f.funded_address();

    let id = f.asserted(&asserter);

    let result = f.client.try_resolve_outcome(&id);
    assert_eq!(result, Err(Ok(Error::NotReveal)));
}

#[test]
fn test_optimistic_timeout_when_neither_side_reaches_majority_by_deadline() {
    let f = Fixture::new();
    let asserter = f.funded_address();
    let disputer = f.funded_address();
    let disagree_voter = f.funded_address();
    let never_revealed_voter = f.funded_address();

    let id = f.asserted(&asserter);
    f.client.dispute(&disputer, &id);
    let policy_hash = f.client.get_assertion(&id).policy_hash;

    let disagree_salt = salt(&f.env, 1);
    let disagree_commitment = compute_commitment(
        &f.env,
        &f.client.address,
        &policy_hash,
        id,
        &disagree_voter,
        false,
        &disagree_salt,
    );
    f.client
        .register(&disagree_voter, &id, &400, &disagree_commitment);
    f.client
        .register(&never_revealed_voter, &id, &400, &commitment(&f.env, 9));

    f.advance_past_registration_deadline(id);
    f.client
        .reveal(&disagree_voter, &id, &false, &disagree_salt);

    // eligible_total = 100 + 100 + 400 + 400 = 1000. disagree_weight after
    // this reveal = 100 + 400 = 500, exactly half: not a strict majority
    // (500 is not > 1000 - 500 = 500). agree_weight is only the asserter's
    // fixed 100. never_revealed_voter's 400 never reveals at all.
    let mid_assertion = f.client.get_assertion(&id);
    assert_eq!(mid_assertion.terminal_cause, TerminalCause::NotYetDecided);

    f.advance_past_reveal_deadline(id);
    let cause = f.client.resolve_outcome(&id);
    assert_eq!(cause, TerminalCause::OptimisticTimeout);

    let assertion = f.client.get_assertion(&id);
    assert_eq!(assertion.phase, PhaseV2::Resolved);
    // The asserted outcome (true) stands: the disagreeing side revealed
    // more weight than the agreeing side, but never assembled a real
    // majority of the full eligible total, exactly the "burden of proof is
    // on the challenger" case V2_RESOLUTION.md calls out.
    assert_eq!(assertion.final_outcome, Some(true));
}

#[test]
fn test_settle_strict_majority_conserves_pool_and_pays_dust_to_asserter() {
    let f = Fixture::new();
    let asserter = f.funded_address();
    let disputer = f.funded_address();
    let voter_x = f.funded_address();
    let voter_y = f.funded_address();

    let id = f.asserted(&asserter);
    f.client.dispute(&disputer, &id);
    let policy_hash = f.client.get_assertion(&id).policy_hash;

    let x_salt = salt(&f.env, 1);
    let x_commitment = compute_commitment(
        &f.env,
        &f.client.address,
        &policy_hash,
        id,
        &voter_x,
        true,
        &x_salt,
    );
    f.client.register(&voter_x, &id, &300, &x_commitment);
    let y_salt = salt(&f.env, 2);
    let y_commitment = compute_commitment(
        &f.env,
        &f.client.address,
        &policy_hash,
        id,
        &voter_y,
        true,
        &y_salt,
    );
    f.client.register(&voter_y, &id, &150, &y_commitment);

    f.advance_past_registration_deadline(id);
    f.client.reveal(&voter_x, &id, &true, &x_salt);
    f.client.reveal(&voter_y, &id, &true, &y_salt);

    let assertion = f.client.get_assertion(&id);
    assert_eq!(assertion.phase, PhaseV2::Resolved);
    assert_eq!(assertion.terminal_cause, TerminalCause::StrictMajorityFor);

    // eligible_total = 100 + 100 + 300 + 150 = 650. recipient_weight (agree)
    // = 100 + 300 + 150 = 550. forfeited_pool = 650 - 550 = 100 (the
    // disputer's forfeited fixed bond).
    // reward_asserter = floor(100 * 100 / 550) = 18
    // reward_x = floor(300 * 100 / 550) = 54
    // reward_y = floor(150 * 100 / 550) = 27
    // sum = 99, dust = 1, credited to the asserter.
    f.client.settle(&id, &asserter);
    f.client.settle(&id, &voter_x);
    f.client.settle(&id, &voter_y);
    f.client.settle(&id, &disputer);

    assert_eq!(f.client.get_credit(&id, &asserter), 100 + 18 + 1);
    assert_eq!(f.client.get_credit(&id, &voter_x), 300 + 54);
    assert_eq!(f.client.get_credit(&id, &voter_y), 150 + 27);
    assert_eq!(f.client.get_credit(&id, &disputer), 0);

    let total = f.client.get_credit(&id, &asserter)
        + f.client.get_credit(&id, &voter_x)
        + f.client.get_credit(&id, &voter_y)
        + f.client.get_credit(&id, &disputer);
    assert_eq!(total, 650);
}

#[test]
fn test_settle_strict_majority_order_independent() {
    // Same scenario as
    // test_settle_strict_majority_conserves_pool_and_pays_dust_to_asserter,
    // but settled in a different order (loser first, dust recipient last),
    // to confirm every payout is identical regardless of call order.
    let f = Fixture::new();
    let asserter = f.funded_address();
    let disputer = f.funded_address();
    let voter_x = f.funded_address();
    let voter_y = f.funded_address();

    let id = f.asserted(&asserter);
    f.client.dispute(&disputer, &id);
    let policy_hash = f.client.get_assertion(&id).policy_hash;

    let x_salt = salt(&f.env, 1);
    let x_commitment = compute_commitment(
        &f.env,
        &f.client.address,
        &policy_hash,
        id,
        &voter_x,
        true,
        &x_salt,
    );
    f.client.register(&voter_x, &id, &300, &x_commitment);
    let y_salt = salt(&f.env, 2);
    let y_commitment = compute_commitment(
        &f.env,
        &f.client.address,
        &policy_hash,
        id,
        &voter_y,
        true,
        &y_salt,
    );
    f.client.register(&voter_y, &id, &150, &y_commitment);

    f.advance_past_registration_deadline(id);
    f.client.reveal(&voter_x, &id, &true, &x_salt);
    f.client.reveal(&voter_y, &id, &true, &y_salt);

    f.client.settle(&id, &disputer);
    f.client.settle(&id, &voter_y);
    f.client.settle(&id, &voter_x);
    f.client.settle(&id, &asserter);

    assert_eq!(f.client.get_credit(&id, &asserter), 100 + 18 + 1);
    assert_eq!(f.client.get_credit(&id, &voter_x), 300 + 54);
    assert_eq!(f.client.get_credit(&id, &voter_y), 150 + 27);
    assert_eq!(f.client.get_credit(&id, &disputer), 0);
}

#[test]
fn test_settle_optimistic_timeout_conserves_pool_and_pays_dust_to_asserter() {
    let f = Fixture::new();
    let asserter = f.funded_address();
    let disputer = f.funded_address();
    let voter_a = f.funded_address();
    let voter_b = f.funded_address();
    let never_revealed = f.funded_address();

    let id = f.asserted(&asserter);
    f.client.dispute(&disputer, &id);
    let policy_hash = f.client.get_assertion(&id).policy_hash;

    let a_salt = salt(&f.env, 1);
    let a_commitment = compute_commitment(
        &f.env,
        &f.client.address,
        &policy_hash,
        id,
        &voter_a,
        true,
        &a_salt,
    );
    f.client.register(&voter_a, &id, &300, &a_commitment);
    let b_salt = salt(&f.env, 2);
    let b_commitment = compute_commitment(
        &f.env,
        &f.client.address,
        &policy_hash,
        id,
        &voter_b,
        false,
        &b_salt,
    );
    f.client.register(&voter_b, &id, &200, &b_commitment);
    f.client
        .register(&never_revealed, &id, &150, &commitment(&f.env, 9));

    f.advance_past_registration_deadline(id);
    f.client.reveal(&voter_a, &id, &true, &a_salt);
    f.client.reveal(&voter_b, &id, &false, &b_salt);

    f.advance_past_reveal_deadline(id);
    let cause = f.client.resolve_outcome(&id);
    assert_eq!(cause, TerminalCause::OptimisticTimeout);

    // eligible_total = 100 + 100 + 300 + 200 + 150 = 850. recipient_weight
    // (revealed_weight) = 100 + 100 + 300 + 200 = 700. forfeited_pool =
    // 850 - 700 = 150 (never_revealed's stake).
    // reward_asserter = floor(100 * 150 / 700) = 21
    // reward_disputer = floor(100 * 150 / 700) = 21
    // reward_a = floor(300 * 150 / 700) = 64
    // reward_b = floor(200 * 150 / 700) = 42
    // sum = 148, dust = 2, credited to the asserter (timeout default).
    f.client.settle(&id, &asserter);
    f.client.settle(&id, &disputer);
    f.client.settle(&id, &voter_a);
    f.client.settle(&id, &voter_b);
    f.client.settle(&id, &never_revealed);

    assert_eq!(f.client.get_credit(&id, &asserter), 100 + 21 + 2);
    assert_eq!(f.client.get_credit(&id, &disputer), 100 + 21);
    assert_eq!(f.client.get_credit(&id, &voter_a), 300 + 64);
    assert_eq!(f.client.get_credit(&id, &voter_b), 200 + 42);
    assert_eq!(f.client.get_credit(&id, &never_revealed), 0);

    let total = f.client.get_credit(&id, &asserter)
        + f.client.get_credit(&id, &disputer)
        + f.client.get_credit(&id, &voter_a)
        + f.client.get_credit(&id, &voter_b)
        + f.client.get_credit(&id, &never_revealed);
    assert_eq!(total, 850);
}

#[test]
fn test_settle_optimistic_timeout_fully_revealed_skips_dust_step() {
    let f = Fixture::new();
    let asserter = f.funded_address();
    let disputer = f.funded_address();

    let id = f.asserted(&asserter);
    f.client.dispute(&disputer, &id);

    f.advance_past_registration_deadline(id);
    let cause = f.client.resolve_outcome(&id);
    assert_eq!(cause, TerminalCause::OptimisticTimeout);

    // eligible_total = 200, recipient_weight = 200 (both fixed positions
    // auto-revealed), forfeited_pool = 0: nothing forfeited, so the dust
    // step never runs, and every position just recovers its own principal.
    f.client.settle(&id, &asserter);
    f.client.settle(&id, &disputer);

    assert_eq!(f.client.get_credit(&id, &asserter), 100);
    assert_eq!(f.client.get_credit(&id, &disputer), 100);
}

#[test]
fn test_settle_forfeiture_distribution_multiply_overflow_returns_error() {
    // initialize()'s max_position/max_total_weight bounds make settle()'s
    // reward multiply (position.amount.checked_mul(forfeited_pool), which
    // MAX_SETTLEMENT_TOTAL_WEIGHT's doc comment names "the forfeiture-
    // distribution multiply") unreachable through the real register/reveal
    // flow, so the overflow state is written directly into storage instead.
    let f = Fixture::new();
    let asserter = f.funded_address();
    let disputer = f.funded_address();

    let id = f.asserted(&asserter);
    f.client.dispute(&disputer, &id);

    f.advance_past_registration_deadline(id);
    let cause = f.client.resolve_outcome(&id);
    assert_eq!(cause, TerminalCause::OptimisticTimeout);

    // HALF_MAX = i128::MAX / 2 = 2^126 - 1 (Rust truncates toward zero).
    // Setting agree_weight = HALF_MAX and eligible_total = i128::MAX makes
    // forfeited_pool = i128::MAX - HALF_MAX = 2^126. With amount = HALF_MAX
    // too, the multiply is (2^126 - 1) * 2^126 = 2^252 - 2^126, roughly
    // 2^125 times past i128::MAX (2^127 - 1): deterministic overflow, not a
    // near-boundary case.
    const HALF_MAX: i128 = i128::MAX / 2;

    let mut resolution = f.client.get_resolution(&id);
    resolution.agree_weight = HALF_MAX;
    resolution.disagree_weight = 0;
    resolution.eligible_total = i128::MAX;

    let mut position = f.client.get_position(&id, &asserter);
    position.amount = HALF_MAX;

    f.env.as_contract(&f.client.address, || {
        f.env
            .storage()
            .persistent()
            .set(&DataKey::Resolution(id), &resolution);
        f.env
            .storage()
            .persistent()
            .set(&DataKey::Position(id, asserter.clone()), &position);
    });

    let result = f.client.try_settle(&id, &asserter);
    assert_eq!(result, Err(Ok(Error::SettlementArithmeticOverflow)));
}

#[test]
fn test_settle_outstanding_liability_overflow_returns_error() {
    // Independent of the reward-multiply overflow above: this position's
    // own payout math is completely ordinary (100 in, 100 out, no
    // forfeited pool at all), only the running outstanding_liability total
    // is corrupted directly in storage beforehand, isolating settle()'s
    // final resolution.outstanding_liability.checked_add(liability_increase).
    let f = Fixture::new();
    let asserter = f.funded_address();
    let disputer = f.funded_address();
    let id = f.asserted(&asserter);
    f.client.dispute(&disputer, &id);
    f.advance_past_registration_deadline(id);
    let cause = f.client.resolve_outcome(&id);
    assert_eq!(cause, TerminalCause::OptimisticTimeout);
    // Same no-voter shape as test_settle_optimistic_timeout_fully_revealed_
    // skips_dust_step: eligible_total = 200, recipient_weight = 200,
    // forfeited_pool = 0. Nothing is forfeited, so reward = 0, the dust
    // block is skipped (forfeited_pool > 0 is false), and
    // liability_increase is exactly position.amount (100) — normal,
    // unremarkable payout math. Only outstanding_liability is corrupted, to
    // i128::MAX, so the single ordinary add at the end of settle() is
    // pushed past the type's bound with nothing else in the call
    // contributing to the overflow.
    let mut resolution = f.client.get_resolution(&id);
    resolution.outstanding_liability = i128::MAX;
    f.env.as_contract(&f.client.address, || {
        f.env
            .storage()
            .persistent()
            .set(&DataKey::Resolution(id), &resolution);
    });
    let result = f.client.try_settle(&id, &asserter);
    assert_eq!(result, Err(Ok(Error::SettlementArithmeticOverflow)));
}

#[test]
fn test_settle_strict_majority_against_dust_goes_to_disputer() {
    let f = Fixture::new();
    let asserter = f.funded_address();
    let disputer = f.funded_address();
    let voter = f.funded_address();

    let id = f.asserted(&asserter);
    f.client.dispute(&disputer, &id);
    let policy_hash = f.client.get_assertion(&id).policy_hash;

    let voter_salt = salt(&f.env, 1);
    let voter_commitment = compute_commitment(
        &f.env,
        &f.client.address,
        &policy_hash,
        id,
        &voter,
        false,
        &voter_salt,
    );
    f.client.register(&voter, &id, &301, &voter_commitment);

    f.advance_past_registration_deadline(id);
    f.client.reveal(&voter, &id, &false, &voter_salt);

    let assertion = f.client.get_assertion(&id);
    assert_eq!(
        assertion.terminal_cause,
        TerminalCause::StrictMajorityAgainst
    );

    // eligible_total = 100 + 100 + 301 = 501. recipient_weight (disagree) =
    // 100 + 301 = 401. forfeited_pool = 501 - 401 = 100 (asserter's fixed
    // bond, the losing side here).
    // reward_disputer = floor(100 * 100 / 401) = 24
    // reward_voter = floor(301 * 100 / 401) = 75
    // sum = 99, dust = 1, credited to the disputer (the winning party).
    f.client.settle(&id, &asserter);
    f.client.settle(&id, &voter);
    f.client.settle(&id, &disputer);

    assert_eq!(f.client.get_credit(&id, &asserter), 0);
    assert_eq!(f.client.get_credit(&id, &voter), 301 + 75);
    assert_eq!(f.client.get_credit(&id, &disputer), 100 + 24 + 1);
}

#[test]
fn test_settle_before_resolved_fails() {
    let f = Fixture::new();
    let asserter = f.funded_address();
    let disputer = f.funded_address();

    let id = f.asserted(&asserter);
    f.client.dispute(&disputer, &id);

    let result = f.client.try_settle(&id, &asserter);
    assert_eq!(result, Err(Ok(Error::NotResolved)));
}

#[test]
fn test_settle_on_uncontested_finalize_fails() {
    let f = Fixture::new();
    let asserter = f.funded_address();

    let id = f.asserted(&asserter);
    f.advance_past_window();
    f.client.finalize(&asserter, &id);

    let result = f.client.try_settle(&id, &asserter);
    assert_eq!(result, Err(Ok(Error::NotResolved)));
}

#[test]
fn test_settle_twice_fails() {
    let f = Fixture::new();
    let asserter = f.funded_address();
    let disputer = f.funded_address();

    let id = f.asserted(&asserter);
    f.client.dispute(&disputer, &id);
    f.advance_past_registration_deadline(id);
    f.client.resolve_outcome(&id);

    f.client.settle(&id, &asserter);
    let result = f.client.try_settle(&id, &asserter);
    assert_eq!(result, Err(Ok(Error::AlreadySettled)));
}

#[test]
fn test_settle_nonexistent_position_fails() {
    let f = Fixture::new();
    let asserter = f.funded_address();
    let disputer = f.funded_address();
    let stranger = f.funded_address();

    let id = f.asserted(&asserter);
    f.client.dispute(&disputer, &id);
    f.advance_past_registration_deadline(id);
    f.client.resolve_outcome(&id);

    let result = f.client.try_settle(&id, &stranger);
    assert_eq!(result, Err(Ok(Error::AssertionNotFound)));
}

#[test]
fn test_get_credit_returns_zero_for_unknown_address() {
    let f = Fixture::new();
    let asserter = f.funded_address();
    let stranger = f.funded_address();

    let id = f.asserted(&asserter);
    assert_eq!(f.client.get_credit(&id, &stranger), 0);
}

#[test]
fn test_withdraw_transfers_credit_and_updates_resolution_totals() {
    let f = Fixture::new();
    let asserter = f.funded_address();
    let disputer = f.funded_address();

    let id = f.asserted(&asserter);
    f.client.dispute(&disputer, &id);
    f.advance_past_registration_deadline(id);
    f.client.resolve_outcome(&id);

    f.client.settle(&id, &asserter);
    assert_eq!(f.client.get_credit(&id, &asserter), DEFAULT_BOND);

    let balance_before = f.token.balance(&asserter);
    let withdrawn = f.client.withdraw(&asserter, &id, &asserter);
    assert_eq!(withdrawn, DEFAULT_BOND);
    assert_eq!(f.token.balance(&asserter), balance_before + DEFAULT_BOND);
    assert_eq!(f.client.get_credit(&id, &asserter), 0);

    let resolution = f.client.get_resolution(&id);
    assert_eq!(resolution.withdrawn_total, DEFAULT_BOND);
    // The disputer's forfeited principal is still outstanding: only the
    // asserter has settled and withdrawn so far.
    assert_eq!(resolution.outstanding_liability, 0);
}

#[test]
fn test_withdraw_to_different_destination() {
    let f = Fixture::new();
    let asserter = f.funded_address();
    let disputer = f.funded_address();
    let destination = f.generate();

    let id = f.asserted(&asserter);
    f.client.dispute(&disputer, &id);
    f.advance_past_registration_deadline(id);
    f.client.resolve_outcome(&id);
    f.client.settle(&id, &asserter);

    f.client.withdraw(&asserter, &id, &destination);

    assert_eq!(f.token.balance(&destination), DEFAULT_BOND);
    // The owner's own balance never received anything directly.
    assert_eq!(f.token.balance(&asserter), DEFAULT_MINT - DEFAULT_BOND);
}

#[test]
fn test_withdraw_with_no_credit_fails() {
    let f = Fixture::new();
    let asserter = f.funded_address();
    let disputer = f.funded_address();

    let id = f.asserted(&asserter);
    f.client.dispute(&disputer, &id);
    f.advance_past_registration_deadline(id);
    f.client.resolve_outcome(&id);

    // The disputer never settled, so has no credit to withdraw.
    let result = f.client.try_withdraw(&disputer, &id, &disputer);
    assert_eq!(result, Err(Ok(Error::NoCreditToWithdraw)));
}

#[test]
fn test_withdraw_on_uncontested_finalize_fails_with_no_credit() {
    // An uncontested assertion resolved via finalize() never had a
    // Resolution created for it at all (only dispute() does), so this must
    // surface NoCreditToWithdraw, not a misleading AssertionNotFound from
    // trying to fetch a Resolution that was never created.
    let f = Fixture::new();
    let asserter = f.funded_address();

    let id = f.asserted(&asserter);
    f.advance_past_window();
    f.client.finalize(&asserter, &id);

    let result = f.client.try_withdraw(&asserter, &id, &asserter);
    assert_eq!(result, Err(Ok(Error::NoCreditToWithdraw)));
}

#[test]
fn test_withdraw_twice_fails() {
    let f = Fixture::new();
    let asserter = f.funded_address();
    let disputer = f.funded_address();

    let id = f.asserted(&asserter);
    f.client.dispute(&disputer, &id);
    f.advance_past_registration_deadline(id);
    f.client.resolve_outcome(&id);
    f.client.settle(&id, &asserter);

    f.client.withdraw(&asserter, &id, &asserter);
    let result = f.client.try_withdraw(&asserter, &id, &asserter);
    assert_eq!(result, Err(Ok(Error::NoCreditToWithdraw)));
}

#[test]
fn test_withdraw_settle_conserves_pool_across_all_participants() {
    // Every position settles and withdraws; the sum of what actually left
    // the contract must equal what was funded, and nothing should remain
    // outstanding afterward.
    let f = Fixture::new();
    let asserter = f.funded_address();
    let disputer = f.funded_address();
    let voter = f.funded_address();

    let id = f.asserted(&asserter);
    f.client.dispute(&disputer, &id);
    let policy_hash = f.client.get_assertion(&id).policy_hash;
    let s = salt(&f.env, 1);
    let c = compute_commitment(
        &f.env,
        &f.client.address,
        &policy_hash,
        id,
        &voter,
        true,
        &s,
    );
    f.client.register(&voter, &id, &200, &c);

    f.advance_past_registration_deadline(id);
    f.client.reveal(&voter, &id, &true, &s);

    let assertion = f.client.get_assertion(&id);
    assert_eq!(assertion.phase, PhaseV2::Resolved);

    for address in [&asserter, &disputer, &voter] {
        f.client.settle(&id, address);
    }

    let mut withdrawn_total = 0;
    for address in [&asserter, &disputer, &voter] {
        let credit = f.client.get_credit(&id, address);
        if credit > 0 {
            withdrawn_total += f.client.withdraw(address, &id, address);
        }
    }

    // eligible_total = 100 (asserter) + 100 (disputer) + 200 (voter) = 400.
    assert_eq!(withdrawn_total, 400);

    let resolution = f.client.get_resolution(&id);
    assert_eq!(resolution.outstanding_liability, 0);
    assert_eq!(resolution.withdrawn_total, 400);
    assert_eq!(f.token.balance(&f.client.address), 0);
}

#[test]
fn test_reentrancy_guard_blocks_calls_while_held() {
    // Simulates a stuck guard, as if a hostile token's transfer callback
    // reentered mid-transfer and never released it, without needing a
    // custom malicious-token contract to actually trigger reentrancy.
    //
    // dispute() and register() only check the guard right before their own
    // transfer (after their other validation), so each needs its own state
    // that would otherwise succeed, to prove the guard is what's actually
    // blocking them rather than an unrelated validation error.
    let f = Fixture::new();
    let asserter = f.funded_address();
    let disputer = f.funded_address();
    let voter = f.funded_address();

    let pending_id = f.asserted(&asserter);

    let finalize_id = f.asserted(&asserter);
    f.advance_past_window();

    let registration_id = f.asserted(&asserter);
    f.client.dispute(&disputer, &registration_id);

    let id = f.asserted(&asserter);
    f.client.dispute(&disputer, &id);

    f.env.as_contract(&f.client.address, || {
        f.env
            .storage()
            .instance()
            .set(&DataKey::ReentrancyGuard, &true);
    });

    assert_eq!(
        f.client.try_assert_outcome(&asserter, &true),
        Err(Ok(Error::ReentrancyGuardActive))
    );
    assert_eq!(
        f.client.try_dispute(&disputer, &pending_id),
        Err(Ok(Error::ReentrancyGuardActive))
    );

    // cancel_round only works while paused; set_paused_v2 isn't itself
    // guarded (it only ever touches the Paused flag, never a position's
    // weight, credit, or terminal state), so calling it here doesn't
    // disturb the guard. pending_id is still NotYetDecided, since the
    // guard-blocked dispute() above reverted entirely, and is reused here
    // to prove the guard, not the pause itself, is what blocks
    // cancel_round.
    f.client.set_paused_v2(&true);
    assert_eq!(
        f.client.try_cancel_round(&pending_id),
        Err(Ok(Error::ReentrancyGuardActive))
    );
    assert_eq!(
        f.client.try_finalize(&asserter, &finalize_id),
        Err(Ok(Error::ReentrancyGuardActive))
    );
    assert_eq!(
        f.client.try_register(
            &voter,
            &registration_id,
            &DEFAULT_BOND,
            &commitment(&f.env, 1)
        ),
        Err(Ok(Error::ReentrancyGuardActive))
    );

    // Advancing the ledger clock is a pure test-harness operation, not a
    // contract call, so it's safe to do here while the guard is still held;
    // id's registration window needs to be closed before reveal/
    // resolve_outcome/settle/withdraw below are meaningful to call at all.
    f.advance_past_registration_deadline(id);

    assert_eq!(
        f.client.try_reveal(&asserter, &id, &true, &salt(&f.env, 1)),
        Err(Ok(Error::ReentrancyGuardActive))
    );
    assert_eq!(
        f.client.try_resolve_outcome(&id),
        Err(Ok(Error::ReentrancyGuardActive))
    );
    assert_eq!(
        f.client.try_settle(&id, &asserter),
        Err(Ok(Error::ReentrancyGuardActive))
    );
    assert_eq!(
        f.client.try_withdraw(&asserter, &id, &asserter),
        Err(Ok(Error::ReentrancyGuardActive))
    );

    // Releasing it lets calls through again.
    f.env.as_contract(&f.client.address, || {
        f.env
            .storage()
            .instance()
            .set(&DataKey::ReentrancyGuard, &false);
    });
    let cause = f.client.resolve_outcome(&id);
    assert_eq!(cause, TerminalCause::OptimisticTimeout);
}

#[test]
fn test_set_paused_v2_blocks_new_assertions() {
    let f = Fixture::new();
    let asserter = f.funded_address();

    f.client.set_paused_v2(&true);

    let result = f.client.try_assert_outcome(&asserter, &true);
    assert_eq!(result, Err(Ok(Error::Paused)));

    f.client.set_paused_v2(&false);
    f.client.assert_outcome(&asserter, &true);
}

#[test]
fn test_set_paused_v2_does_not_block_existing_round() {
    // The narrower v2 pause only ever gates assert_outcome: an
    // already-active round's registration, reveal, resolution, settlement,
    // and withdrawal all continue normally while paused.
    let f = Fixture::new();
    let asserter = f.funded_address();
    let disputer = f.funded_address();
    let voter = f.funded_address();

    let id = f.asserted(&asserter);
    f.client.dispute(&disputer, &id);
    let policy_hash = f.client.get_assertion(&id).policy_hash;
    let s = salt(&f.env, 1);
    let c = compute_commitment(
        &f.env,
        &f.client.address,
        &policy_hash,
        id,
        &voter,
        true,
        &s,
    );

    f.client.set_paused_v2(&true);

    f.client.register(&voter, &id, &DEFAULT_BOND, &c);
    f.advance_past_registration_deadline(id);
    f.client.reveal(&voter, &id, &true, &s);

    let assertion = f.client.get_assertion(&id);
    assert_eq!(assertion.phase, PhaseV2::Resolved);

    f.client.settle(&id, &asserter);
    f.client.withdraw(&asserter, &id, &asserter);
}

#[test]
fn test_cancel_round_before_pause_fails() {
    let f = Fixture::new();
    let asserter = f.funded_address();

    let id = f.asserted(&asserter);

    let result = f.client.try_cancel_round(&id);
    assert_eq!(result, Err(Ok(Error::NotPaused)));
}

#[test]
fn test_cancel_round_nonexistent_assertion_fails() {
    let f = Fixture::new();
    f.client.set_paused_v2(&true);

    let result = f.client.try_cancel_round(&0);
    assert_eq!(result, Err(Ok(Error::AssertionNotFound)));
}

#[test]
fn test_cancel_round_refunds_pending_asserter_directly() {
    let f = Fixture::new();
    let asserter = f.funded_address();

    let id = f.asserted(&asserter);
    let balance_before = f.token.balance(&asserter);

    f.client.set_paused_v2(&true);
    f.client.cancel_round(&id);

    assert_eq!(f.token.balance(&asserter), balance_before + DEFAULT_BOND);

    let assertion = f.client.get_assertion(&id);
    assert_eq!(assertion.phase, PhaseV2::Resolved);
    assert_eq!(assertion.terminal_cause, TerminalCause::AdminCancelled);
    assert_eq!(assertion.final_outcome, None);
}

#[test]
fn test_cancel_round_during_registration_refunds_everyone_full_principal() {
    let f = Fixture::new();
    let asserter = f.funded_address();
    let disputer = f.funded_address();
    let voter = f.funded_address();

    let id = f.asserted(&asserter);
    f.client.dispute(&disputer, &id);
    let policy_hash = f.client.get_assertion(&id).policy_hash;
    let s = salt(&f.env, 1);
    let c = compute_commitment(
        &f.env,
        &f.client.address,
        &policy_hash,
        id,
        &voter,
        true,
        &s,
    );
    f.client.register(&voter, &id, &250, &c);

    f.client.set_paused_v2(&true);
    f.client.cancel_round(&id);

    let assertion = f.client.get_assertion(&id);
    assert_eq!(assertion.phase, PhaseV2::Resolved);
    assert_eq!(assertion.terminal_cause, TerminalCause::AdminCancelled);
    assert_eq!(assertion.final_outcome, None);

    // Every position, including the never-revealed voter, recovers exactly
    // its own principal: no forfeiture, no reward.
    for (address, expected) in [
        (&asserter, DEFAULT_BOND),
        (&disputer, DEFAULT_BOND),
        (&voter, 250),
    ] {
        let payout = f.client.settle(&id, address);
        assert_eq!(payout, expected);
    }

    let resolution = f.client.get_resolution(&id);
    assert_eq!(resolution.outstanding_liability, DEFAULT_BOND * 2 + 250);

    for address in [&asserter, &disputer, &voter] {
        let balance_before = f.token.balance(address);
        let withdrawn = f.client.withdraw(address, &id, address);
        assert_eq!(f.token.balance(address), balance_before + withdrawn);
    }

    let resolution = f.client.get_resolution(&id);
    assert_eq!(resolution.outstanding_liability, 0);
    assert_eq!(resolution.withdrawn_total, DEFAULT_BOND * 2 + 250);
    assert_eq!(f.token.balance(&f.client.address), 0);
}

#[test]
fn test_cancel_round_during_reveal_still_refunds_full_principal_no_reward() {
    // A position that already revealed, and one that never gets the
    // chance to, both just recover their own principal on cancellation:
    // revealing early confers no advantage over a cancelled round.
    let f = Fixture::new();
    let asserter = f.funded_address();
    let disputer = f.funded_address();
    let revealed_voter = f.funded_address();
    let unrevealed_voter = f.funded_address();

    let id = f.asserted(&asserter);
    f.client.dispute(&disputer, &id);
    let policy_hash = f.client.get_assertion(&id).policy_hash;
    let s = salt(&f.env, 1);
    let c = compute_commitment(
        &f.env,
        &f.client.address,
        &policy_hash,
        id,
        &revealed_voter,
        true,
        &s,
    );
    f.client.register(&revealed_voter, &id, &DEFAULT_BOND, &c);
    f.client
        .register(&unrevealed_voter, &id, &300, &commitment(&f.env, 9));

    f.advance_past_registration_deadline(id);
    f.client.reveal(&revealed_voter, &id, &true, &s);

    let mid_assertion = f.client.get_assertion(&id);
    assert_eq!(mid_assertion.phase, PhaseV2::Reveal);
    assert_eq!(mid_assertion.terminal_cause, TerminalCause::NotYetDecided);

    f.client.set_paused_v2(&true);
    f.client.cancel_round(&id);

    let assertion = f.client.get_assertion(&id);
    assert_eq!(assertion.phase, PhaseV2::Resolved);
    assert_eq!(assertion.terminal_cause, TerminalCause::AdminCancelled);

    assert_eq!(f.client.settle(&id, &asserter), DEFAULT_BOND);
    assert_eq!(f.client.settle(&id, &disputer), DEFAULT_BOND);
    assert_eq!(f.client.settle(&id, &revealed_voter), DEFAULT_BOND);
    assert_eq!(f.client.settle(&id, &unrevealed_voter), 300);
}

#[test]
fn test_cancel_round_after_strict_majority_fails() {
    let f = Fixture::new();
    let asserter = f.funded_address();
    let disputer = f.funded_address();
    let voter = f.funded_address();

    let id = f.asserted(&asserter);
    f.client.dispute(&disputer, &id);
    let policy_hash = f.client.get_assertion(&id).policy_hash;
    let s = salt(&f.env, 1);
    let c = compute_commitment(
        &f.env,
        &f.client.address,
        &policy_hash,
        id,
        &voter,
        true,
        &s,
    );
    f.client.register(&voter, &id, &300, &c);

    f.advance_past_registration_deadline(id);
    f.client.reveal(&voter, &id, &true, &s);

    let assertion = f.client.get_assertion(&id);
    assert_eq!(assertion.terminal_cause, TerminalCause::StrictMajorityFor);

    f.client.set_paused_v2(&true);
    let result = f.client.try_cancel_round(&id);
    assert_eq!(result, Err(Ok(Error::RoundAlreadyDecided)));
}

#[test]
fn test_cancel_round_after_optimistic_timeout_fails() {
    let f = Fixture::new();
    let asserter = f.funded_address();
    let disputer = f.funded_address();

    let id = f.asserted(&asserter);
    f.client.dispute(&disputer, &id);
    f.advance_past_registration_deadline(id);
    f.client.resolve_outcome(&id);

    f.client.set_paused_v2(&true);
    let result = f.client.try_cancel_round(&id);
    assert_eq!(result, Err(Ok(Error::RoundAlreadyDecided)));
}

#[test]
fn test_cancel_round_twice_fails() {
    let f = Fixture::new();
    let asserter = f.funded_address();

    let id = f.asserted(&asserter);

    f.client.set_paused_v2(&true);
    f.client.cancel_round(&id);

    let result = f.client.try_cancel_round(&id);
    assert_eq!(result, Err(Ok(Error::RoundAlreadyDecided)));
}

#[test]
fn test_cancel_round_on_uncontested_finalize_fails() {
    let f = Fixture::new();
    let asserter = f.funded_address();

    let id = f.asserted(&asserter);
    f.advance_past_window();
    f.client.finalize(&asserter, &id);

    f.client.set_paused_v2(&true);
    let result = f.client.try_cancel_round(&id);
    assert_eq!(result, Err(Ok(Error::RoundAlreadyDecided)));
}

// ---------------------------------------------------------------------------
// Property-based tests for settlement's pro-rata forfeiture splitting and
// dust routing (see `settle`)
// ---------------------------------------------------------------------------
//
// The hand-written tests above (`test_settle_strict_majority_conserves_pool_
// and_pays_dust_to_asserter`, `test_settle_optimistic_timeout_conserves_pool_
// and_pays_dust_to_asserter`, etc.) each pin one specific weight distribution
// by hand. These tests instead generate a random number of third-party
// positions with random weights, random sides, and a random subset that
// never reveals at all, force the round through to `Resolved` (whichever
// terminal cause that reveal pattern actually produces), and check the two
// invariants #106 asks for against `settlement_pool`/`settle`'s own formula:
//
//   1. `prop_settlement_payouts_conserve_pool_and_match_formula`: every
//      settled position's payout matches the documented pro-rata formula
//      (`amount` plus `floor(amount * forfeited_pool / recipient_weight)`
//      for a recipient, 0 otherwise) exactly, and the sum of every payout
//      across every position -- winners, losers, and the dust recipient --
//      exactly equals `eligible_total`: nothing lost, nothing invented,
//      regardless of how many positions there are or how their weight is
//      distributed.
//   2. `prop_dust_credited_to_exactly_one_recipient`: the one-time
//      floor-division remainder ("dust") is credited to exactly one address
//      (the deterministic dust recipient `settle` documents: the winning
//      asserter or disputer), never split across recipients, never dropped,
//      and never paid to more than one address.
//
// Same in-process rationale as `contracts/tholos/src/test.rs`'s
// `proptest_vote_counting`/`proptest_initialize_bounds`: Soroban's `Env` is
// not `Send`, so these run with `fork: false`.
mod proptest_settlement {
    use super::*;
    use proptest::prelude::*;

    // Use the standard-library vec for test-side bookkeeping, mirroring
    // contracts/tholos/src/test.rs's `proptest_vote_counting` (`Vec` in
    // scope from `super::*`'s wildcard import is `soroban_sdk::Vec`).
    extern crate alloc;
    use alloc::vec::Vec as StdVec;

    /// One third-party registrant. `agrees` doubles as the exact `choice`
    /// passed to `register`/`reveal`: `Fixture::asserted` always asserts
    /// outcome `true`, so "agrees with the asserted outcome" and "choice ==
    /// true" are the same thing. `reveals` is whether this voter ever calls
    /// `reveal` at all -- a voter that never reveals is exactly the
    /// "leftover, never-recovered weight" scenario dust routing exists for.
    #[derive(Clone, Debug)]
    struct Voter {
        amount: i128,
        agrees: bool,
        reveals: bool,
    }

    /// Realistic-range weights (at least `min_resolution_bond`, i.e.
    /// `DEFAULT_BOND`, well under `max_position`/`max_total_weight`) rather
    /// than the full `i128` domain, matching how v1's committee/vote
    /// generators are scoped to valid values rather than fuzzing rejection
    /// paths. Up to 6 third-party voters, on top of the asserter's and
    /// disputer's always-present fixed positions.
    fn voters() -> impl Strategy<Value = StdVec<Voter>> {
        proptest::collection::vec(
            (DEFAULT_BOND..=500_000i128, any::<bool>(), any::<bool>()),
            0..=6,
        )
        .prop_map(|raw| {
            raw.into_iter()
                .map(|(amount, agrees, reveals)| Voter {
                    amount,
                    agrees,
                    reveals,
                })
                .collect()
        })
    }

    /// One position's outcome after a scenario resolves: its address, staked
    /// `amount`, and `agrees_with_outcome` (`None` if it never revealed --
    /// always the case for a `Voter` with `reveals: false`).
    struct SettledPosition {
        address: Address,
        amount: i128,
        agrees_with_outcome: Option<bool>,
    }

    /// Runs one full randomized dispute -- asserts, disputes, registers
    /// every `Voter`, reveals the ones marked `reveals`, then force-closes
    /// the round by advancing past the reveal deadline and calling
    /// `resolve_outcome` -- so every case reaches `Resolved` regardless of
    /// whether a strict majority locked early or nobody ever revealed at
    /// all (see `PhaseV2::Reveal`'s doc comment: an early majority lock
    /// doesn't stop further reveals or close the phase by itself).
    ///
    /// Returns the fixture, the assertion id, and every position (asserter,
    /// disputer, and every registered voter) for the caller to compute
    /// expected payouts from and settle.
    fn run_scenario(voter_specs: &[Voter]) -> (Fixture, u64, StdVec<SettledPosition>) {
        let f = Fixture::new();
        let asserter = f.funded_address();
        let disputer = f.funded_address();

        let id = f.asserted(&asserter);
        f.client.dispute(&disputer, &id);
        let policy_hash = f.client.get_assertion(&id).policy_hash;

        // Fixed positions: always revealed automatically once Reveal opens
        // (see `open_reveal_phase`), asserter agreeing, disputer disagreeing.
        let mut positions: StdVec<SettledPosition> = StdVec::from([
            SettledPosition {
                address: asserter.clone(),
                amount: DEFAULT_BOND,
                agrees_with_outcome: Some(true),
            },
            SettledPosition {
                address: disputer.clone(),
                amount: DEFAULT_BOND,
                agrees_with_outcome: Some(false),
            },
        ]);

        // Register every voter first (registration must be fully closed
        // before any reveal is accepted).
        let mut registered: StdVec<(Address, BytesN<32>, bool, i128)> = StdVec::new();
        for (i, spec) in voter_specs.iter().enumerate() {
            let voter = f.generate();
            f.mint(&voter, spec.amount);
            let voter_salt = salt(&f.env, i as u8);
            let voter_commitment = compute_commitment(
                &f.env,
                &f.client.address,
                &policy_hash,
                id,
                &voter,
                spec.agrees,
                &voter_salt,
            );
            f.client
                .register(&voter, &id, &spec.amount, &voter_commitment);
            registered.push((voter, voter_salt, spec.agrees, spec.amount));
        }

        f.advance_past_registration_deadline(id);

        if registered.is_empty() {
            // No third-party weight at all: the fixed positions alone
            // already account for the full `eligible_total`, so this one
            // call both lazily opens `Reveal` (see `open_reveal_phase`)
            // and immediately closes it to `Resolved` in the same
            // successful invocation -- no earlier `reveal` call is needed
            // to persist anything first.
            f.client.resolve_outcome(&id);
        } else {
            // `Reveal`'s deadline only gets persisted on a call that
            // *succeeds*: Soroban rolls back every storage write of a
            // failing invocation, so a bare `resolve_outcome` before any
            // real weight has revealed would open-then-immediately-fail
            // (`RevealNotClosed`, since revealed weight can't yet have
            // caught up with `eligible_total`) and roll the phase change
            // back with it, leaving `reveal_deadline` at its 0 default
            // forever. A real deployment escapes this the same way: at
            // least one participant (often the asserter or disputer,
            // chasing their own payout) eventually calls `reveal`, which
            // succeeds regardless of whether the round fully closes right
            // then, and that's what actually starts the reveal clock. If
            // every generated `Voter` says `reveals: false`, force the
            // first one to reveal anyway so the scenario doesn't deadlock;
            // the bookkeeping below reflects what actually happened
            // on-chain, not the original random spec.
            let mut reveal_flags: StdVec<bool> = voter_specs.iter().map(|v| v.reveals).collect();
            if !reveal_flags.iter().any(|&r| r) {
                reveal_flags[0] = true;
            }

            for (i, (voter, voter_salt, choice, amount)) in registered.iter().enumerate() {
                if reveal_flags[i] {
                    f.client.reveal(voter, &id, choice, voter_salt);
                    positions.push(SettledPosition {
                        address: voter.clone(),
                        amount: *amount,
                        agrees_with_outcome: Some(*choice),
                    });
                } else {
                    positions.push(SettledPosition {
                        address: voter.clone(),
                        amount: *amount,
                        agrees_with_outcome: None,
                    });
                }
            }

            // At least one successful reveal above guarantees `Reveal` is
            // open with a real, persisted `reveal_deadline`; force-close
            // whatever hasn't already resolved (e.g. via an early strict
            // majority plus full reveal) by advancing past it.
            if f.client.get_assertion(&id).phase != PhaseV2::Resolved {
                f.advance_past_reveal_deadline(id);
                f.client.resolve_outcome(&id);
            }
        }

        (f, id, positions)
    }

    /// `settlement_pool`'s own rule, mirrored here so the reference
    /// calculation is independent of (and thus actually checks) the
    /// contract's implementation rather than restating it.
    fn expected_pool(
        terminal_cause: TerminalCause,
        agree_weight: i128,
        disagree_weight: i128,
        eligible_total: i128,
    ) -> (i128, i128) {
        let recipient_weight = match terminal_cause {
            TerminalCause::StrictMajorityFor => agree_weight,
            TerminalCause::StrictMajorityAgainst => disagree_weight,
            TerminalCause::OptimisticTimeout => agree_weight + disagree_weight,
            other => unreachable!(
                "run_scenario only ever produces a contested resolution: got {:?}",
                other
            ),
        };
        (recipient_weight, eligible_total - recipient_weight)
    }

    /// Whether a position is a recipient (recovers principal + pro-rata
    /// reward) under `terminal_cause`, mirroring `settle`'s own
    /// `is_recipient` match arm by arm.
    fn is_recipient(terminal_cause: TerminalCause, agrees_with_outcome: Option<bool>) -> bool {
        match terminal_cause {
            TerminalCause::StrictMajorityFor => agrees_with_outcome == Some(true),
            TerminalCause::StrictMajorityAgainst => agrees_with_outcome == Some(false),
            TerminalCause::OptimisticTimeout => agrees_with_outcome.is_some(),
            other => unreachable!(
                "run_scenario only ever produces a contested resolution: got {:?}",
                other
            ),
        }
    }

    /// Every settled position's expected payout (principal + pro-rata
    /// reward for a recipient, 0 otherwise) under the documented formula,
    /// alongside the leftover dust and its deterministic recipient.
    struct Expected {
        payouts: StdVec<(Address, i128, i128)>, // (address, amount, payout)
        dust: i128,
        dust_recipient: Address,
    }

    fn compute_expected(
        assertion: &AssertionV2,
        recipient_weight: i128,
        forfeited_pool: i128,
        positions: &[SettledPosition],
    ) -> Expected {
        let dust_recipient = match assertion.terminal_cause {
            TerminalCause::StrictMajorityFor | TerminalCause::OptimisticTimeout => {
                assertion.asserter.clone()
            }
            TerminalCause::StrictMajorityAgainst => assertion
                .disputer
                .clone()
                .expect("disputer set once phase reaches Registration"),
            other => unreachable!("unexpected terminal cause: {:?}", other),
        };

        let mut reward_total: i128 = 0;
        let mut payouts: StdVec<(Address, i128, i128)> = StdVec::new();
        for position in positions {
            let recipient = is_recipient(assertion.terminal_cause, position.agrees_with_outcome);
            let reward = if recipient {
                (position.amount * forfeited_pool) / recipient_weight
            } else {
                0
            };
            reward_total += reward;
            let payout = if recipient {
                position.amount + reward
            } else {
                0
            };
            payouts.push((position.address.clone(), position.amount, payout));
        }

        Expected {
            payouts,
            dust: forfeited_pool - reward_total,
            dust_recipient,
        }
    }

    proptest! {
        // Don't fork: Soroban's Env internals are not Send.
        #![proptest_config(ProptestConfig {
            fork: false,
            cases: 96,
            ..ProptestConfig::default()
        })]

        /// For any random distribution of third-party weights, sides, and
        /// reveal participation: `settle`'s own return value for every
        /// position matches the documented pro-rata formula exactly (and
        /// never includes dust routed elsewhere, even when that position
        /// itself turns out to be the dust recipient -- see `settle`'s doc
        /// comment), no payout ever exceeds its position's principal plus
        /// the entire forfeited pool, and -- once every position has
        /// settled and whatever dust exists has landed via `get_credit`,
        /// regardless of settle order -- the sum of every credited balance
        /// exactly equals `eligible_total`. This is the property #106 calls
        /// "payouts sum correctly and never exceed the pool".
        #[test]
        fn prop_settlement_payouts_conserve_pool_and_match_formula(voter_specs in voters()) {
            let (f, id, positions) = run_scenario(&voter_specs);

            let assertion = f.client.get_assertion(&id);
            let resolution = f.client.get_resolution(&id);
            prop_assert_eq!(assertion.phase, PhaseV2::Resolved);

            let (recipient_weight, forfeited_pool) = expected_pool(
                assertion.terminal_cause,
                resolution.agree_weight,
                resolution.disagree_weight,
                resolution.eligible_total,
            );
            prop_assert!(recipient_weight > 0);
            prop_assert!(forfeited_pool >= 0);

            let expected = compute_expected(&assertion, recipient_weight, forfeited_pool, &positions);
            prop_assert!(expected.dust >= 0);

            for (address, amount, payout) in &expected.payouts {
                let actual = f.client.settle(&id, address);
                prop_assert_eq!(
                    actual, *payout,
                    "settle() returned {} but its own pro-rata payout should be {} (recipient_weight {}, forfeited_pool {})",
                    actual, payout, recipient_weight, forfeited_pool
                );
                prop_assert!(actual <= amount + forfeited_pool);
            }

            let mut credited_total: i128 = 0;
            for (address, _amount, payout) in &expected.payouts {
                let extra_dust = if *address == expected.dust_recipient { expected.dust } else { 0 };
                prop_assert_eq!(
                    f.client.get_credit(&id, address), payout + extra_dust,
                    "final credited balance should be this position's payout plus any dust it collects"
                );
                credited_total += f.client.get_credit(&id, address);
            }

            prop_assert_eq!(credited_total, resolution.eligible_total);
        }

        /// The floor-division remainder from pro-rata splitting is credited
        /// (via `get_credit`, which reflects `settle`'s side effects rather
        /// than its per-call return value -- see the previous test's doc
        /// comment) to exactly one address, the deterministic dust
        /// recipient, and every other settled position ends up with exactly
        /// its own formula-computed payout, no more, no less: dust is never
        /// split across recipients and never silently dropped.
        #[test]
        fn prop_dust_credited_to_exactly_one_recipient(voter_specs in voters()) {
            let (f, id, positions) = run_scenario(&voter_specs);

            let assertion = f.client.get_assertion(&id);
            let resolution = f.client.get_resolution(&id);

            let (recipient_weight, forfeited_pool) = expected_pool(
                assertion.terminal_cause,
                resolution.agree_weight,
                resolution.disagree_weight,
                resolution.eligible_total,
            );
            let expected = compute_expected(&assertion, recipient_weight, forfeited_pool, &positions);

            for (address, _amount, _payout) in &expected.payouts {
                f.client.settle(&id, address);
            }

            let mut addresses_with_bonus = 0u32;
            for (address, _amount, formula_payout) in &expected.payouts {
                let bonus = f.client.get_credit(&id, address) - formula_payout;
                if *address == expected.dust_recipient {
                    prop_assert_eq!(
                        bonus, expected.dust,
                        "dust recipient's bonus must equal the full dust amount"
                    );
                    if expected.dust > 0 {
                        addresses_with_bonus += 1;
                    }
                } else {
                    prop_assert_eq!(bonus, 0, "a non-dust-recipient address must never receive a bonus");
                }
            }
            prop_assert!(
                addresses_with_bonus <= 1,
                "dust must never be paid to more than one address"
            );
        }
    }
}

#[test]
fn test_initialize_rejects_anti_snipe_hard_max_over_max() {
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
        MAX_ANTI_SNIPE_HARD_MAX_SECS + 1,
        DEFAULT_REVEAL_SECS,
        DEFAULT_MAX_POSITION,
        DEFAULT_MAX_TOTAL_WEIGHT,
    );
    assert_eq!(result, Err(Ok(Error::InvalidAntiSnipeParams)));
}

#[test]
fn test_initialize_accepts_anti_snipe_hard_max_at_max() {
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
        MAX_ANTI_SNIPE_HARD_MAX_SECS,
        DEFAULT_REVEAL_SECS,
        DEFAULT_MAX_POSITION,
        DEFAULT_MAX_TOTAL_WEIGHT,
    );
    assert_eq!(result, Ok(Ok(())));
}


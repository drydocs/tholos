#![cfg(test)]

use super::*;
use soroban_sdk::testutils::Address as _;

const DEFAULT_BOND: i128 = 100;
const DEFAULT_REGISTRATION_SECS: u64 = 3600;
const DEFAULT_ANTI_SNIPE_EXT_SECS: u64 = 300;
const DEFAULT_ANTI_SNIPE_HARD_MAX_SECS: u64 = 1800;
const DEFAULT_REVEAL_SECS: u64 = 3600;
const DEFAULT_MAX_POSITION: i128 = 1_000_000;
const DEFAULT_MAX_TOTAL_WEIGHT: i128 = 10_000_000;

/// A ready-to-use, already-initialized TholosV2 instance with default
/// parameters. Tests that need an *uninitialized* contract, or non-default
/// init parameters, call `initialize` directly instead.
struct Fixture {
    env: Env,
    contract_id: Address,
    admin: Address,
    token: Address,
}

impl Fixture {
    fn new() -> Self {
        let env = Env::default();
        env.mock_all_auths();

        let contract_id = env.register(TholosV2, ());
        let admin = Address::generate(&env);
        let token = Address::generate(&env);

        env.as_contract(&contract_id, || {
            TholosV2::initialize(
                env.clone(),
                admin.clone(),
                token.clone(),
                DEFAULT_BOND,
                DEFAULT_REGISTRATION_SECS,
                DEFAULT_ANTI_SNIPE_EXT_SECS,
                DEFAULT_ANTI_SNIPE_HARD_MAX_SECS,
                DEFAULT_REVEAL_SECS,
                DEFAULT_MAX_POSITION,
                DEFAULT_MAX_TOTAL_WEIGHT,
            )
            .unwrap();
        });

        Fixture {
            env,
            contract_id,
            admin,
            token,
        }
    }

    fn generate(&self) -> Address {
        Address::generate(&self.env)
    }
}

#[test]
fn test_initialize_pins_expected_policy() {
    let f = Fixture::new();

    let policy = f.env.as_contract(&f.contract_id, || {
        TholosV2::get_policy(f.env.clone()).unwrap()
    });

    assert_eq!(policy.token, f.token);
    assert_eq!(policy.base_bond, DEFAULT_BOND);
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

    let result = f.env.as_contract(&f.contract_id, || {
        TholosV2::initialize(
            f.env.clone(),
            f.admin.clone(),
            f.token.clone(),
            DEFAULT_BOND,
            DEFAULT_REGISTRATION_SECS,
            DEFAULT_ANTI_SNIPE_EXT_SECS,
            DEFAULT_ANTI_SNIPE_HARD_MAX_SECS,
            DEFAULT_REVEAL_SECS,
            DEFAULT_MAX_POSITION,
            DEFAULT_MAX_TOTAL_WEIGHT,
        )
    });

    assert_eq!(result, Err(Error::AlreadyInitialized));
}

#[test]
fn test_get_policy_before_initialize_fails() {
    let env = Env::default();
    let contract_id = env.register(TholosV2, ());

    let result = env.as_contract(&contract_id, || TholosV2::get_policy(env.clone()));

    assert_eq!(result, Err(Error::NotInitialized));
}

fn init_with_bond(
    env: &Env,
    contract_id: &Address,
    admin: &Address,
    token: &Address,
    bond: i128,
) -> Result<(), Error> {
    env.as_contract(contract_id, || {
        TholosV2::initialize(
            env.clone(),
            admin.clone(),
            token.clone(),
            bond,
            DEFAULT_REGISTRATION_SECS,
            DEFAULT_ANTI_SNIPE_EXT_SECS,
            DEFAULT_ANTI_SNIPE_HARD_MAX_SECS,
            DEFAULT_REVEAL_SECS,
            DEFAULT_MAX_POSITION,
            DEFAULT_MAX_TOTAL_WEIGHT,
        )
    })
}

#[test]
fn test_initialize_rejects_zero_bond() {
    let env = Env::default();
    env.mock_all_auths();
    let contract_id = env.register(TholosV2, ());
    let admin = Address::generate(&env);
    let token = Address::generate(&env);

    let result = init_with_bond(&env, &contract_id, &admin, &token, 0);
    assert_eq!(result, Err(Error::InvalidBondAmount));
}

#[test]
fn test_initialize_rejects_negative_bond() {
    let env = Env::default();
    env.mock_all_auths();
    let contract_id = env.register(TholosV2, ());
    let admin = Address::generate(&env);
    let token = Address::generate(&env);

    let result = init_with_bond(&env, &contract_id, &admin, &token, -1);
    assert_eq!(result, Err(Error::InvalidBondAmount));
}

#[test]
fn test_initialize_rejects_bond_over_max() {
    let env = Env::default();
    env.mock_all_auths();
    let contract_id = env.register(TholosV2, ());
    let admin = Address::generate(&env);
    let token = Address::generate(&env);

    let result = init_with_bond(&env, &contract_id, &admin, &token, MAX_BOND_AMOUNT + 1);
    assert_eq!(result, Err(Error::InvalidBondAmount));
}

#[test]
fn test_initialize_accepts_bond_at_max() {
    let env = Env::default();
    env.mock_all_auths();
    let contract_id = env.register(TholosV2, ());
    let admin = Address::generate(&env);
    let token = Address::generate(&env);

    let result = init_with_bond(&env, &contract_id, &admin, &token, MAX_BOND_AMOUNT);
    assert_eq!(result, Ok(()));
}

#[test]
fn test_initialize_rejects_zero_registration_duration() {
    let env = Env::default();
    env.mock_all_auths();
    let contract_id = env.register(TholosV2, ());
    let admin = Address::generate(&env);
    let token = Address::generate(&env);

    let result = env.as_contract(&contract_id, || {
        TholosV2::initialize(
            env.clone(),
            admin.clone(),
            token.clone(),
            DEFAULT_BOND,
            0,
            DEFAULT_ANTI_SNIPE_EXT_SECS,
            DEFAULT_ANTI_SNIPE_HARD_MAX_SECS,
            DEFAULT_REVEAL_SECS,
            DEFAULT_MAX_POSITION,
            DEFAULT_MAX_TOTAL_WEIGHT,
        )
    });

    assert_eq!(result, Err(Error::InvalidRegistrationDuration));
}

#[test]
fn test_initialize_rejects_registration_duration_over_max() {
    let env = Env::default();
    env.mock_all_auths();
    let contract_id = env.register(TholosV2, ());
    let admin = Address::generate(&env);
    let token = Address::generate(&env);

    let result = env.as_contract(&contract_id, || {
        TholosV2::initialize(
            env.clone(),
            admin.clone(),
            token.clone(),
            DEFAULT_BOND,
            MAX_REGISTRATION_DURATION_SECS + 1,
            DEFAULT_ANTI_SNIPE_EXT_SECS,
            DEFAULT_ANTI_SNIPE_HARD_MAX_SECS,
            DEFAULT_REVEAL_SECS,
            DEFAULT_MAX_POSITION,
            DEFAULT_MAX_TOTAL_WEIGHT,
        )
    });

    assert_eq!(result, Err(Error::InvalidRegistrationDuration));
}

#[test]
fn test_initialize_rejects_zero_reveal_duration() {
    let env = Env::default();
    env.mock_all_auths();
    let contract_id = env.register(TholosV2, ());
    let admin = Address::generate(&env);
    let token = Address::generate(&env);

    let result = env.as_contract(&contract_id, || {
        TholosV2::initialize(
            env.clone(),
            admin.clone(),
            token.clone(),
            DEFAULT_BOND,
            DEFAULT_REGISTRATION_SECS,
            DEFAULT_ANTI_SNIPE_EXT_SECS,
            DEFAULT_ANTI_SNIPE_HARD_MAX_SECS,
            0,
            DEFAULT_MAX_POSITION,
            DEFAULT_MAX_TOTAL_WEIGHT,
        )
    });

    assert_eq!(result, Err(Error::InvalidRevealDuration));
}

#[test]
fn test_initialize_rejects_anti_snipe_extension_over_hard_max() {
    let env = Env::default();
    env.mock_all_auths();
    let contract_id = env.register(TholosV2, ());
    let admin = Address::generate(&env);
    let token = Address::generate(&env);

    let result = env.as_contract(&contract_id, || {
        TholosV2::initialize(
            env.clone(),
            admin.clone(),
            token.clone(),
            DEFAULT_BOND,
            DEFAULT_REGISTRATION_SECS,
            // Extension bigger than its own hard max: a single qualifying
            // deposit could blow past the deployment's stated cap in one step.
            2000,
            1800,
            DEFAULT_REVEAL_SECS,
            DEFAULT_MAX_POSITION,
            DEFAULT_MAX_TOTAL_WEIGHT,
        )
    });

    assert_eq!(result, Err(Error::InvalidAntiSnipeParams));
}

#[test]
fn test_initialize_accepts_anti_snipe_extension_equal_to_hard_max() {
    let env = Env::default();
    env.mock_all_auths();
    let contract_id = env.register(TholosV2, ());
    let admin = Address::generate(&env);
    let token = Address::generate(&env);

    let result = env.as_contract(&contract_id, || {
        TholosV2::initialize(
            env.clone(),
            admin.clone(),
            token.clone(),
            DEFAULT_BOND,
            DEFAULT_REGISTRATION_SECS,
            1800,
            1800,
            DEFAULT_REVEAL_SECS,
            DEFAULT_MAX_POSITION,
            DEFAULT_MAX_TOTAL_WEIGHT,
        )
    });

    assert_eq!(result, Ok(()));
}

#[test]
fn test_initialize_rejects_max_position_over_max_total_weight() {
    let env = Env::default();
    env.mock_all_auths();
    let contract_id = env.register(TholosV2, ());
    let admin = Address::generate(&env);
    let token = Address::generate(&env);

    let result = env.as_contract(&contract_id, || {
        TholosV2::initialize(
            env.clone(),
            admin.clone(),
            token.clone(),
            DEFAULT_BOND,
            DEFAULT_REGISTRATION_SECS,
            DEFAULT_ANTI_SNIPE_EXT_SECS,
            DEFAULT_ANTI_SNIPE_HARD_MAX_SECS,
            DEFAULT_REVEAL_SECS,
            // A single position that could exceed the entire frozen total
            // it's supposedly part of is nonsensical.
            DEFAULT_MAX_TOTAL_WEIGHT + 1,
            DEFAULT_MAX_TOTAL_WEIGHT,
        )
    });

    assert_eq!(result, Err(Error::InvalidMaxPosition));
}

#[test]
fn test_initialize_rejects_zero_max_position() {
    let env = Env::default();
    env.mock_all_auths();
    let contract_id = env.register(TholosV2, ());
    let admin = Address::generate(&env);
    let token = Address::generate(&env);

    let result = env.as_contract(&contract_id, || {
        TholosV2::initialize(
            env.clone(),
            admin.clone(),
            token.clone(),
            DEFAULT_BOND,
            DEFAULT_REGISTRATION_SECS,
            DEFAULT_ANTI_SNIPE_EXT_SECS,
            DEFAULT_ANTI_SNIPE_HARD_MAX_SECS,
            DEFAULT_REVEAL_SECS,
            0,
            DEFAULT_MAX_TOTAL_WEIGHT,
        )
    });

    assert_eq!(result, Err(Error::InvalidMaxPosition));
}

#[test]
fn test_initialize_rejects_zero_max_total_weight() {
    let env = Env::default();
    env.mock_all_auths();
    let contract_id = env.register(TholosV2, ());
    let admin = Address::generate(&env);
    let token = Address::generate(&env);

    let result = env.as_contract(&contract_id, || {
        TholosV2::initialize(
            env.clone(),
            admin.clone(),
            token.clone(),
            DEFAULT_BOND,
            DEFAULT_REGISTRATION_SECS,
            DEFAULT_ANTI_SNIPE_EXT_SECS,
            DEFAULT_ANTI_SNIPE_HARD_MAX_SECS,
            DEFAULT_REVEAL_SECS,
            DEFAULT_MAX_POSITION,
            0,
        )
    });

    assert_eq!(result, Err(Error::InvalidMaxTotalWeight));
}

#[test]
fn test_initialize_rejects_max_total_weight_over_max_bond() {
    let env = Env::default();
    env.mock_all_auths();
    let contract_id = env.register(TholosV2, ());
    let admin = Address::generate(&env);
    let token = Address::generate(&env);

    let result = env.as_contract(&contract_id, || {
        TholosV2::initialize(
            env.clone(),
            admin.clone(),
            token.clone(),
            DEFAULT_BOND,
            DEFAULT_REGISTRATION_SECS,
            DEFAULT_ANTI_SNIPE_EXT_SECS,
            DEFAULT_ANTI_SNIPE_HARD_MAX_SECS,
            DEFAULT_REVEAL_SECS,
            MAX_BOND_AMOUNT,
            MAX_BOND_AMOUNT + 1,
        )
    });

    assert_eq!(result, Err(Error::InvalidMaxTotalWeight));
}

#[test]
fn test_create_pending_assertion_pins_policy_and_stores_assertion() {
    let f = Fixture::new();
    let asserter = f.generate();

    let id = f.env.as_contract(&f.contract_id, || {
        TholosV2::create_pending_assertion(&f.env, asserter.clone(), true).unwrap()
    });
    assert_eq!(id, 0);

    let assertion = f.env.as_contract(&f.contract_id, || {
        TholosV2::get_assertion(f.env.clone(), id).unwrap()
    });

    assert_eq!(assertion.id, 0);
    assert_eq!(assertion.asserter, asserter);
    assert_eq!(assertion.disputer, None);
    assert!(assertion.outcome);
    assert_eq!(assertion.phase, PhaseV2::Pending);
    assert_eq!(assertion.policy.base_bond, DEFAULT_BOND);
    assert_eq!(assertion.terminal_cause, TerminalCause::NotYetDecided);
    assert_eq!(assertion.final_outcome, None);
}

#[test]
fn test_create_pending_assertion_ids_increment() {
    let f = Fixture::new();
    let asserter = f.generate();

    let (first, second) = f.env.as_contract(&f.contract_id, || {
        let first = TholosV2::create_pending_assertion(&f.env, asserter.clone(), true).unwrap();
        let second = TholosV2::create_pending_assertion(&f.env, asserter.clone(), false).unwrap();
        (first, second)
    });

    assert_eq!(first, 0);
    assert_eq!(second, 1);
}

#[test]
fn test_create_pending_assertion_before_initialize_fails() {
    let env = Env::default();
    env.mock_all_auths();
    let contract_id = env.register(TholosV2, ());
    let asserter = Address::generate(&env);

    let result = env.as_contract(&contract_id, || {
        TholosV2::create_pending_assertion(&env, asserter, true)
    });

    assert_eq!(result, Err(Error::NotInitialized));
}

#[test]
fn test_get_assertion_not_found() {
    let f = Fixture::new();

    let result = f
        .env
        .as_contract(&f.contract_id, || TholosV2::get_assertion(f.env.clone(), 0));

    assert_eq!(result, Err(Error::AssertionNotFound));
}

#[test]
fn test_policy_hash_is_deterministic_for_identical_policy() {
    let f = Fixture::new();
    let asserter = f.generate();

    // Two assertions created under the same unchanged deployment policy must
    // hash to the same value: the hash is a function of policy content, not
    // of which assertion it's attached to.
    let (hash_a, hash_b) = f.env.as_contract(&f.contract_id, || {
        let id_a = TholosV2::create_pending_assertion(&f.env, asserter.clone(), true).unwrap();
        let id_b = TholosV2::create_pending_assertion(&f.env, asserter.clone(), false).unwrap();
        let a = TholosV2::get_assertion(f.env.clone(), id_a).unwrap();
        let b = TholosV2::get_assertion(f.env.clone(), id_b).unwrap();
        (a.policy_hash, b.policy_hash)
    });

    assert_eq!(hash_a, hash_b);
}

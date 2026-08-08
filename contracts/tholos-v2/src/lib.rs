#![no_std]

//! Tholos v2: stake-weighted resolution. Design in `docs/src/V2_RESOLUTION.md`.
//! Deployed as a wholly separate contract from v1 (`contracts/tholos`), never
//! upgraded in place: see the design doc's "Migration from existing v1
//! deployments" section for why.
//!
//! This crate currently implements only #64's scope: the immutable
//! `PolicySnapshotV2` pinned at assertion creation, and the `AssertionV2`
//! record it lives on. Registration, reveal, outcome resolution, settlement,
//! and the freeze/cancel mechanism are separate issues (#65-#71) and land as
//! this crate grows.

use soroban_sdk::{
    contract, contracterror, contractimpl, contracttype, xdr::ToXdr, Address, BytesN, Env,
};

/// Which weight rule this assertion's vote is decided under. A version marker
/// rather than a formula, so a future weight rule can be added without
/// reinterpreting already-pinned assertions under new math.
#[contracttype]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WeightRuleVersion {
    /// `weight(address) = locked_bond(address)`, linear and unweighted by
    /// anything but the amount actually escrowed. See "Voting weight" in
    /// V2_RESOLUTION.md for why linear weight is deliberate.
    LinearStakeV1,
}

/// What happens if neither side reaches strict majority by the reveal
/// deadline. `AssertedOutcomeStands` is the only variant today (the
/// optimistic-default design decision), kept as an enum so a future
/// `Inconclusive`-style rule could be pinned per-deployment without touching
/// already-open assertions under the old rule.
#[contracttype]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TimeoutDefaultRule {
    AssertedOutcomeStands,
}

/// Version marker for the settlement/payout formula (forfeiture distribution,
/// dust handling). Bumped independently of `WeightRuleVersion` since the two
/// can change on different schedules.
#[contracttype]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PayoutRuleVersion {
    ProRataV1,
}

/// The lifecycle phase of an `AssertionV2`. `OutcomeLocked` (a strict majority
/// reached before the reveal deadline) is folded into `Reveal` for now:
/// distinguishing "revealing, outcome undecided" from "revealing, outcome
/// already locked" only matters once settlement exists, which is #69's scope.
#[contracttype]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PhaseV2 {
    Pending,
    Registration,
    Reveal,
    Resolved,
}

/// Why an assertion reached `Resolved`. Distinguishes a real vote outcome
/// from the optimistic default and from an admin cancellation, so history and
/// indexers never have to infer which rule actually decided the result.
///
/// `NotYetDecided` stands in for `None`: the soroban-sdk 26.1.0 `contracttype`
/// derive doesn't generate an XDR `ScVal` conversion for `Option<EnumType>`
/// (only `Option` of built-in types like `Address`/`bool`), so `AssertionV2`
/// deriving `contracttype` fails to compile if `terminal_cause` is
/// `Option<TerminalCause>`. Same information, no SDK limitation.
#[contracttype]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TerminalCause {
    NotYetDecided,
    StrictMajorityFor,
    StrictMajorityAgainst,
    OptimisticTimeout,
    /// Set only by the freeze/cancel mechanism (#71), before any position
    /// reveals. Reserved here so `AssertionV2.terminal_cause`'s type doesn't
    /// need to change when #71 lands.
    AdminCancelled,
}

/// Pinned in full onto every `AssertionV2` at creation time, never mutated
/// afterward. Deployment-wide parameter changes (via a future `initialize`-
/// adjacent admin call) only affect assertions created after the change; an
/// already-open assertion always executes under the snapshot it was created
/// with. See "Policy is pinned when the assertion opens" in
/// V2_RESOLUTION.md.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PolicySnapshotV2 {
    pub token: Address,
    pub base_bond: i128,
    /// Always equal to `base_bond`: see the rationale on `initialize` below.
    pub min_resolution_bond: i128,
    pub registration_duration_secs: u64,
    /// A qualifying late deposit pushes the soft registration cutoff out by
    /// this much, never past `anti_snipe_hard_max_secs` total extension.
    pub anti_snipe_extension_secs: u64,
    pub anti_snipe_hard_max_secs: u64,
    pub reveal_duration_secs: u64,
    pub weight_rule: WeightRuleVersion,
    pub timeout_default: TimeoutDefaultRule,
    pub payout_rule: PayoutRuleVersion,
    /// Upper bound on any single position's size, checked at deposit time so
    /// settlement's forfeiture-distribution arithmetic (#69) can't overflow.
    pub max_position: i128,
    /// Upper bound on the frozen eligible total `W`, for the same reason.
    pub max_total_weight: i128,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AssertionV2 {
    pub id: u64,
    pub asserter: Address,
    /// Set once `dispute` (#66) opens registration. `None` while `Pending`.
    pub disputer: Option<Address>,
    pub outcome: bool,
    pub phase: PhaseV2,
    pub policy: PolicySnapshotV2,
    /// Hash of `policy`'s canonical encoding, computed once at creation and
    /// stored alongside it. Lets a client or auditor confirm which exact
    /// policy an assertion is bound to without re-deriving the encoding
    /// themselves.
    pub policy_hash: BytesN<32>,
    /// `TerminalCause::NotYetDecided` until `phase == Resolved`.
    pub terminal_cause: TerminalCause,
    /// The authoritative resolved outcome. `None` until `phase == Resolved`.
    /// Stored directly (not only in an event), unlike v1's `Assertion.outcome`
    /// which always stays the original claim even after a dispute overturns
    /// it, a sharp edge v1 needed a separate `final_outcome` field to fix.
    pub final_outcome: Option<bool>,
}

#[contracttype]
pub enum DataKey {
    Admin,
    Policy,
    NextId,
    AssertionV2(u64),
}

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
pub enum Error {
    AlreadyInitialized = 1,
    NotInitialized = 2,
    AssertionNotFound = 3,
    /// `base_bond` was not positive, or exceeded the interim safety bound
    /// below.
    InvalidBondAmount = 4,
    InvalidRegistrationDuration = 5,
    InvalidRevealDuration = 6,
    /// `anti_snipe_extension_secs` exceeded `anti_snipe_hard_max_secs`, which
    /// would let a single qualifying deposit blow past the deployment's own
    /// stated hard cap in one step.
    InvalidAntiSnipeParams = 7,
    InvalidMaxPosition = 8,
    InvalidMaxTotalWeight = 9,
}

const DAY_IN_LEDGERS: u32 = 17280;
const INSTANCE_BUMP_AMOUNT: u32 = 30 * DAY_IN_LEDGERS;
const INSTANCE_LIFETIME_THRESHOLD: u32 = INSTANCE_BUMP_AMOUNT - DAY_IN_LEDGERS;

const MAX_REGISTRATION_DURATION_SECS: u64 = 7 * 24 * 60 * 60;
const MAX_REVEAL_DURATION_SECS: u64 = 7 * 24 * 60 * 60;

/// Interim safety bound on `base_bond`, reused from v1's `MAX_BOND_AMOUNT`
/// derivation (i128::MAX bounded by the tighter of the dispute-balance-sum
/// and a reward-multiply overflow). V2's actual settlement formula
/// (`s_i * forfeited_pool / recipient_weight`, see #69) has different
/// overflow characteristics and needs its own derivation once that
/// arithmetic exists; this bound is deliberately conservative until then,
/// not a final answer.
const MAX_BOND_AMOUNT: i128 = i128::MAX / 1_000;

#[contract]
pub struct TholosV2;

#[contractimpl]
impl TholosV2 {
    /// One-time setup, pinning the deployment-wide defaults every future
    /// assertion's `PolicySnapshotV2` is built from. Requires `admin`'s
    /// signature. Fails with `AlreadyInitialized` if called twice.
    #[allow(clippy::too_many_arguments)]
    pub fn initialize(
        env: Env,
        admin: Address,
        token: Address,
        base_bond: i128,
        registration_duration_secs: u64,
        anti_snipe_extension_secs: u64,
        anti_snipe_hard_max_secs: u64,
        reveal_duration_secs: u64,
        max_position: i128,
        max_total_weight: i128,
    ) -> Result<(), Error> {
        admin.require_auth();

        if env.storage().instance().has(&DataKey::Admin) {
            return Err(Error::AlreadyInitialized);
        }

        if base_bond <= 0 || base_bond > MAX_BOND_AMOUNT {
            return Err(Error::InvalidBondAmount);
        }
        if registration_duration_secs == 0
            || registration_duration_secs > MAX_REGISTRATION_DURATION_SECS
        {
            return Err(Error::InvalidRegistrationDuration);
        }
        if reveal_duration_secs == 0 || reveal_duration_secs > MAX_REVEAL_DURATION_SECS {
            return Err(Error::InvalidRevealDuration);
        }
        if anti_snipe_extension_secs > anti_snipe_hard_max_secs {
            return Err(Error::InvalidAntiSnipeParams);
        }
        // max_total_weight's own bounds must be checked before comparing
        // max_position against it, otherwise a max_total_weight <= 0 always
        // trips the max_position check first (no positive max_position can
        // ever be <= a non-positive total), making InvalidMaxTotalWeight's
        // <= 0 case unreachable.
        if max_total_weight <= 0 || max_total_weight > MAX_BOND_AMOUNT {
            return Err(Error::InvalidMaxTotalWeight);
        }
        // A position can't usefully exceed the frozen total it's part of.
        if max_position <= 0 || max_position > max_total_weight {
            return Err(Error::InvalidMaxPosition);
        }

        let policy = PolicySnapshotV2 {
            token,
            base_bond,
            // Equal to base_bond by design: a cheaper minimum would let a
            // third party break an asserter/disputer tie for a fraction of
            // what the original two parties risked. See #64/#66.
            min_resolution_bond: base_bond,
            registration_duration_secs,
            anti_snipe_extension_secs,
            anti_snipe_hard_max_secs,
            reveal_duration_secs,
            weight_rule: WeightRuleVersion::LinearStakeV1,
            timeout_default: TimeoutDefaultRule::AssertedOutcomeStands,
            payout_rule: PayoutRuleVersion::ProRataV1,
            max_position,
            max_total_weight,
        };

        env.storage().instance().set(&DataKey::Admin, &admin);
        env.storage().instance().set(&DataKey::Policy, &policy);
        env.storage().instance().set(&DataKey::NextId, &0u64);
        env.storage()
            .instance()
            .extend_ttl(INSTANCE_LIFETIME_THRESHOLD, INSTANCE_BUMP_AMOUNT);

        Ok(())
    }

    /// Read-only lookup of the deployment-wide policy defaults new assertions
    /// are currently pinned from. Fails with `NotInitialized` before
    /// `initialize`.
    pub fn get_policy(env: Env) -> Result<PolicySnapshotV2, Error> {
        env.storage()
            .instance()
            .get(&DataKey::Policy)
            .ok_or(Error::NotInitialized)
    }

    /// Read-only lookup of one assertion. Fails with `AssertionNotFound` if
    /// the id doesn't exist.
    pub fn get_assertion(env: Env, id: u64) -> Result<AssertionV2, Error> {
        env.storage()
            .persistent()
            .get(&DataKey::AssertionV2(id))
            .ok_or(Error::AssertionNotFound)
    }

    /// Builds and stores a `Pending` `AssertionV2` with a freshly pinned
    /// policy snapshot, without moving any tokens or requiring the
    /// asserter's bond. Exists so this issue's policy-pinning behavior is
    /// independently testable before #65 wires up the real, bond-transferring
    /// `assert_outcome` entrypoint on top of this same helper.
    ///
    /// Not called from any `#[contractimpl]` fn yet, only from tests, hence
    /// `allow(dead_code)`. #65 removes this allow when it adds the public
    /// entrypoint that calls it for real.
    #[allow(dead_code)]
    fn create_pending_assertion(env: &Env, asserter: Address, outcome: bool) -> Result<u64, Error> {
        let policy: PolicySnapshotV2 = env
            .storage()
            .instance()
            .get(&DataKey::Policy)
            .ok_or(Error::NotInitialized)?;

        let id: u64 = env
            .storage()
            .instance()
            .get(&DataKey::NextId)
            .ok_or(Error::NotInitialized)?;
        env.storage().instance().set(&DataKey::NextId, &(id + 1));

        let policy_hash = env.crypto().sha256(&policy.clone().to_xdr(env)).into();

        let assertion = AssertionV2 {
            id,
            asserter,
            disputer: None,
            outcome,
            phase: PhaseV2::Pending,
            policy,
            policy_hash,
            terminal_cause: TerminalCause::NotYetDecided,
            final_outcome: None,
        };

        env.storage()
            .persistent()
            .set(&DataKey::AssertionV2(id), &assertion);
        env.storage().persistent().extend_ttl(
            &DataKey::AssertionV2(id),
            INSTANCE_LIFETIME_THRESHOLD,
            INSTANCE_BUMP_AMOUNT,
        );

        Ok(id)
    }
}

mod test;

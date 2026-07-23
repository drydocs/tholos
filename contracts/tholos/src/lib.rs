#![no_std]

use soroban_sdk::{
    contract, contracterror, contractevent, contractimpl, contracttype, token, Address, Env, Vec,
};

#[contractevent]
pub struct Asserted {
    #[topic]
    pub id: u64,
    pub asserter: Address,
    pub outcome: bool,
}

#[contractevent]
pub struct Disputed {
    #[topic]
    pub id: u64,
    pub disputer: Address,
}

#[contractevent]
pub struct Finalized {
    #[topic]
    pub id: u64,
    pub outcome: bool,
    /// Who called `finalize`. Always a verified address — `finalize` requires
    /// the caller's auth unconditionally, so this value is trustworthy
    /// regardless of whether a reward was configured.
    pub finalizer: Address,
    /// How many tokens were paid to the finalizer as a reward (0 when
    /// `finalize_reward_bps` was configured as 0).
    pub reward: i128,
}

#[contractevent]
pub struct Resolved {
    #[topic]
    pub id: u64,
    pub outcome: bool,
}

#[contractevent]
pub struct ResolversUpdated {
    pub resolvers: Vec<Address>,
}

#[contractevent]
pub struct PauseUpdated {
    pub paused: bool,
}

#[contractevent]
pub struct RotationProposed {
    pub old_resolver: Address,
    pub new_resolver: Address,
    pub proposed_by: Address,
}

#[contractevent]
pub struct RotationExecuted {
    pub old_resolver: Address,
    pub new_resolver: Address,
}

#[contractevent]
pub struct RotationCancelled {
    pub old_resolver: Address,
    pub new_resolver: Address,
}

/// An in-flight single-slot committee rotation proposed by a current resolver.
/// Decided by a strict majority of the live committee via `vote_rotation`. Only
/// one may be open at a time. See `docs/src/ROTATION_DESIGN.md`.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RotationProposal {
    /// The current resolver to remove. Must be on the committee when proposed.
    pub old_resolver: Address,
    /// The new resolver to add. Must not already be on the committee.
    pub new_resolver: Address,
    /// The resolver who opened the proposal.
    pub proposed_by: Address,
    /// Resolvers who voted yes, to prevent double-voting.
    pub yes: Vec<Address>,
    /// Resolvers who voted no, to prevent double-voting and detect deadlock.
    pub no: Vec<Address>,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Status {
    Pending,
    Disputed,
    Resolved,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Assertion {
    pub asserter: Address,
    pub outcome: bool,
    pub bond: i128,
    pub opened_at: u64,
    pub status: Status,
    pub disputer: Option<Address>,
    pub votes_for_outcome: u32,
    pub votes_against_outcome: u32,
    pub voted: Vec<Address>,
    /// The resolver committee at the moment this assertion was disputed.
    /// Empty until `dispute` is called. Voting and majority are always
    /// computed against this snapshot, not the live committee, so an
    /// `update_resolvers` call mid-dispute can't change who gets to decide
    /// an already-disputed assertion.
    pub resolvers: Vec<Address>,
    /// Who called `finalize`. `None` until the assertion is finalized via
    /// `finalize` (never set for assertions resolved via `resolve`). Always
    /// `Some` after `finalize` completes — the caller must authorize the call
    /// unconditionally, so this is always a verified address.
    pub finalizer: Option<Address>,
}

#[contracttype]
pub enum DataKey {
    Admin,
    Token,
    BondAmount,
    ChallengeWindow,
    Resolvers,
    Assertion(u64),
    NextId,
    Paused,
    /// Basis points (0–1000) of the bond paid to whoever calls `finalize` as
    /// an incentive for prompt finalization. 0 means no reward is taken; the
    /// full bond is returned to the asserter (original behavior).
    FinalizeRewardBps,
    RotationProposal,
}

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
pub enum Error {
    AlreadyInitialized = 1,
    NotInitialized = 2,
    InvalidResolverCount = 3,
    AssertionNotFound = 4,
    NotPending = 5,
    NotDisputed = 6,
    ChallengeWindowClosed = 7,
    ChallengeWindowOpen = 8,
    NotAResolver = 9,
    AlreadyVoted = 10,
    Paused = 11,
    /// `bond_amount` was not positive, or exceeded `MAX_BOND_AMOUNT`.
    InvalidBondAmount = 12,
    InvalidChallengeWindow = 13,
    TooManyResolvers = 14,
    /// `finalize_reward_bps` was greater than `MAX_FINALIZE_REWARD_BPS` (1000).
    InvalidFinalizeReward = 15,
    DuplicateResolvers = 16,
    RotationInProgress = 17,
    NoRotationProposal = 18,
    ResolverNotInCommittee = 19,
    DuplicateResolver = 20,
    NotProposer = 21,
}

const DAY_IN_LEDGERS: u32 = 17280;
const INSTANCE_BUMP_AMOUNT: u32 = 30 * DAY_IN_LEDGERS;
const INSTANCE_LIFETIME_THRESHOLD: u32 = INSTANCE_BUMP_AMOUNT - DAY_IN_LEDGERS;

/// Persistent `Assertion` entries get the same 30-day TTL bump as instance
/// storage, applied every time an assertion is written. A `challenge_window_secs`
/// of at most `MAX_CHALLENGE_WINDOW_SECS` (7 days) leaves comfortable headroom
/// within that 30-day bump for the window to elapse and for `finalize`,
/// `dispute`, or a resolver's `resolve` to actually be called afterward.
const ASSERTION_BUMP_AMOUNT: u32 = 30 * DAY_IN_LEDGERS;
const ASSERTION_LIFETIME_THRESHOLD: u32 = ASSERTION_BUMP_AMOUNT - DAY_IN_LEDGERS;
const MAX_CHALLENGE_WINDOW_SECS: u64 = 7 * 24 * 60 * 60;

/// A resolver committee larger than this gets copied in full onto every
/// disputed assertion (see `Assertion.resolvers`), so an unbounded size
/// would grow the storage and iteration cost of every future dispute.
const MAX_RESOLVERS: u32 = 21;

/// The reward is expressed in basis points of the bond. Capping at 1000 bps
/// (10 %) keeps the incentive meaningful without allowing a deployment to
/// accidentally haircut the asserter's bond by more than a tenth.
pub const MAX_FINALIZE_REWARD_BPS: u32 = 1_000;

/// `bond_amount` is bounded by the tighter of two independent overflow
/// constraints:
///
/// 1. **Dispute-balance-sum.** The asserter's and disputer's bonds (each
///    `bond_amount`) both land in the contract's token balance across
///    `assert_outcome` and `dispute`. The SAC token panics with a balance
///    overflow inside `receive_balance` once that sum exceeds `i128::MAX`,
///    so `2 * bond_amount` must stay in range.
/// 2. **Finalize reward-multiply.** `finalize` computes the caller's
///    reward as `assertion.bond * (reward_bps as i128) / 10_000` — the
///    multiply happens *before* the divide, so `bond_amount *
///    MAX_FINALIZE_REWARD_BPS` must independently stay in range, for any
///    `reward_bps` up to `MAX_FINALIZE_REWARD_BPS`.
///
/// `MAX_FINALIZE_REWARD_BPS` (1000) is greater than the `2` from the first
/// constraint, so the reward-multiply constraint is tighter and is what
/// currently binds: `i128::MAX / MAX_FINALIZE_REWARD_BPS` is ~500x smaller
/// than `i128::MAX / 2`. Deriving `MAX_BOND_AMOUNT` as the minimum of both
/// keeps this correct automatically if `MAX_FINALIZE_REWARD_BPS` — or a
/// future divisor introduced elsewhere — ever changes.
const MAX_BOND_AMOUNT: i128 = {
    let dispute_balance_sum_bound = i128::MAX / 2;
    let reward_multiply_bound = i128::MAX / (MAX_FINALIZE_REWARD_BPS as i128);
    if dispute_balance_sum_bound < reward_multiply_bound {
        dispute_balance_sum_bound
    } else {
        reward_multiply_bound
    }
};

// Compile-time guard: if a future change to either constant ever makes
// `MAX_BOND_AMOUNT * MAX_FINALIZE_REWARD_BPS` overflow again, fail the build
// instead of silently reintroducing the finalize reward-multiply overflow.
const _: () = assert!(MAX_BOND_AMOUNT
    .checked_mul(MAX_FINALIZE_REWARD_BPS as i128)
    .is_some());

#[contract]
pub struct Tholos;

#[contractimpl]
impl Tholos {
    /// Initializes the contract. `resolvers` must have an odd length so a
    /// simple majority vote can never tie. `finalize_reward_bps` sets the
    /// fraction of the bond (in basis points, 0–1000) paid to whoever calls
    /// `finalize` as an incentive for prompt finalization; 0 disables the
    /// reward entirely and preserves the original behavior where the full
    /// bond is returned to the asserter.
    pub fn initialize(
        env: Env,
        admin: Address,
        token: Address,
        bond_amount: i128,
        challenge_window_secs: u64,
        resolvers: Vec<Address>,
        finalize_reward_bps: u32,
    ) -> Result<(), Error> {
        if env.storage().instance().has(&DataKey::Admin) {
            return Err(Error::AlreadyInitialized);
        }
        if resolvers.is_empty() || resolvers.len().is_multiple_of(2) {
            return Err(Error::InvalidResolverCount);
        }
        if resolvers.len() > MAX_RESOLVERS {
            return Err(Error::TooManyResolvers);
        }
        Self::assert_unique_resolvers(&resolvers)?;
        if bond_amount <= 0 || bond_amount > MAX_BOND_AMOUNT {
            return Err(Error::InvalidBondAmount);
        }
        if challenge_window_secs == 0 || challenge_window_secs > MAX_CHALLENGE_WINDOW_SECS {
            return Err(Error::InvalidChallengeWindow);
        }
        if finalize_reward_bps > MAX_FINALIZE_REWARD_BPS {
            return Err(Error::InvalidFinalizeReward);
        }

        admin.require_auth();

        env.storage().instance().set(&DataKey::Admin, &admin);
        env.storage().instance().set(&DataKey::Token, &token);
        env.storage()
            .instance()
            .set(&DataKey::BondAmount, &bond_amount);
        env.storage()
            .instance()
            .set(&DataKey::ChallengeWindow, &challenge_window_secs);
        env.storage()
            .instance()
            .set(&DataKey::Resolvers, &resolvers);
        env.storage().instance().set(&DataKey::NextId, &0u64);
        env.storage().instance().set(&DataKey::Paused, &false);
        env.storage()
            .instance()
            .set(&DataKey::FinalizeRewardBps, &finalize_reward_bps);
        env.storage()
            .instance()
            .extend_ttl(INSTANCE_LIFETIME_THRESHOLD, INSTANCE_BUMP_AMOUNT);

        Ok(())
    }

    /// Replaces the resolver committee. Only callable by the admin set at
    /// initialization. `new_resolvers` must have an odd length so a simple
    /// majority vote can never tie. Callable even while paused, so a
    /// compromised committee can be replaced without waiting to unpause.
    ///
    /// This is the emergency override path. It supersedes any in-flight
    /// self-rotation vote: an open `RotationProposal` is cleared (emitting
    /// `RotationCancelled` when one was present), so a proposal can never
    /// execute against a committee it wasn't built for. Day-to-day committee
    /// changes go through `propose_rotation` / `vote_rotation` instead.
    pub fn update_resolvers(env: Env, new_resolvers: Vec<Address>) -> Result<(), Error> {
        let admin: Address = env
            .storage()
            .instance()
            .get(&DataKey::Admin)
            .ok_or(Error::NotInitialized)?;
        admin.require_auth();

        if new_resolvers.is_empty() || new_resolvers.len().is_multiple_of(2) {
            return Err(Error::InvalidResolverCount);
        }
        if new_resolvers.len() > MAX_RESOLVERS {
            return Err(Error::TooManyResolvers);
        }
        Self::assert_unique_resolvers(&new_resolvers)?;

        // Admin override cancels any committee-driven rotation in flight. The
        // only other way the committee changes is rotation execution, which
        // also clears the proposal, so a live proposal always matches the
        // current committee it was validated against.
        if let Some(proposal) = env
            .storage()
            .instance()
            .get::<_, RotationProposal>(&DataKey::RotationProposal)
        {
            env.storage().instance().remove(&DataKey::RotationProposal);
            RotationCancelled {
                old_resolver: proposal.old_resolver,
                new_resolver: proposal.new_resolver,
            }
            .publish(&env);
        }

        env.storage()
            .instance()
            .set(&DataKey::Resolvers, &new_resolvers);
        ResolversUpdated {
            resolvers: new_resolvers,
        }
        .publish(&env);

        Ok(())
    }

    /// Proposes a single-slot committee rotation: remove `old_resolver` (must be
    /// a current resolver) and add `new_resolver` (must not already be one). Only
    /// a current resolver may propose, and only one rotation may be open at a
    /// time. The proposal is decided by a strict majority of the live committee
    /// (the same threshold used to resolve disputes) via `vote_rotation`. The
    /// committee written on execution is the same `Resolvers` slot `update_resolvers`
    /// writes, so a rotation has no effect on disputes already open (their
    /// committee was snapshotted at `dispute` time). Pause-exempt, like
    /// `update_resolvers`.
    pub fn propose_rotation(
        env: Env,
        resolver: Address,
        old_resolver: Address,
        new_resolver: Address,
    ) -> Result<(), Error> {
        // Audited via: test_cannot_propose_rotation_by_non_resolver (NotAResolver),
        // test_cannot_propose_rotation_for_non_member (ResolverNotInCommittee),
        // test_cannot_propose_rotation_with_duplicate_new (DuplicateResolver),
        // test_rotation_in_progress_blocks_second_proposal (RotationInProgress).
        let committee: Vec<Address> = Self::get(&env, &DataKey::Resolvers)?;
        resolver.require_auth();
        if !committee.contains(&resolver) {
            return Err(Error::NotAResolver);
        }
        if env.storage().instance().has(&DataKey::RotationProposal) {
            return Err(Error::RotationInProgress);
        }
        if !committee.contains(&old_resolver) {
            return Err(Error::ResolverNotInCommittee);
        }
        if committee.contains(&new_resolver) || old_resolver == new_resolver {
            return Err(Error::DuplicateResolver);
        }

        let proposal = RotationProposal {
            old_resolver: old_resolver.clone(),
            new_resolver: new_resolver.clone(),
            proposed_by: resolver.clone(),
            yes: Vec::new(&env),
            no: Vec::new(&env),
        };
        env.storage()
            .instance()
            .set(&DataKey::RotationProposal, &proposal);
        RotationProposed {
            old_resolver,
            new_resolver,
            proposed_by: resolver,
        }
        .publish(&env);

        Ok(())
    }

    /// A resolver votes on the open rotation proposal. `approve` records a yes or
    /// no (both prevent re-voting). Once yes-votes reach a strict majority of the
    /// live committee, the rotation executes immediately: `old_resolver` is swapped
    /// for `new_resolver` in the live committee, and the proposal is cleared.
    /// If the remaining unvoted resolvers can no longer supply enough yes-votes to
    /// reach a majority, the proposal is cancelled automatically (deadlock guard).
    /// Returns `Some(true)` if the rotation executed, `Some(false)` if it was
    /// auto-cancelled as dead, and `None` if the proposal remains open.
    pub fn vote_rotation(
        env: Env,
        resolver: Address,
        approve: bool,
    ) -> Result<Option<bool>, Error> {
        // Audited via: test_rotation_requires_majority_then_executes (execute path),
        // test_rotation_vote_twice_fails (AlreadyVoted),
        // test_non_resolver_cannot_vote_rotation (NotAResolver),
        // test_deadlock_autocancels_rotation (deadlock guard),
        // test_cannot_vote_rotation_without_proposal (NoRotationProposal below).
        let mut proposal: RotationProposal = env
            .storage()
            .instance()
            .get(&DataKey::RotationProposal)
            // NoRotationProposal: triggered by test_cannot_vote_rotation_without_proposal.
            .ok_or(Error::NoRotationProposal)?;
        resolver.require_auth();

        let committee: Vec<Address> = Self::get(&env, &DataKey::Resolvers)?;
        if !committee.contains(&resolver) {
            return Err(Error::NotAResolver);
        }
        if proposal.yes.contains(&resolver) || proposal.no.contains(&resolver) {
            return Err(Error::AlreadyVoted);
        }

        if approve {
            proposal.yes.push_back(resolver);
        } else {
            proposal.no.push_back(resolver);
        }

        let n = committee.len();
        let majority = (n / 2) + 1;

        if proposal.yes.len() >= majority {
            // Execute: swap old -> new in the live committee. The proposal is
            // the only live reference to the old/new pair, and the committee
            // has not changed since the proposal was validated (update_resolvers
            // and rotation execution both clear the proposal), so the swap is
            // always well-formed.
            let mut new_committee = Vec::new(&env);
            for addr in committee.iter() {
                if addr == proposal.old_resolver {
                    new_committee.push_back(proposal.new_resolver.clone());
                } else {
                    new_committee.push_back(addr);
                }
            }

            env.storage()
                .instance()
                .set(&DataKey::Resolvers, &new_committee);
            env.storage().instance().remove(&DataKey::RotationProposal);
            RotationExecuted {
                old_resolver: proposal.old_resolver.clone(),
                new_resolver: proposal.new_resolver.clone(),
            }
            .publish(&env);
            ResolversUpdated {
                resolvers: new_committee,
            }
            .publish(&env);
            return Ok(Some(true));
        }

        // Deadlock guard: yes-votes cast plus every still-unvoted resolver still
        // can't reach a majority, so the proposal can never pass. Cancel it.
        let remaining = n - proposal.yes.len() - proposal.no.len();
        if proposal.yes.len() + remaining < majority {
            env.storage().instance().remove(&DataKey::RotationProposal);
            RotationCancelled {
                old_resolver: proposal.old_resolver.clone(),
                new_resolver: proposal.new_resolver.clone(),
            }
            .publish(&env);
            return Ok(Some(false));
        }

        env.storage()
            .instance()
            .set(&DataKey::RotationProposal, &proposal);
        Ok(None)
    }

    /// Cancels the open rotation proposal. The proposer may cancel at any time.
    /// Any current resolver may also cancel once the proposal can no longer reach
    /// a majority (deadlock guard), so a lost proposer key can't permanently
    /// block rotation. Emits `RotationCancelled`.
    pub fn cancel_rotation(env: Env, resolver: Address) -> Result<(), Error> {
        // Audited via: test_proposer_can_cancel_rotation (proposer cancel),
        // test_non_proposer_cannot_cancel_passable_rotation (NotProposer),
        // test_cannot_cancel_rotation_without_proposal (NoRotationProposal below).
        let proposal: RotationProposal = env
            .storage()
            .instance()
            .get(&DataKey::RotationProposal)
            // NoRotationProposal: triggered by test_cannot_cancel_rotation_without_proposal.
            .ok_or(Error::NoRotationProposal)?;
        resolver.require_auth();

        let committee: Vec<Address> = Self::get(&env, &DataKey::Resolvers)?;
        if !committee.contains(&resolver) {
            return Err(Error::NotAResolver);
        }

        // Proposer may always cancel; anyone may cancel a proposal that can no
        // longer pass. Otherwise a non-proposer touching a still-passable
        // proposal is rejected.
        let n = committee.len();
        let majority = (n / 2) + 1;
        let remaining = n - proposal.yes.len() - proposal.no.len();
        let can_cancel =
            resolver == proposal.proposed_by || proposal.yes.len() + remaining < majority;
        if !can_cancel {
            return Err(Error::NotProposer);
        }

        env.storage().instance().remove(&DataKey::RotationProposal);
        RotationCancelled {
            old_resolver: proposal.old_resolver,
            new_resolver: proposal.new_resolver,
        }
        .publish(&env);

        Ok(())
    }

    /// Pauses or unpauses new assertions, disputes, and resolver votes.
    /// Assertions already `Pending` can still be `finalize`d while paused,
    /// so existing uncontested claims aren't stuck waiting on an unpause.
    /// Only callable by the admin set at initialization.
    pub fn set_paused(env: Env, paused: bool) -> Result<(), Error> {
        let admin: Address = env
            .storage()
            .instance()
            .get(&DataKey::Admin)
            .ok_or(Error::NotInitialized)?;
        admin.require_auth();

        env.storage().instance().set(&DataKey::Paused, &paused);
        PauseUpdated { paused }.publish(&env);

        Ok(())
    }

    fn require_not_paused(env: &Env) -> Result<(), Error> {
        let paused: bool = env
            .storage()
            .instance()
            .get(&DataKey::Paused)
            .ok_or(Error::NotInitialized)?;
        if paused {
            return Err(Error::Paused);
        }
        Ok(())
    }

    /// Posts a bonded claim about an outcome. Returns the new assertion id.
    pub fn assert_outcome(env: Env, asserter: Address, outcome: bool) -> Result<u64, Error> {
        Self::require_not_paused(&env)?;
        asserter.require_auth();

        let bond_amount: i128 = Self::get(&env, &DataKey::BondAmount)?;

        // The new id is reserved and the assertion written before the
        // external token transfer below, so a reentrant call during the
        // transfer can't be allocated the same not-yet-incremented id.
        let id: u64 = Self::get(&env, &DataKey::NextId)?;
        env.storage().instance().set(&DataKey::NextId, &(id + 1));
        let assertion = Assertion {
            asserter: asserter.clone(),
            outcome,
            bond: bond_amount,
            opened_at: env.ledger().timestamp(),
            status: Status::Pending,
            disputer: None,
            votes_for_outcome: 0,
            votes_against_outcome: 0,
            voted: Vec::new(&env),
            resolvers: Vec::new(&env),
            finalizer: None,
        };
        Self::set_assertion(&env, id, &assertion);

        let token_id: Address = Self::get(&env, &DataKey::Token)?;
        token::Client::new(&env, &token_id).transfer(
            &asserter,
            env.current_contract_address(),
            &bond_amount,
        );

        Asserted {
            id,
            asserter,
            outcome,
        }
        .publish(&env);

        Ok(id)
    }

    /// Disputes a pending assertion within the challenge window by matching its bond.
    pub fn dispute(env: Env, disputer: Address, id: u64) -> Result<(), Error> {
        Self::require_not_paused(&env)?;
        disputer.require_auth();

        let mut assertion = Self::get_assertion(&env, id)?;
        if assertion.status != Status::Pending {
            return Err(Error::NotPending);
        }

        let window: u64 = Self::get(&env, &DataKey::ChallengeWindow)?;
        if env.ledger().timestamp() > assertion.opened_at + window {
            return Err(Error::ChallengeWindowClosed);
        }

        // Snapshot the current resolver committee onto the assertion: voting
        // and majority for this dispute are decided against this snapshot
        // for its whole lifetime, not the live committee, so a later
        // `update_resolvers` can't change who gets to decide it.
        assertion.resolvers = Self::get(&env, &DataKey::Resolvers)?;

        // State is written before the external token transfer below so that
        // a reentrant call from a non-standard token sees this assertion as
        // already disputed, rather than still `Pending`.
        assertion.disputer = Some(disputer.clone());
        assertion.status = Status::Disputed;
        Self::set_assertion(&env, id, &assertion);

        let token_id: Address = Self::get(&env, &DataKey::Token)?;
        token::Client::new(&env, &token_id).transfer(
            &disputer,
            env.current_contract_address(),
            &assertion.bond,
        );

        Disputed { id, disputer }.publish(&env);

        Ok(())
    }

    /// Finalizes a pending assertion once its challenge window has elapsed
    /// with no dispute. `caller` must authorize the call unconditionally —
    /// regardless of whether `finalize_reward_bps` is zero — so the address
    /// recorded in `Assertion.finalizer` and the `Finalized` event is always
    /// a verified caller and cannot be spoofed. When `finalize_reward_bps` is
    /// non-zero, `caller` also receives `bond * finalize_reward_bps / 10_000`
    /// tokens as an incentive for prompt finalization and the asserter
    /// receives the remainder; when it is zero the full bond is returned to
    /// the asserter and no reward is paid. Returns the asserted outcome.
    pub fn finalize(env: Env, caller: Address, id: u64) -> Result<bool, Error> {
        // Auth is required unconditionally: even when finalize_reward_bps is
        // zero and no reward is paid, the caller's address is written into
        // Assertion.finalizer and the Finalized event as the finalizer of
        // record. Requiring auth here ensures that value is always a verified
        // address, not an arbitrary one anyone could have passed in.
        caller.require_auth();

        let mut assertion = Self::get_assertion(&env, id)?;
        if assertion.status != Status::Pending {
            return Err(Error::NotPending);
        }

        let window: u64 = Self::get(&env, &DataKey::ChallengeWindow)?;
        if env.ledger().timestamp() <= assertion.opened_at + window {
            return Err(Error::ChallengeWindowOpen);
        }

        let reward_bps: u32 = Self::get(&env, &DataKey::FinalizeRewardBps)?;
        let reward = if reward_bps > 0 {
            assertion.bond * (reward_bps as i128) / 10_000
        } else {
            0
        };

        // State is written before the external token transfers below so that
        // a reentrant call from a non-standard token sees this assertion as
        // already resolved, rather than still `Pending`.
        assertion.status = Status::Resolved;
        assertion.finalizer = Some(caller.clone());
        Self::set_assertion(&env, id, &assertion);

        let token_id: Address = Self::get(&env, &DataKey::Token)?;
        let token_client = token::Client::new(&env, &token_id);

        if reward > 0 {
            // Pay the caller their reward first, then pay the asserter the
            // remainder. Both transfers happen after the state write above, so
            // a reentrant token can't trigger a second finalize on the same id.
            token_client.transfer(&env.current_contract_address(), &caller, &reward);
        }

        let asserter_payout = assertion.bond - reward;
        token_client.transfer(
            &env.current_contract_address(),
            &assertion.asserter,
            &asserter_payout,
        );

        Finalized {
            id,
            outcome: assertion.outcome,
            finalizer: caller,
            reward,
        }
        .publish(&env);

        Ok(assertion.outcome)
    }

    /// A resolver votes on a disputed assertion. Once a strict majority of
    /// the resolver committee agrees, the assertion finalizes: the winning
    /// side (asserter if the original outcome stands, disputer otherwise)
    /// receives both bonds.
    pub fn resolve(
        env: Env,
        resolver: Address,
        id: u64,
        agrees_with_asserter: bool,
    ) -> Result<Option<bool>, Error> {
        Self::require_not_paused(&env)?;
        resolver.require_auth();

        let mut assertion = Self::get_assertion(&env, id)?;
        if assertion.status != Status::Disputed {
            return Err(Error::NotDisputed);
        }
        // Membership and majority are decided against the committee snapshot
        // taken when this assertion was disputed, not the live committee.
        if !assertion.resolvers.contains(&resolver) {
            return Err(Error::NotAResolver);
        }
        if assertion.voted.contains(&resolver) {
            return Err(Error::AlreadyVoted);
        }

        assertion.voted.push_back(resolver);
        if agrees_with_asserter {
            assertion.votes_for_outcome += 1;
        } else {
            assertion.votes_against_outcome += 1;
        }

        let majority = (assertion.resolvers.len() / 2) + 1;
        let winner_is_asserter = if assertion.votes_for_outcome >= majority {
            Some(true)
        } else if assertion.votes_against_outcome >= majority {
            Some(false)
        } else {
            None
        };

        let Some(winner_is_asserter) = winner_is_asserter else {
            Self::set_assertion(&env, id, &assertion);
            return Ok(None);
        };

        let payout = assertion.bond * 2;
        let winner = if winner_is_asserter {
            assertion.asserter.clone()
        } else {
            assertion.disputer.clone().unwrap()
        };
        let final_outcome = if winner_is_asserter {
            assertion.outcome
        } else {
            !assertion.outcome
        };

        // State is written before the external token transfer below so that
        // a reentrant call from a non-standard token sees this assertion as
        // already resolved (and this resolver as already voted), rather than
        // still open for further votes.
        assertion.status = Status::Resolved;
        Self::set_assertion(&env, id, &assertion);

        let token_id: Address = Self::get(&env, &DataKey::Token)?;
        token::Client::new(&env, &token_id).transfer(
            &env.current_contract_address(),
            &winner,
            &payout,
        );
        Resolved {
            id,
            outcome: final_outcome,
        }
        .publish(&env);

        Ok(Some(final_outcome))
    }

    pub fn get_assertion_state(env: Env, id: u64) -> Result<Assertion, Error> {
        Self::get_assertion(&env, id)
    }

    fn get_assertion(env: &Env, id: u64) -> Result<Assertion, Error> {
        env.storage()
            .persistent()
            .get(&DataKey::Assertion(id))
            .ok_or(Error::AssertionNotFound)
    }

    /// Writes an assertion and extends its persistent storage TTL. Every
    /// write site uses this rather than a bare `.set()` so an assertion's
    /// ledger entry can't be archived out from under it while it's still
    /// `Pending` or `Disputed`.
    fn set_assertion(env: &Env, id: u64, assertion: &Assertion) {
        let key = DataKey::Assertion(id);
        env.storage().persistent().set(&key, assertion);
        env.storage().persistent().extend_ttl(
            &key,
            ASSERTION_LIFETIME_THRESHOLD,
            ASSERTION_BUMP_AMOUNT,
        );
    }

    fn get<T: soroban_sdk::TryFromVal<Env, soroban_sdk::Val>>(
        env: &Env,
        key: &DataKey,
    ) -> Result<T, Error> {
        env.storage()
            .instance()
            .get(key)
            .ok_or(Error::NotInitialized)
    }

    /// Rejects a resolver committee containing duplicate addresses.
    ///
    /// Called from `initialize` and `update_resolvers` to preserve the
    /// invariant documented on `initialize` (odd length → a simple majority
    /// can never tie): a committee like `[A, A, B]` passes the odd-length
    /// check while being an effective electorate of two, silently breaking
    /// that guarantee. Duplicates also make the majority denominator
    /// unreachable for cases like `[A, A, A, B, C]` (majority 3, only 3
    /// distinct voters), which would strand both bonds.
    ///
    /// O(n²) pairwise scan, bounded by `MAX_RESOLVERS` (21) → at most ~210
    /// comparisons, well within budget and cheaper than pulling a hashing
    /// dependency into `no_std`.
    fn assert_unique_resolvers(resolvers: &Vec<Address>) -> Result<(), Error> {
        let len = resolvers.len();
        for i in 0..len {
            let a = resolvers.get(i).unwrap();
            for j in (i + 1)..len {
                if a == resolvers.get(j).unwrap() {
                    return Err(Error::DuplicateResolvers);
                }
            }
        }
        Ok(())
    }
}

mod test;

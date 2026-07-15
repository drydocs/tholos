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
    InvalidBondAmount = 12,
    InvalidChallengeWindow = 13,
    TooManyResolvers = 14,
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

#[contract]
pub struct Tholos;

#[contractimpl]
impl Tholos {
    /// Initializes the contract. `resolvers` must have an odd length so a
    /// simple majority vote can never tie.
    pub fn initialize(
        env: Env,
        admin: Address,
        token: Address,
        bond_amount: i128,
        challenge_window_secs: u64,
        resolvers: Vec<Address>,
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
        if bond_amount <= 0 {
            return Err(Error::InvalidBondAmount);
        }
        if challenge_window_secs == 0 || challenge_window_secs > MAX_CHALLENGE_WINDOW_SECS {
            return Err(Error::InvalidChallengeWindow);
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
            .extend_ttl(INSTANCE_LIFETIME_THRESHOLD, INSTANCE_BUMP_AMOUNT);

        Ok(())
    }

    /// Replaces the resolver committee. Only callable by the admin set at
    /// initialization. `new_resolvers` must have an odd length so a simple
    /// majority vote can never tie. Callable even while paused, so a
    /// compromised committee can be replaced without waiting to unpause.
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

        env.storage()
            .instance()
            .set(&DataKey::Resolvers, &new_resolvers);
        ResolversUpdated {
            resolvers: new_resolvers,
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

    /// Anyone can finalize a pending assertion once its challenge window has
    /// elapsed with no dispute. The asserter's bond is simply returned.
    pub fn finalize(env: Env, id: u64) -> Result<bool, Error> {
        let mut assertion = Self::get_assertion(&env, id)?;
        if assertion.status != Status::Pending {
            return Err(Error::NotPending);
        }

        let window: u64 = Self::get(&env, &DataKey::ChallengeWindow)?;
        if env.ledger().timestamp() <= assertion.opened_at + window {
            return Err(Error::ChallengeWindowOpen);
        }

        // State is written before the external token transfer below so that
        // a reentrant call from a non-standard token sees this assertion as
        // already resolved, rather than still `Pending`.
        assertion.status = Status::Resolved;
        Self::set_assertion(&env, id, &assertion);

        let token_id: Address = Self::get(&env, &DataKey::Token)?;
        token::Client::new(&env, &token_id).transfer(
            &env.current_contract_address(),
            &assertion.asserter,
            &assertion.bond,
        );

        Finalized {
            id,
            outcome: assertion.outcome,
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
}

mod test;

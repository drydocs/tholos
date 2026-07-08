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
}

const DAY_IN_LEDGERS: u32 = 17280;
const INSTANCE_BUMP_AMOUNT: u32 = 30 * DAY_IN_LEDGERS;
const INSTANCE_LIFETIME_THRESHOLD: u32 = INSTANCE_BUMP_AMOUNT - DAY_IN_LEDGERS;

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
        env.storage()
            .instance()
            .extend_ttl(INSTANCE_LIFETIME_THRESHOLD, INSTANCE_BUMP_AMOUNT);

        Ok(())
    }

    /// Replaces the resolver committee. Only callable by the admin set at
    /// initialization. `new_resolvers` must have an odd length so a simple
    /// majority vote can never tie.
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

        env.storage()
            .instance()
            .set(&DataKey::Resolvers, &new_resolvers);
        ResolversUpdated {
            resolvers: new_resolvers,
        }
        .publish(&env);

        Ok(())
    }

    /// Posts a bonded claim about an outcome. Returns the new assertion id.
    pub fn assert_outcome(env: Env, asserter: Address, outcome: bool) -> Result<u64, Error> {
        asserter.require_auth();

        let token_id: Address = Self::get(&env, &DataKey::Token);
        let bond_amount: i128 = Self::get(&env, &DataKey::BondAmount);

        token::Client::new(&env, &token_id).transfer(
            &asserter,
            env.current_contract_address(),
            &bond_amount,
        );

        let id: u64 = env.storage().instance().get(&DataKey::NextId).unwrap_or(0);
        let assertion = Assertion {
            asserter,
            outcome,
            bond: bond_amount,
            opened_at: env.ledger().timestamp(),
            status: Status::Pending,
            disputer: None,
            votes_for_outcome: 0,
            votes_against_outcome: 0,
            voted: Vec::new(&env),
        };

        env.storage()
            .persistent()
            .set(&DataKey::Assertion(id), &assertion);
        env.storage().instance().set(&DataKey::NextId, &(id + 1));
        Asserted {
            id,
            asserter: assertion.asserter,
            outcome,
        }
        .publish(&env);

        Ok(id)
    }

    /// Disputes a pending assertion within the challenge window by matching its bond.
    pub fn dispute(env: Env, disputer: Address, id: u64) -> Result<(), Error> {
        disputer.require_auth();

        let mut assertion = Self::get_assertion(&env, id)?;
        if assertion.status != Status::Pending {
            return Err(Error::NotPending);
        }

        let window: u64 = Self::get(&env, &DataKey::ChallengeWindow);
        if env.ledger().timestamp() > assertion.opened_at + window {
            return Err(Error::ChallengeWindowClosed);
        }

        let token_id: Address = Self::get(&env, &DataKey::Token);
        token::Client::new(&env, &token_id).transfer(
            &disputer,
            env.current_contract_address(),
            &assertion.bond,
        );

        assertion.disputer = Some(disputer.clone());
        assertion.status = Status::Disputed;
        env.storage()
            .persistent()
            .set(&DataKey::Assertion(id), &assertion);
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

        let window: u64 = Self::get(&env, &DataKey::ChallengeWindow);
        if env.ledger().timestamp() <= assertion.opened_at + window {
            return Err(Error::ChallengeWindowOpen);
        }

        let token_id: Address = Self::get(&env, &DataKey::Token);
        token::Client::new(&env, &token_id).transfer(
            &env.current_contract_address(),
            &assertion.asserter,
            &assertion.bond,
        );

        assertion.status = Status::Resolved;
        env.storage()
            .persistent()
            .set(&DataKey::Assertion(id), &assertion);
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
        resolver.require_auth();

        let resolvers: Vec<Address> = Self::get(&env, &DataKey::Resolvers);
        if !resolvers.contains(&resolver) {
            return Err(Error::NotAResolver);
        }

        let mut assertion = Self::get_assertion(&env, id)?;
        if assertion.status != Status::Disputed {
            return Err(Error::NotDisputed);
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

        let majority = (resolvers.len() / 2) + 1;
        let winner_is_asserter = if assertion.votes_for_outcome >= majority {
            Some(true)
        } else if assertion.votes_against_outcome >= majority {
            Some(false)
        } else {
            None
        };

        let Some(winner_is_asserter) = winner_is_asserter else {
            env.storage()
                .persistent()
                .set(&DataKey::Assertion(id), &assertion);
            return Ok(None);
        };

        let token_id: Address = Self::get(&env, &DataKey::Token);
        let payout = assertion.bond * 2;
        let winner = if winner_is_asserter {
            assertion.asserter.clone()
        } else {
            assertion.disputer.clone().unwrap()
        };
        token::Client::new(&env, &token_id).transfer(
            &env.current_contract_address(),
            &winner,
            &payout,
        );

        assertion.status = Status::Resolved;
        env.storage()
            .persistent()
            .set(&DataKey::Assertion(id), &assertion);

        let final_outcome = if winner_is_asserter {
            assertion.outcome
        } else {
            !assertion.outcome
        };
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

    fn get<T: soroban_sdk::TryFromVal<Env, soroban_sdk::Val>>(env: &Env, key: &DataKey) -> T {
        env.storage().instance().get(key).unwrap()
    }
}

mod test;

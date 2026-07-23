#![no_std]
use soroban_sdk::{contract, contracttype, Address, Env, String, Vec};

/// Contract that acts as an asserter using authorize_as_current_contract
#[contract]
pub struct AsserterConsumer;

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AsserterMessage {
    pub id: String,
    pub content: String,
    pub sender: Address,
    pub timestamp: u64,
}

#[contractimpl]
impl AsserterConsumer {
    /// Initialize the contract
    pub fn initialize(env: Env) {
        // Simple initialization
        env.storage().set(&String::from_str(&env, "initialized"), &true);
    }

    /// Create a message as the contract (using authorize_as_current_contract)
    pub fn create_message_as_contract(
        env: Env,
        tholos_contract: Address,
        message_id: String,
        content: String,
        sender: Address,
    ) -> Vec<String> {
        // This is the key pattern: authorize_as_current_contract
        // The contract authorizes itself to call Tholos
        
        // Create the message
        let message = AsserterMessage {
            id: message_id.clone(),
            content: content.clone(),
            sender: sender.clone(),
            timestamp: env.ledger().timestamp(),
        };

        // Use authorize_as_current_contract to call Tholos
        // This is the advanced pattern documented in INTEGRATION.md
        let result: Vec<String> = env
            .invoke_contract_with_authorization(
                &tholos_contract,
                &("store_message", env.current_contract_address(), &message),
            )
            .unwrap_or_else(|e| {
                panic!("Failed to store message: {:?}", e);
            });

        // Return the result
        result
    }

    /// Verify a message (using authorize_as_current_contract)
    pub fn verify_message_as_contract(
        env: Env,
        tholos_contract: Address,
        message_id: String,
    ) -> bool {
        // Use authorize_as_current_contract to verify a message
        let result: bool = env
            .invoke_contract_with_authorization(
                &tholos_contract,
                &("verify_message", env.current_contract_address(), &message_id),
            )
            .unwrap_or(false);

        result
    }

    /// Get all messages for the contract
    pub fn get_messages(
        env: Env,
        tholos_contract: Address,
    ) -> Vec<AsserterMessage> {
        // Use authorize_as_current_contract to get messages
        let result: Vec<AsserterMessage> = env
            .invoke_contract_with_authorization(
                &tholos_contract,
                &("get_messages", env.current_contract_address()),
            )
            .unwrap_or_else(|e| {
                panic!("Failed to get messages: {:?}", e);
            });

        result
    }

    /// Contract's own assertion method (for testing)
    pub fn assert_authorization(
        env: Env,
        expected_caller: Address,
    ) -> bool {
        // This demonstrates that the contract can assert its own authorization
        let current_caller = env.current_contract_address();
        current_caller == expected_caller
    }
}

#[cfg(test)]
mod test;

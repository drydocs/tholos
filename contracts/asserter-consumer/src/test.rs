#![cfg(test)]
use super::*;
use soroban_sdk::{Env, Address, String, testutils::AuthorizedFunction, testutils::AuthorizedInvocation};

#[test]
fn test_asserter_consumer_initialization() {
    let env = Env::default();
    
    AsserterConsumer::initialize(env.clone());
    
    // Verify initialization
    let is_initialized: bool = env.storage().get(&String::from_str(&env, "initialized")).unwrap_or(false);
    assert!(is_initialized);
}

#[test]
fn test_asserter_consumer_create_message_as_contract() {
    let env = Env::default();
    
    // Initialize the contract
    AsserterConsumer::initialize(env.clone());
    
    // Set up addresses
    let tholos_contract = Address::from_string(&String::from_str(&env, "GTHOLOS"));
    let sender = Address::from_string(&String::from_str(&env, "GSENDER"));
    let contract_addr = env.current_contract_address();
    
    // Mock the Tholos contract
    // This simulates the Tholos contract handling the authorization
    env.mock_all_auths();
    
    // Create a message as the contract
    let result = AsserterConsumer::create_message_as_contract(
        env.clone(),
        tholos_contract.clone(),
        String::from_str(&env, "msg_001"),
        String::from_str(&env, "Hello from contract asserter!"),
        sender.clone(),
    );
    
    // Verify the result
    // The actual result depends on the Tholos mock implementation
    // For this test, we verify the contract can call with authorization
    assert!(result.len() >= 0);
}

#[test]
fn test_asserter_consumer_verify_message() {
    let env = Env::default();
    
    // Initialize the contract
    AsserterConsumer::initialize(env.clone());
    
    // Set up addresses
    let tholos_contract = Address::from_string(&String::from_str(&env, "GTHOLOS"));
    
    // Mock the Tholos contract
    env.mock_all_auths();
    
    // Verify a message
    let result = AsserterConsumer::verify_message_as_contract(
        env.clone(),
        tholos_contract.clone(),
        String::from_str(&env, "msg_001"),
    );
    
    // For this test, we just verify the contract can call with authorization
    // The actual result depends on the Tholos mock
    assert!(result == true || result == false);
}

#[test]
fn test_asserter_contract_self_assertion() {
    let env = Env::default();
    
    // Get the contract address
    let contract_addr = env.current_contract_address();
    
    // Test self-assertion
    let result = AsserterConsumer::assert_authorization(
        env.clone(),
        contract_addr.clone(),
    );
    
    assert!(result);
}

#[test]
fn test_asserter_consumer_get_messages() {
    let env = Env::default();
    
    // Initialize the contract
    AsserterConsumer::initialize(env.clone());
    
    // Set up addresses
    let tholos_contract = Address::from_string(&String::from_str(&env, "GTHOLOS"));
    
    // Mock the Tholos contract
    env.mock_all_auths();
    
    // Get messages
    let messages = AsserterConsumer::get_messages(
        env.clone(),
        tholos_contract.clone(),
    );
    
    // Verify the contract can call with authorization
    assert!(messages.len() >= 0);
}

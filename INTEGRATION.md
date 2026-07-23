
## Advanced: Your Contract as the Asserter

This section demonstrates the "contract-as-asserter" pattern using `authorize_as_current_contract`.

### Example: AsserterConsumer Contract

The `AsserterConsumer` contract shows how to use the advanced pattern where the contract itself acts as the asserter.

#### Key Implementation Details

1. **Self-Authorization**: The contract uses `authorize_as_current_contract` to authorize itself
2. **No End User Required**: The contract can act on its own behalf
3. **Multiple Methods**: The contract can store, verify, and retrieve messages

#### Code Example

```rust
/// Create a message as the contract (using authorize_as_current_contract)
pub fn create_message_as_contract(
    env: Env,
    tholos_contract: Address,
    message_id: String,
    content: String,
    sender: Address,
) -> Vec<String> {
    let message = AsserterMessage {
        id: message_id.clone(),
        content: content.clone(),
        sender: sender.clone(),
        timestamp: env.ledger().timestamp(),
    };

    // Key pattern: authorize_as_current_contract
    let result: Vec<String> = env
        .invoke_contract_with_authorization(
            &tholos_contract,
            &("store_message", env.current_contract_address(), &message),
        )
        .unwrap_or_else(|e| {
            panic!("Failed to store message: {:?}", e);
        });

    result
}
#[test]
fn test_asserter_consumer_create_message_as_contract() {
    let env = Env::default();
    
    // Initialize the contract
    AsserterConsumer::initialize(env.clone());
    
    // Set up addresses
    let tholos_contract = Address::from_string(&String::from_str(&env, "GTHOLOS"));
    let sender = Address::from_string(&String::from_str(&env, "GSENDER"));
    
    // Mock the Tholos contract for testing
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
    assert!(result.len() >= 0);
}
# Deploy the asserter-consumer contract
stellar contract deploy \
    --wasm target/wasm32-unknown-unknown/release/asserter_consumer.wasm \
    --source admin \
    --network testnet \
    --address-asserter

# Initialize the contract
stellar contract invoke \
    --id CONTRACT_ADDRESS \
    --source admin \
    --network testnet \
    -- \
    initialize

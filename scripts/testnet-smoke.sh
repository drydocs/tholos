#!/usr/bin/env bash
# End-to-end smoke test for the Tholos assertion contract against Stellar testnet.
# Deploys a fresh instance, then walks it through: initialize, assert, dispute,
# resolve (majority vote), and a committee self-rotation (propose + majority vote),
# asserting on the token balance movement and post-rotation membership at each step.
set -euo pipefail

NETWORK=testnet
CONTRACT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
WASM_PATH="$CONTRACT_DIR/target/wasm32v1-none/release/tholos.wasm"
BOND_AMOUNT=1000000
CHALLENGE_WINDOW_SECS=3600

log() { echo ">> $*"; }

gen_key() {
  local name=$1
  stellar keys generate "$name" --network "$NETWORK" --fund --overwrite >/dev/null
  stellar keys address "$name"
}

balance() {
  local token=$1
  local addr=$2
  stellar contract invoke --id "$token" --source deployer --network "$NETWORK" -- balance --id "$addr" \
    | tr -d '"'
}

log "Building contract"
(cd "$CONTRACT_DIR/contracts/tholos" && stellar contract build >/dev/null)

log "Generating and funding identities"
DEPLOYER=$(gen_key deployer)
R1=$(gen_key resolver1)
R2=$(gen_key resolver2)
R3=$(gen_key resolver3)
ASSERTER=$(gen_key asserter)
DISPUTER=$(gen_key disputer)

log "Deploying contract"
CONTRACT=$(stellar contract deploy --wasm "$WASM_PATH" --source deployer --network "$NETWORK" 2>/dev/null | tail -1)
log "Contract: $CONTRACT"

TOKEN=$(stellar contract id asset --asset native --network "$NETWORK")
log "Token (native XLM SAC): $TOKEN"

log "Initializing with a 3-member resolver committee"
stellar contract invoke --id "$CONTRACT" --source deployer --network "$NETWORK" -- initialize \
  --admin "$DEPLOYER" \
  --token "$TOKEN" \
  --bond_amount "$BOND_AMOUNT" \
  --challenge_window_secs "$CHALLENGE_WINDOW_SECS" \
  --resolvers "[\"$R1\",\"$R2\",\"$R3\"]" \
  --finalize_reward_bps 0 >/dev/null

log "Posting assertion (outcome = true)"
ID=$(stellar contract invoke --id "$CONTRACT" --source asserter --network "$NETWORK" -- assert_outcome \
  --asserter "$ASSERTER" --outcome true 2>/dev/null | tail -1)
log "Assertion id: $ID"

log "Disputing assertion"
stellar contract invoke --id "$CONTRACT" --source disputer --network "$NETWORK" -- dispute \
  --disputer "$DISPUTER" --id "$ID" >/dev/null

BEFORE=$(balance "$TOKEN" "$DISPUTER")

log "Resolver 1 votes against the asserter"
stellar contract invoke --id "$CONTRACT" --source resolver1 --network "$NETWORK" -- resolve \
  --resolver "$R1" --id "$ID" --agrees_with_asserter false >/dev/null

log "Resolver 2 votes against the asserter (majority reached, should pay out)"
stellar contract invoke --id "$CONTRACT" --source resolver2 --network "$NETWORK" -- resolve \
  --resolver "$R2" --id "$ID" --agrees_with_asserter false >/dev/null

AFTER=$(balance "$TOKEN" "$DISPUTER")

log "Disputer balance: $BEFORE -> $AFTER"
EXPECTED=$((BEFORE + BOND_AMOUNT * 2))
if [ "$AFTER" != "$EXPECTED" ]; then
  echo "FAIL: expected disputer balance $EXPECTED, got $AFTER"
  exit 1
fi

STATE=$(stellar contract invoke --id "$CONTRACT" --source deployer --network "$NETWORK" -- get_assertion_state --id "$ID")
if ! echo "$STATE" | grep -q '"Resolved"'; then
  echo "FAIL: expected assertion status Resolved, got: $STATE"
  exit 1
fi

log "PASS: dispute resolved correctly, disputer paid both bonds"

log "Generating a 4th identity to rotate into the committee"
R4=$(gen_key resolver4)

log "Self-rotating R3 out, R4 in (strict-majority committee vote, no admin key)"
stellar contract invoke --id "$CONTRACT" --source resolver3 --network "$NETWORK" -- \
  propose_rotation --resolver "$R3" --old_resolver "$R3" --new_resolver "$R4" >/dev/null
stellar contract invoke --id "$CONTRACT" --source resolver1 --network "$NETWORK" -- \
  vote_rotation --resolver "$R1" --approve true >/dev/null
stellar contract invoke --id "$CONTRACT" --source resolver2 --network "$NETWORK" -- \
  vote_rotation --resolver "$R2" --approve true >/dev/null

log "Post-rotation: R4 is now a resolver; proving it by resolving a fresh dispute"
ID2=$(stellar contract invoke --id "$CONTRACT" --source asserter --network "$NETWORK" -- assert_outcome \
  --asserter "$ASSERTER" --outcome true 2>/dev/null | tail -1)
log "Second assertion id: $ID2"
stellar contract invoke --id "$CONTRACT" --source disputer --network "$NETWORK" -- dispute \
  --disputer "$DISPUTER" --id "$ID2" >/dev/null
stellar contract invoke --id "$CONTRACT" --source resolver4 --network "$NETWORK" -- resolve \
  --resolver "$R4" --id "$ID2" --agrees_with_asserter false >/dev/null
stellar contract invoke --id "$CONTRACT" --source resolver1 --network "$NETWORK" -- resolve \
  --resolver "$R1" --id "$ID2" --agrees_with_asserter false >/dev/null

STATE2=$(stellar contract invoke --id "$CONTRACT" --source deployer --network "$NETWORK" -- get_assertion_state --id "$ID2")
if ! echo "$STATE2" | grep -q '"Resolved"'; then
  echo "FAIL: expected second assertion status Resolved, got: $STATE2"
  exit 1
fi

log "PASS: self-rotation executed; R4 (the new resolver) helped resolve a dispute"


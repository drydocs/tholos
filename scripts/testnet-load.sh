#!/usr/bin/env bash
# E2E load and concurrency test for Tholos oracle on Stellar testnet.
# Usage: bash scripts/testnet-load.sh [N_assertions] [D_disputes]
set -euo pipefail

# Configuration
NETWORK=testnet
CONTRACT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
WASM_PATH="$CONTRACT_DIR/target/wasm32v1-none/release/tholos.wasm"
BOND_AMOUNT=1000000
CHALLENGE_WINDOW_SECS=120 # Short challenge window for quick test execution
# Funded identity this script deploys and reads balances from.
DEPLOYER_IDENTITY=load_deployer

# Logging, timing and invocation helpers shared with testnet-load-v2.sh.
# They read NETWORK and DEPLOYER_IDENTITY, so this must come after them.
# shellcheck source=SCRIPTDIR/lib/load-test-common.sh
source "$CONTRACT_DIR/scripts/lib/load-test-common.sh"

# Input parameters
N=${1:-5}
D=${2:-3}

# Clamp D to N
if [ "$D" -gt "$N" ]; then
  echo "Warning: Dispute count ($D) cannot exceed assertion count ($N). Clamping D to $N."
  D=$N
fi

log "Starting E2E load test (N=$N, D=$D)"

# Ensure contract is built
log "Rebuilding contract if necessary"
(cd "$CONTRACT_DIR/contracts/tholos" && $STELLAR contract build >/dev/null)

setup_start=$(get_time)
log "Generating and funding load test identities on testnet..."
DEPLOYER=$(gen_key "$DEPLOYER_IDENTITY")
R1=$(gen_key load_resolver1)
R2=$(gen_key load_resolver2)
R3=$(gen_key load_resolver3)
ASSERTER=$(gen_key load_asserter)
DISPUTER=$(gen_key load_disputer)

log "Deploying contract"
CONTRACT=$($STELLAR contract deploy --wasm "$WASM_PATH" --source "$DEPLOYER_IDENTITY" --network "$NETWORK" 2>/dev/null | tail -1)
log "Contract ID: $CONTRACT"

TOKEN=$($STELLAR contract id asset --asset native --network "$NETWORK")
log "Token (native XLM SAC): $TOKEN"

log "Initializing contract with 3-member resolver committee and challenge_window_secs=$CHALLENGE_WINDOW_SECS"
invoke_contract "$DEPLOYER_IDENTITY" --id "$CONTRACT" -- initialize \
  --admin "$DEPLOYER" \
  --token "$TOKEN" \
  --bond_amount "$BOND_AMOUNT" \
  --challenge_window_secs "$CHALLENGE_WINDOW_SECS" \
  --resolvers "[\"$R1\",\"$R2\",\"$R3\"]" >/dev/null
setup_end=$(get_time)
setup_duration=$(elapsed_time "$setup_start" "$setup_end")
log_success "Setup completed in ${setup_duration}s."

# --- PHASE 1: ASSERTIONS ---
log "Starting Phase 1: Creating $N assertions sequentially..."
phase1_start=$(get_time)
assertion_ids=()
assertion_times=()

for ((i=0; i<N; i++)); do
  assert_start=$(get_time)
  # Alternate outcomes to vary state
  outcome="true"
  if [ $((i % 2)) -eq 1 ]; then
    outcome="false"
  fi
  
  log "Posting assertion $((i+1))/$N (outcome=$outcome)"
  if ! ID=$(invoke_contract load_asserter --id "$CONTRACT" -- assert_outcome \
    --asserter "$ASSERTER" --outcome "$outcome"); then
    log_error "Assertion $((i+1)) failed!"
    exit 1
  fi
  
  assert_end=$(get_time)
  duration=$(elapsed_time "$assert_start" "$assert_end")
  assertion_ids+=("$ID")
  assertion_times+=("$duration")
  log_success "Assertion $((i+1)) posted successfully, ID: $ID (took ${duration}s)"
done

phase1_end=$(get_time)
phase1_duration=$(elapsed_time "$phase1_start" "$phase1_end")
log_success "Phase 1 (Assertions) completed in ${phase1_duration}s."

# --- PHASE 2: DISPUTES ---
log "Starting Phase 2: Disputing a subset of $D assertions..."
phase2_start=$(get_time)
dispute_times=()

for ((i=0; i<D; i++)); do
  dispute_start=$(get_time)
  id=${assertion_ids[i]}
  log "Disputing assertion ID: $id ($((i+1))/$D)"
  
  if ! invoke_contract load_disputer --id "$CONTRACT" -- dispute \
    --disputer "$DISPUTER" --id "$id" >/dev/null; then
    log_error "Dispute of ID $id failed!"
    exit 1
  fi
  
  dispute_end=$(get_time)
  duration=$(elapsed_time "$dispute_start" "$dispute_end")
  dispute_times+=("$duration")
  log_success "Disputed assertion ID $id successfully (took ${duration}s)"
done

phase2_end=$(get_time)
phase2_duration=$(elapsed_time "$phase2_start" "$phase2_end")
log_success "Phase 2 (Disputes) completed in ${phase2_duration}s."

# --- PHASE 3: RESOLUTIONS ---
log "Starting Phase 3: Resolving the $D disputes..."
phase3_start=$(get_time)
resolution_times=()

for ((i=0; i<D; i++)); do
  res_start=$(get_time)
  id=${assertion_ids[i]}
  log "Resolving assertion ID: $id ($((i+1))/$D)"
  
  # Vary the resolution outcome:
  # Even indices: vote agrees_with_asserter false (disputer wins)
  # Odd indices: vote agrees_with_asserter true (asserter wins)
  agrees_with_asserter="false"
  if [ $((i % 2)) -eq 1 ]; then
    agrees_with_asserter="true"
  fi
  
  log "Resolver 1 voting agrees_with_asserter=$agrees_with_asserter for ID $id"
  if ! invoke_contract load_resolver1 --id "$CONTRACT" -- resolve \
    --resolver "$R1" --id "$id" --agrees_with_asserter "$agrees_with_asserter" >/dev/null; then
    log_error "Resolver 1 vote failed for ID $id"
    exit 1
  fi
  
  log "Resolver 2 voting agrees_with_asserter=$agrees_with_asserter for ID $id (should resolve)"
  if ! invoke_contract load_resolver2 --id "$CONTRACT" -- resolve \
    --resolver "$R2" --id "$id" --agrees_with_asserter "$agrees_with_asserter" >/dev/null; then
    log_error "Resolver 2 vote failed for ID $id"
    exit 1
  fi
  
  # Verify state is Resolved
  state=$(invoke_contract "$DEPLOYER_IDENTITY" --id "$CONTRACT" -- get_assertion_state --id "$id")
  if ! echo "$state" | grep -q '"status":.*"Resolved"'; then
    log_error "Assertion $id state is not Resolved! Got: $state"
    exit 1
  fi
  
  res_end=$(get_time)
  duration=$(elapsed_time "$res_start" "$res_end")
  resolution_times+=("$duration")
  log_success "Resolved dispute ID $id successfully (took ${duration}s)"
done

phase3_end=$(get_time)
phase3_duration=$(elapsed_time "$phase3_start" "$phase3_end")
log_success "Phase 3 (Resolutions) completed in ${phase3_duration}s."

# --- PHASE 4: FINALIZATIONS ---
log "Starting Phase 4: Finalizing uncontested assertions..."
# Calculate remaining sleep time to satisfy challenge window
# Wait until CHALLENGE_WINDOW_SECS seconds have elapsed since phase1_end, plus 5 seconds safety margin
now_s=$(date +%s)
phase1_end_s=${phase1_end%.*}
elapsed_since_assertions=$((now_s - phase1_end_s))
sleep_needed=$((CHALLENGE_WINDOW_SECS - elapsed_since_assertions + 5))

if [ "$sleep_needed" -gt 0 ]; then
  log "Sleeping $sleep_needed seconds for the challenge window to close (with safety buffer)..."
  sleep "$sleep_needed"
fi

phase4_start=$(get_time)
finalization_times=()
uncontested_count=$((N - D))

for ((i=D; i<N; i++)); do
  fin_start=$(get_time)
  id=${assertion_ids[i]}
  log "Finalizing uncontested assertion ID: $id ($((i-D+1))/$uncontested_count)"
  
  # Implement retry logic to handle minor network/ledger timestamp delays
  retries=3
  success=false
  for ((r=0; r<retries; r++)); do
    if invoke_contract load_asserter --id "$CONTRACT" -- finalize --id "$id" >/dev/null; then
      success=true
      break
    else
      log "Finalization failed (waiting for ledger timestamp to advance). Retrying in 10 seconds (attempt $((r+1))/$retries)..."
      sleep 10
    fi
  done
  
  if [ "$success" = false ]; then
    log_error "Finalization of ID $id failed after retries!"
    exit 1
  fi
  
  # Verify state is Resolved
  state=$(invoke_contract "$DEPLOYER_IDENTITY" --id "$CONTRACT" -- get_assertion_state --id "$id")
  if ! echo "$state" | grep -q '"status":.*"Resolved"'; then
    log_error "Assertion $id state is not Resolved! Got: $state"
    exit 1
  fi
  
  fin_end=$(get_time)
  duration=$(elapsed_time "$fin_start" "$fin_end")
  finalization_times+=("$duration")
  log_success "Finalized assertion ID $id successfully (took ${duration}s)"
done

phase4_end=$(get_time)
phase4_duration=$(elapsed_time "$phase4_start" "$phase4_end")
log_success "Phase 4 (Finalizations) completed in ${phase4_duration}s."

# --- INTEGRITY CHECKS ---
log "Running contract and token balance integrity checks..."
contract_bal=$(balance "$TOKEN" "$CONTRACT")
log "Contract native token balance: $contract_bal"

if [ "$contract_bal" -ne 0 ]; then
  log_error "Integrity check failed: Contract balance is not 0 (got $contract_bal)"
  exit 1
fi
log_success "Integrity check passed: Contract token balance is exactly 0."

# --- TIMING SUMMARY ---
log "=================================================="
log "               LOAD TEST SUMMARY"
log "=================================================="
echo "Total Assertions: $N"
echo "Total Disputes:   $D"
echo "Total Finalized:  $uncontested_count"
echo ""
echo "Setup Phase:      ${setup_duration}s"
echo "Phase 1 (Assert): ${phase1_duration}s"
echo "Phase 2 (Disput): ${phase2_duration}s"
echo "Phase 3 (Resolv): ${phase3_duration}s"
echo "Phase 4 (Finalz): ${phase4_duration}s"
echo ""

avg_assert=$(avg_time "${assertion_times[@]}")
avg_dispute=$(avg_time "${dispute_times[@]}")
avg_resolve=$(avg_time "${resolution_times[@]}")
avg_finalize=$(avg_time "${finalization_times[@]}")

echo "Average Invocation Durations:"
echo "  Assert outcome: ${avg_assert}s"
echo "  Dispute:        ${avg_dispute}s"
echo "  Resolve (2txs): ${avg_resolve}s"
echo "  Finalize:       ${avg_finalize}s"
log "=================================================="
log_success "E2E Load and Concurrency test passed successfully!"

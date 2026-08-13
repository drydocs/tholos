#!/usr/bin/env bash
# E2E load test for Tholos v2 on Stellar testnet: many concurrent third-party
# positions on one dispute, multiple disputes open at the same time, one
# driven to a strict-majority result and one to the optimistic timeout
# default, settled and withdrawn in a shuffled (not registration) order to
# exercise the O(1)/order-independence invariant from #69.
#
# Deploys its own throwaway instance; not the canonical v2 deployment (there
# isn't one yet, see docs/src/DEPLOYMENT.md), and its short registration/
# reveal windows are sized for fast test iteration, not real dispute use.
#
# Usage: bash scripts/testnet-load-v2.sh [P_positions]
set -euo pipefail

NETWORK=testnet
CONTRACT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
WASM_PATH="$CONTRACT_DIR/target/wasm32v1-none/release/tholos_v2.wasm"
BOND_AMOUNT=1000000
CHALLENGE_WINDOW_SECS=120
# Sized for P sequential registration/reveal transactions at real testnet
# latency (each simulate+sign+submit+confirm round trip runs several
# seconds), not for how long a real deployment's windows should be; a
# too-short window here risks a late voter's registration failing with
# RegistrationClosed partway through the test, not just an unrealistic
# parameter choice. ANTI_SNIPE_HARD_MAX_SECS is deliberately larger than
# REGISTRATION_DURATION_SECS so the anti-snipe extension has actual
# headroom to use, rather than being capped at zero extra time.
REGISTRATION_DURATION_SECS=180
ANTI_SNIPE_EXTENSION_SECS=10
ANTI_SNIPE_HARD_MAX_SECS=240
REVEAL_DURATION_SECS=180
MAX_POSITION=10000000
MAX_TOTAL_WEIGHT=1000000000

# Number of third-party positions to fund on the strict-majority dispute.
P=${1:-8}

log() {
  echo -e "\033[1;34m>>\033[0m $*"
}

log_success() {
  echo -e "\033[1;32m✓\033[0m $*"
}

log_error() {
  echo -e "\033[1;31m✗\033[0m $*"
}

get_time() {
  date +%s.%N 2>/dev/null || date +%s
}

elapsed_time() {
  local start=$1
  local end=$2
  if command -v awk >/dev/null 2>&1; then
    awk -v s="$start" -v e="$end" 'BEGIN { printf "%.2f", e - s }'
  else
    local diff=$(( ${end%.*} - ${start%.*} ))
    echo "$diff"
  fi
}

avg_time() {
  local sum=0
  local count=${#@}
  if [ "$count" -eq 0 ]; then
    echo "0.00"
    return
  fi
  for val in "$@"; do
    sum=$(awk -v s="$sum" -v v="$val" 'BEGIN { print s + v }')
  done
  awk -v s="$sum" -v c="$count" 'BEGIN { printf "%.2f", s / c }'
}

STELLAR="stellar"

gen_key() {
  local name=$1
  $STELLAR keys generate "$name" --network "$NETWORK" --fund --overwrite >/dev/null
  $STELLAR keys address "$name"
}

balance() {
  local token=$1
  local addr=$2
  $STELLAR contract invoke --id "$token" --source v2load_deployer --network "$NETWORK" -- balance --id "$addr" 2>/dev/null \
    | tr -d '"'
}

invoke_contract() {
  local source=$1
  shift
  local tmp_out
  tmp_out=$(mktemp)
  local tmp_err
  tmp_err=$(mktemp)

  if ! $STELLAR contract invoke --source "$source" --network "$NETWORK" "$@" >"$tmp_out" 2>"$tmp_err"; then
    log_error "Invocation failed!"
    cat "$tmp_err" >&2
    rm -f "$tmp_out" "$tmp_err"
    return 1
  fi

  tail -1 "$tmp_out"
  rm -f "$tmp_out" "$tmp_err"
}

network_id() {
  # register()'s commitment hashes over env.ledger().network_id(), the
  # sha256 of the network passphrase (a public, well-known Stellar
  # constant, not something the RPC exposes as a plain getter). Prefer
  # sha256sum (coreutils, present on the ubuntu-latest CI runner this also
  # has to work on); fall back to openssl for local runs that lack it.
  local passphrase="Test SDF Network ; September 2015"
  if command -v sha256sum >/dev/null 2>&1; then
    printf '%s' "$passphrase" | sha256sum | awk '{print $1}'
  else
    printf '%s' "$passphrase" | openssl dgst -sha256 | awk '{print $NF}'
  fi
}

# Extracts one JSON string field's value from a `stellar contract invoke`
# response without depending on jq (not guaranteed present on every runner
# this script targets); every response here is single-line, unindented
# JSON, so a field always appears as a literal `"field":"value"` substring.
json_field() {
  local json=$1
  local field=$2
  echo "$json" | sed -n "s/.*\"$field\":\"\([^\"]*\)\".*/\\1/p"
}

log "Starting Tholos v2 E2E load test (P=$P third-party positions)"

log "Building contract"
(cd "$CONTRACT_DIR/contracts/tholos-v2" && $STELLAR contract build >/dev/null)

log "Building the commitment-computation helper"
(cd "$CONTRACT_DIR" && cargo build -p compute-commitment >/dev/null 2>&1)
COMPUTE_COMMITMENT="$CONTRACT_DIR/target/debug/compute-commitment"
if [ ! -x "$COMPUTE_COMMITMENT" ]; then
  COMPUTE_COMMITMENT="$CONTRACT_DIR/target/debug/compute-commitment.exe"
fi

NETWORK_ID=$(network_id)
log "Network id (sha256 of the testnet passphrase): $NETWORK_ID"

setup_start=$(get_time)
log "Generating and funding load test identities on testnet..."
DEPLOYER=$(gen_key v2load_deployer)
ASSERTER=$(gen_key v2load_asserter)
DISPUTER=$(gen_key v2load_disputer)

VOTERS=()
VOTER_ADDRS=()
for ((i=0; i<P; i++)); do
  name="v2load_voter$i"
  addr=$(gen_key "$name")
  VOTERS+=("$name")
  VOTER_ADDRS+=("$addr")
done

log "Deploying contract"
CONTRACT=$($STELLAR contract deploy --wasm "$WASM_PATH" --source v2load_deployer --network "$NETWORK" 2>/dev/null | tail -1)
log "Contract ID: $CONTRACT"

TOKEN=$($STELLAR contract id asset --asset native --network "$NETWORK")
log "Token (native XLM SAC): $TOKEN"

log "Initializing contract"
invoke_contract v2load_deployer --id "$CONTRACT" -- initialize \
  --admin "$DEPLOYER" \
  --token "$TOKEN" \
  --base_bond "$BOND_AMOUNT" \
  --challenge_window_secs "$CHALLENGE_WINDOW_SECS" \
  --finalize_reward_bps 0 \
  --registration_duration_secs "$REGISTRATION_DURATION_SECS" \
  --anti_snipe_extension_secs "$ANTI_SNIPE_EXTENSION_SECS" \
  --anti_snipe_hard_max_secs "$ANTI_SNIPE_HARD_MAX_SECS" \
  --reveal_duration_secs "$REVEAL_DURATION_SECS" \
  --max_position "$MAX_POSITION" \
  --max_total_weight "$MAX_TOTAL_WEIGHT" >/dev/null
setup_end=$(get_time)
setup_duration=$(elapsed_time "$setup_start" "$setup_end")
log_success "Setup completed in ${setup_duration}s ($((P + 3)) identities funded)."

# --- PHASE 1: OPEN AND DISPUTE, BOTH ASSERTIONS OPEN CONCURRENTLY ---
# Both assertions are opened and disputed before either is registered
# against or revealed, so the two disputes are genuinely concurrently
# active on-chain (not resolved one at a time), stressing the incremental
# W-tracking and per-position storage from #66 the same way real concurrent
# disputes would.
log "Starting Phase 1: opening and disputing two assertions..."
phase1_start=$(get_time)

MAJORITY_ID=$(invoke_contract v2load_asserter --id "$CONTRACT" -- assert_outcome \
  --asserter "$ASSERTER" --outcome true)
log "Majority-path assertion ID: $MAJORITY_ID"

TIMEOUT_ID=$(invoke_contract v2load_asserter --id "$CONTRACT" -- assert_outcome \
  --asserter "$ASSERTER" --outcome true)
log "Timeout-path assertion ID: $TIMEOUT_ID"

invoke_contract v2load_disputer --id "$CONTRACT" -- dispute \
  --disputer "$DISPUTER" --id "$MAJORITY_ID" >/dev/null
log_success "Disputed $MAJORITY_ID"

invoke_contract v2load_disputer --id "$CONTRACT" -- dispute \
  --disputer "$DISPUTER" --id "$TIMEOUT_ID" >/dev/null
log_success "Disputed $TIMEOUT_ID (both disputes now concurrently open)"

phase1_end=$(get_time)
phase1_duration=$(elapsed_time "$phase1_start" "$phase1_end")
log_success "Phase 1 completed in ${phase1_duration}s."

# --- PHASE 2: REGISTER P THIRD-PARTY POSITIONS ON THE MAJORITY DISPUTE ---
# TIMEOUT_ID deliberately gets zero third-party registrations, leaving its
# two fixed positions an exact tie: the eligible_total case that has to
# fall through to the optimistic timeout default (see #68).
#
# Each voter's commitment is the real sha256(VoteCommitmentPreimage)
# register()/reveal() actually verify (computed by compute_commitment, see
# lib.rs's VoteCommitmentPreimage), not an arbitrary placeholder: reveal
# would reject anything else with CommitmentVerificationFailed.
MAJORITY_POLICY_HASH=$(json_field "$(invoke_contract v2load_deployer --id "$CONTRACT" -- get_assertion --id "$MAJORITY_ID")" policy_hash)
log "Starting Phase 2: registering $P third-party positions on $MAJORITY_ID (all agreeing, driving it to a strict majority)..."
phase2_start=$(get_time)
registration_times=()
salts=()

for ((i=0; i<P; i++)); do
  reg_start=$(get_time)
  voter="${VOTERS[i]}"
  voter_addr="${VOTER_ADDRS[i]}"
  salt=$(printf '%064d' "$i")
  salts+=("$salt")
  commitment=$("$COMPUTE_COMMITMENT" "$NETWORK_ID" "$CONTRACT" "$MAJORITY_POLICY_HASH" "$MAJORITY_ID" "$voter_addr" true "$salt")

  if ! invoke_contract "$voter" --id "$CONTRACT" -- register \
    --voter "$voter_addr" --id "$MAJORITY_ID" --amount "$BOND_AMOUNT" \
    --commitment "$commitment" >/dev/null; then
    log_error "Registration $((i+1))/$P failed!"
    exit 1
  fi

  reg_end=$(get_time)
  duration=$(elapsed_time "$reg_start" "$reg_end")
  registration_times+=("$duration")
  log_success "Voter $((i+1))/$P registered (took ${duration}s)"
done

phase2_end=$(get_time)
phase2_duration=$(elapsed_time "$phase2_start" "$phase2_end")
log_success "Phase 2 (Registration) completed in ${phase2_duration}s."

# --- PHASE 3: WAIT FOR REGISTRATION TO CLOSE, THEN REVEAL ---
log "Waiting for the registration window to close..."
sleep $((REGISTRATION_DURATION_SECS + 5))

log "Starting Phase 3: revealing $P positions on $MAJORITY_ID..."
phase3_start=$(get_time)
reveal_times=()

for ((i=0; i<P; i++)); do
  rev_start=$(get_time)
  voter="${VOTERS[i]}"
  voter_addr="${VOTER_ADDRS[i]}"
  salt="${salts[i]}"

  if ! invoke_contract "$voter" --id "$CONTRACT" -- reveal \
    --voter "$voter_addr" --id "$MAJORITY_ID" --choice true --salt "$salt" >/dev/null; then
    log_error "Reveal $((i+1))/$P failed!"
    exit 1
  fi

  rev_end=$(get_time)
  duration=$(elapsed_time "$rev_start" "$rev_end")
  reveal_times+=("$duration")
  log_success "Voter $((i+1))/$P revealed (took ${duration}s)"
done

phase3_end=$(get_time)
phase3_duration=$(elapsed_time "$phase3_start" "$phase3_end")
log_success "Phase 3 (Reveal) completed in ${phase3_duration}s."

state=$(invoke_contract v2load_deployer --id "$CONTRACT" -- get_assertion --id "$MAJORITY_ID")
if ! echo "$state" | grep -q '"terminal_cause":"StrictMajorityFor"'; then
  log_error "Expected $MAJORITY_ID to have locked StrictMajorityFor. Got: $state"
  exit 1
fi
log_success "$MAJORITY_ID locked StrictMajorityFor as expected."

# --- PHASE 4: RESOLVE THE TIMEOUT-PATH DISPUTE ---
log "Waiting for $TIMEOUT_ID's registration and reveal windows to close..."
sleep $((REGISTRATION_DURATION_SECS + REVEAL_DURATION_SECS + 10))

log "Closing $TIMEOUT_ID via resolve_outcome (permissionless)..."
cause=$(invoke_contract v2load_deployer --id "$CONTRACT" -- resolve_outcome --id "$TIMEOUT_ID")
if [ "$cause" != '"OptimisticTimeout"' ]; then
  log_error "Expected $TIMEOUT_ID to resolve as OptimisticTimeout. Got: $cause"
  exit 1
fi
log_success "$TIMEOUT_ID resolved OptimisticTimeout as expected."

# --- PHASE 5: SETTLE, THEN WITHDRAW, BOTH DISPUTES IN A SHUFFLED ORDER ---
# Settling in an order deliberately different from registration order
# (last voter first, asserter/disputer interleaved in the middle) is the
# point here: #69's invariant is that every position's payout is
# independent of settlement order, so this has to produce the same result
# a natural order would.
#
# Settling and withdrawing the SAME position back to back (rather than
# settling everyone, then withdrawing everyone) would be a mistake here:
# any leftover floor-division dust is only credited once the settlement
# that brings the winning side's recipient weight up to its full total
# runs, which can easily be a *later* call than some recipient's own
# settlement. An interleaved settle-then-withdraw would let that early
# recipient's withdrawal run before the dust it's entitled to has even
# been credited yet, leaving it stranded in Credit(id, address) instead of
# actually paid out. Settling everyone first, then withdrawing everyone
# once every settlement (including whichever one turns out to be last) has
# already run, avoids that regardless of the shuffle order chosen.
log "Starting Phase 5: settling both disputes in shuffled order..."
phase5_start=$(get_time)

settle_times=()

settle_one() {
  local id=$1
  local addr=$2
  local s_start
  s_start=$(get_time)
  invoke_contract v2load_deployer --id "$CONTRACT" -- settle --id "$id" --address "$addr" >/dev/null
  local s_end
  s_end=$(get_time)
  elapsed_time "$s_start" "$s_end"
}

# TIMEOUT_ID: disputer before asserter, the reverse of dispute() creation order.
d=$(settle_one "$TIMEOUT_ID" "$DISPUTER")
settle_times+=("$d")
log_success "Settled $TIMEOUT_ID/disputer (took ${d}s)"
d=$(settle_one "$TIMEOUT_ID" "$ASSERTER")
settle_times+=("$d")
log_success "Settled $TIMEOUT_ID/asserter (took ${d}s)"

# MAJORITY_ID: last-registered voter first, then the disputer (forfeited),
# then the asserter, then the remaining voters in reverse.
d=$(settle_one "$MAJORITY_ID" "${VOTER_ADDRS[$((P-1))]}")
settle_times+=("$d")
log_success "Settled $MAJORITY_ID/voter $P (took ${d}s)"

d=$(settle_one "$MAJORITY_ID" "$DISPUTER")
settle_times+=("$d")
log_success "Settled $MAJORITY_ID/disputer (forfeited, took ${d}s)"

d=$(settle_one "$MAJORITY_ID" "$ASSERTER")
settle_times+=("$d")
log_success "Settled $MAJORITY_ID/asserter (took ${d}s)"

for ((i=P-2; i>=0; i--)); do
  d=$(settle_one "$MAJORITY_ID" "${VOTER_ADDRS[i]}")
  settle_times+=("$d")
  log_success "Settled $MAJORITY_ID/voter $((i+1)) (took ${d}s)"
done

# All settlements for both disputes are done, so every address's final
# credit balance (including any dust the last MAJORITY_ID settlement
# above routed to the asserter) is already fixed; withdrawing now, in a
# third, independent order, can't miss anything regardless of how Phase 5
# settled things.
log "All positions settled; withdrawing every non-zero credit balance..."
withdraw_times=()

withdraw_if_owed() {
  local id=$1
  local name=$2
  local addr=$3
  local credit
  credit=$(invoke_contract v2load_deployer --id "$CONTRACT" -- get_credit --id "$id" --address "$addr")
  credit=$(echo "$credit" | tr -d '"')
  if [ "$credit" = "0" ]; then
    log_success "Nothing owed to $name on $id, skipping withdraw"
    return
  fi
  local w_start
  w_start=$(get_time)
  invoke_contract "$name" --id "$CONTRACT" -- withdraw --owner "$addr" --id "$id" --destination "$addr" >/dev/null
  local w_end
  w_end=$(get_time)
  local duration
  duration=$(elapsed_time "$w_start" "$w_end")
  withdraw_times+=("$duration")
  log_success "Withdrew $name/$id (credit $credit, took ${duration}s)"
}

withdraw_if_owed "$TIMEOUT_ID" v2load_disputer "$DISPUTER"
withdraw_if_owed "$TIMEOUT_ID" v2load_asserter "$ASSERTER"
withdraw_if_owed "$MAJORITY_ID" v2load_asserter "$ASSERTER"
withdraw_if_owed "$MAJORITY_ID" v2load_disputer "$DISPUTER"
for ((i=0; i<P; i++)); do
  withdraw_if_owed "$MAJORITY_ID" "${VOTERS[i]}" "${VOTER_ADDRS[i]}"
done

phase5_end=$(get_time)
phase5_duration=$(elapsed_time "$phase5_start" "$phase5_end")
log_success "Phase 5 (Settle+Withdraw) completed in ${phase5_duration}s."

# --- INTEGRITY CHECKS ---
log "Running contract balance integrity check..."
contract_bal=$(balance "$TOKEN" "$CONTRACT")
log "Contract native token balance: $contract_bal"

if [ "$contract_bal" -ne 0 ]; then
  log_error "Integrity check failed: contract balance is not 0 (got $contract_bal)"
  exit 1
fi
log_success "Integrity check passed: contract token balance is exactly 0."

disputer_bal=$(balance "$TOKEN" "$DISPUTER")
log "Disputer final balance: $disputer_bal (expected: lost its $MAJORITY_ID bond, recovered its $TIMEOUT_ID bond in full)"

# --- TIMING SUMMARY ---
log "=================================================="
log "            V2 LOAD TEST SUMMARY"
log "=================================================="
echo "Third-party positions: $P"
echo "Disputes:               2 (1 strict-majority, 1 optimistic-timeout)"
echo ""
echo "Setup Phase:             ${setup_duration}s"
echo "Phase 1 (Assert+Dispute):${phase1_duration}s"
echo "Phase 2 (Registration):  ${phase2_duration}s"
echo "Phase 3 (Reveal):        ${phase3_duration}s"
echo "Phase 5 (Settle+Withdraw):${phase5_duration}s"
echo ""
echo "Average Invocation Durations:"
echo "  Register: $(avg_time "${registration_times[@]}")s"
echo "  Reveal:   $(avg_time "${reveal_times[@]}")s"
echo "  Settle:   $(avg_time "${settle_times[@]}")s"
echo "  Withdraw: $(avg_time "${withdraw_times[@]}")s"
log "=================================================="
log_success "Tholos v2 E2E load test passed successfully!"

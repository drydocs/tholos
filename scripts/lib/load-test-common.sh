# shellcheck shell=bash
# Helpers shared by the testnet load tests (scripts/testnet-load.sh and
# scripts/testnet-load-v2.sh). Sourced, never executed, so it deliberately
# has no shebang and no `set -euo pipefail` of its own: sourcing runs in the
# caller's shell, which already sets those, and re-setting them here would
# silently re-enable them for any future caller that had turned one off.
#
# Two variables must be set by the sourcing script *before* the source line:
#   NETWORK            - the Stellar network passed to every CLI call.
#   DEPLOYER_IDENTITY  - a funded `stellar keys` identity name, used as the
#                        source for the read-only balance() query.
# Everything else here is self-contained.

# The installed stellar CLI.
STELLAR="stellar"

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

# Averages its arguments, formatted to 2dp; "0.00" for an empty list, so a
# phase that recorded no timings doesn't divide by zero.
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

gen_key() {
  local name=$1
  $STELLAR keys generate "$name" --network "$NETWORK" --fund --overwrite >/dev/null
  $STELLAR keys address "$name"
}

# Reads an address's balance in a token contract. The invoke is a read-only
# simulation, so DEPLOYER_IDENTITY is just a funded identity to source it
# from; nothing is charged to or mutated on it.
balance() {
  local token=$1
  local addr=$2
  $STELLAR contract invoke --id "$token" --source "$DEPLOYER_IDENTITY" --network "$NETWORK" -- balance --id "$addr" 2>/dev/null \
    | tr -d '"'
}

# Wrapper to execute contract calls, capturing stdout/stderr for robust error reporting
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

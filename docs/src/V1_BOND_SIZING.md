# V1 Mainnet Parameter Selection Proposal

## Executive Summary

`docs/src/DEPLOYMENT.md` provides operational instructions for deploying and administering Tholos instances, but leaves the selection of core immutable parameters (`bond_amount`, `challenge_window_secs`, resolver committee size, and `finalize_reward_bps`) largely unprescribed.

Historically, scripts like `scripts/testnet-load.sh` set `BOND_AMOUNT=1000000` (0.10 XLM) and `CHALLENGE_WINDOW_SECS=120` (2 minutes). These values were chosen strictly for CI/test runtime convenience so automated suites complete within seconds, and are unsafe for production value.

This proposal extends the theoretical framework in [BOND_SIZING.md](BOND_SIZING.md) with empirical operational data, following the evidence-based methodology established in [V2_BOND_SIZING.md](V2_BOND_SIZING.md). It provides clear deployment profiles, threat model assumptions, and quantitative sizing formulas for teams deploying Tholos v1 on Stellar Mainnet.

---

## 1. Summary of Recommended Parameter Profiles

Because Tholos v1 parameters are immutable after `initialize` (with the exception of the resolver committee, which can rotate via admin or committee vote), deployers must size parameters to match the economic value of the claims being secured.

| Parameter Profile | Securing Value Range ($V$) | Asset | Recommended `bond_amount` | `challenge_window_secs` | Resolver Size / Quorum | `finalize_reward_bps` |
| :--- | :--- | :--- | :--- | :--- | :--- | :--- |
| **Profile A: Fast / Micropayment Feeds** | $10 – $500 | USDC | `20_0000000` (20 USDC) | `7200` (2 hours) | 3 members (2 of 3) | `50` (0.50% / 0.10 USDC) |
| **Profile B: Standard Commercial Escrow** *(Default)* | $500 – $25,000 | USDC | `100_0000000` (100 USDC) | `86400` (24 hours) | 5 members (3 of 5) | `50` (0.50% / 0.50 USDC) |
| **Profile C: High-Value Institutional Settlement** | $25,000 – $500,000+ | USDC | `1000_0000000` (1,000 USDC) | `259200` (72 hours) | 5 or 7 members (3 of 5 / 4 of 7) | `25` (0.25% / 2.50 USDC) |

---

## 2. Parameter Analysis: `bond_amount`

### 2.1 The Economic Role of the Bond

In Tholos v1, `bond_amount` is charged symmetrically:
1. An **asserter** locks `bond_amount` upon calling `assert_outcome`.
2. A **disputer** must lock an identical `bond_amount` upon calling `dispute`.
3. The winner of the dispute recovers their initial bond plus the loser's forfeited bond (minus any configured fee).

### 2.2 Sizing Floors & Constraints

Per `BOND_SIZING.md`, `bond_amount` must satisfy four lower bounds simultaneously:

```text
bond_amount >= max(
  R_case / max(1, K_spam),
  R_case / max(1, K_dispute),
  target_attacker_loss - min(C_assert, C_dispute),
  min_finalizer_reward * 10_000 / max(1, reward_bps)
)
```

1. **Resolver Triage Floor ($R_{\text{case}} / K_{\text{spam}}$)**:
   - Reviewing an off-chain dispute requires humans or oracle services to verify external state, document evidence, and submit on-chain votes (`resolve`).
   - Sizing estimate: An off-chain dispute triage costs an estimated $15 to $30 in operational overhead ($R_{\text{case}}$).
   - If an oracle deployment tolerates at most $K_{\text{spam}} = 1$ unpunished spam attempt per challenge window, the bond must be at least $30.
2. **Griefing Delay Cost**:
   - When an attacker disputes a valid assertion in bad faith, they freeze the asserter's capital and delay settlement for the duration of the resolution.
   - Sizing rule: To impose meaningful economic penalty, `bond_amount` must exceed the time-value-of-money advantage gained by delaying settlement.
3. **Affordability Ceiling**:
   - For legitimate users, posting a bond represents working capital friction.
   - We enforce `bond_amount <= V_min * 0.20` (bond should not exceed 20% of the minimum intended assertion value).

### 2.3 Worked Mainnet Examples

- For **Profile B (Standard Commercial Escrow, $500+ value)**:
  - Setting `bond_amount = 100 USDC` (`100_0000000` in 7-decimal token units).
  - An attacker attempting 5 bad-faith disputes burns **$500 USDC** directly to honest asserters.
  - Legitimate asserters of $1,000 escrows post a 10% bond, which is fully returned upon finalization.

---

## 3. Parameter Analysis: `challenge_window_secs`

### 3.1 Why 120 Seconds is Unsafe for Mainnet

In `scripts/testnet-load.sh`, `CHALLENGE_WINDOW_SECS=120`. On a live network:
- Stellar ledger close times average 5 seconds, but RPC propagation, horizon indexing, and transaction queuing can introduce 15–30 seconds of latency.
- Automated off-chain watcher bots require time to fetch emitted `AssertionCreated` events, verify claims against real-world data sources, and broadcast `dispute` transactions.
- If a watcher bot experiences temporary RPC disconnection, container restart, or network congestion, a 120-second window allows fraudulent assertions to finalize uncontested.

### 3.2 Mainnet Window Sizing Model

The challenge window duration $T_{\text{window}}$ must provide sufficient headroom for three sequential phases:

```text
T_window >= T_detection + T_verification + T_execution + T_contingency
```

| Phase | Automated Bot Architecture | Human / Multi-Sig Review Architecture |
| :--- | :--- | :--- |
| **Detection ($T_{\text{detection}}$)** | 10 – 30 seconds | 1 – 4 hours |
| **Verification ($T_{\text{verification}}$)** | 5 – 20 seconds | 2 – 8 hours |
| **Execution ($T_{\text{execution}}$)** | 10 – 60 seconds (retry queue) | 1 – 2 hours (signer quorum) |
| **Contingency / Outage ($T_{\text{contingency}}$)** | 1 – 2 hours (failover window) | 12 – 24 hours (cross-timezone / sleep) |
| **Total Minimum Window** | **2 hours (7,200s)** | **24 hours (86,400s)** |

### 3.3 Recommendations
- **Automated Deployments with 24/7 Watcher Daemons**: Minimum **2 hours (7,200 seconds)** to **6 hours (21,600 seconds)**.
- **Human-Reviewed Escrows & General Production**: Standard **24 hours (86,400 seconds)**.
- **High-Value / Legal Agreements**: **72 hours (259,200 seconds)** to guarantee coverage across weekends and international holidays.

---

## 4. Parameter Analysis: Resolver Committee Size & Quorum

Tholos v1 enforces strict majority voting among registered resolvers:
`votes_for_asserted > committee_size / 2` or `votes_against > committee_size / 2`.

### 4.1 Trade-Off Matrix

| Committee Size | Quorum Needed | Single-Node Fault Tolerance | Liveness Risk | Collusion Threshold | Operational Suitability |
| :---: | :---: | :---: | :---: | :---: | :--- |
| **3 Members** | 2 votes (66.7%) | **0 members** (if 1 is offline, remaining 2 must both agree) | High | 2 of 3 keys | Low-value testnets / Internal private instances |
| **5 Members** *(Recommended)* | 3 votes (60.0%) | **1 member** (1 offline node still permits 3-of-4 decision) | Low | 3 of 5 keys | **Standard production mainnet baseline** |
| **7 Members** | 4 votes (57.1%) | **2 members** (2 offline nodes still permit 4-of-5 decision) | Very Low | 4 of 7 keys | High TVL / Consortium deployments |

### 4.2 Committee Composition Guidelines
1. **Key Isolation**: Each resolver must operate on an independent machine with independent RPC endpoints (e.g. mix of SDF public RPC, Blockdaemon, and private Horizon nodes).
2. **Geographic Distribution**: Avoid colocating all resolver keys in a single cloud provider or region.
3. **Emergency Rotation**: Maintainers should keep `update_resolvers` admin key in a secure cold/hardware wallet to replace compromised resolvers during an incident, as detailed in [DEPLOYMENT.md](DEPLOYMENT.md).

---

## 5. Parameter Analysis: `finalize_reward_bps`

### 5.1 The Crank Incentive Problem

In Tholos v1, once an assertion's challenge window elapses without a dispute, it does not finalize automatically; an account must call `finalize`.

If `finalize_reward_bps = 0`:
- Only the asserter (who wants their bond and payout released) or the consumer contract has an economic incentive to call `finalize`.
- Third-party keeper bots will not execute transactions that cost gas without compensation.

### 5.2 Sizing the Reward

On Stellar Mainnet:
- Transaction fee for `finalize`: ~100 to 500 stroops ($0.00001 - $0.00005 USD at standard XLM prices).
- To incentivize public keeper bots (e.g., Gelato, OpenZeppelin Defender, or community cron keepers), a net reward of **$0.25 to $1.00 USDC** is sufficient.

```text
Reward = bond_amount * (finalize_reward_bps / 10_000)
```

- At `bond_amount = 100 USDC` (`100_0000000` stroops):
  - Setting `finalize_reward_bps = 50` (0.50%) yields **0.50 USDC** to the keeper.
  - Setting `finalize_reward_bps = 100` (1.00%) yields **1.00 USDC** to the keeper.
- The asserter receives back $99.00 to $99.50 of their 100 USDC bond, treating the 0.5%–1.0% fee as a small cost of guaranteed prompt liquidity release.

---

## 6. Implementation & Deployment Reference

When deploying a production instance on Stellar Mainnet via the Stellar CLI:

```bash
# Production Mainnet Deployment (Profile B: Standard Commercial Escrow)
stellar contract invoke \
  --id "$CONTRACT_ID" \
  --source admin \
  --network mainnet \
  -- initialize \
  --admin "$ADMIN_ADDRESS" \
  --token "$USDC_MAINNET_ADDRESS" \
  --bond_amount 100000000 \
  --challenge_window_secs 86400 \
  --resolvers "[\"$RESOLVER_1\",\"$RESOLVER_2\",\"$RESOLVER_3\",\"$RESOLVER_4\",\"$RESOLVER_5\"]" \
  --finalize_reward_bps 50
```

### Mainnet Readiness Verification:
- [x] Canonical token selected (e.g., Circle USDC `GA5ZSEJY...`)
- [x] `bond_amount` sized above resolver review cost floor ($R_{\text{case}} / K_{\text{spam}}$)
- [x] `challenge_window_secs` sized to at least 24 hours (86,400s) for safe off-chain review
- [x] 5-member committee initialized across distinct infrastructure providers
- [x] `finalize_reward_bps` set to 50 bps to incentivize prompt decentralized finalization

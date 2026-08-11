# V2 canonical claim identifier and evidence convention

> **Status:** Proposed; design-only, not implemented.
>
> **Tracking:** [Issue #76](https://github.com/drydocs/tholos/issues/76).
>
> This document proposes an on-chain reference for what a v2 assertion
> actually claims. It records a recommendation for review; it does not
> change any already-merged v2 code. A follow-up implementation issue is
> opened separately once this is accepted, the same split V2_RESOLUTION.md
> and its own implementation issues (#64-#71) used.

## Why this needs deciding at all

V1 stores only a boolean per assertion (`Assertion.outcome`) and leaves the
actual claim, what the assertion is about, entirely off-chain, tracked by
whichever integrator posted it. That works in v1 because the only people who
ever need to know what an assertion means are the asserter, the disputer,
and the integrator, all of whom coordinated off-chain before any bond was
posted. Nobody else ever has a reason to look at assertion `#42` and form an
opinion about it.

V2 breaks that assumption on purpose: registration (#66) opens voting to an
unbounded set of third parties who were never in contact with the
integrator. For a stranger to rationally lock capital on one side of a
dispute, they need to know what they're actually evaluating, and they need
assurance that what they're shown is the same thing every other voter is
shown, not a claim description that quietly changed after some voters
already committed. V2_RESOLUTION.md's own threat table already names this
gap explicitly: an open electorate needs an unambiguous immutable reference
to the proposition, and flagged it as a decision deferred to a later issue.
This is that issue.

## Decision summary

| Question | Proposed answer |
| --- | --- |
| On-chain identifier or off-chain registry only? | A mandatory on-chain content hash. An off-chain-only registry (v1's model) doesn't give third-party voters any way to verify they're all looking at the same claim. |
| What does the hash commit to? | The canonical encoding of an off-chain claim document (format is the integrator's choice: prose, structured JSON, whatever fits their domain). The contract never inspects the content, only stores and exposes the hash. |
| Format | `BytesN<32>`, same shape as `PolicySnapshotV2.policy_hash` already uses. Reuses an established, already-tested pattern in this crate rather than inventing a new one. |
| Discoverability | An optional URI string alongside the hash, best-effort only, not verified by the contract. The hash is authoritative; the URI is a convenience pointer to where the matching content currently lives. |
| Structured on-chain schema? | No. Forcing claims into a rigid on-chain schema doesn't scale across integrator domains and turns Tholos from a generic primitive into an opinionated claims format. The off-chain document the hash commits to can be as structured as the integrator needs. |
| Validation at `assert_outcome` time | Reject an all-zero hash (a sentinel for "no claim specified"); every assertion must commit to *something*. The contract cannot and does not verify the hash corresponds to real, fetchable content, that's unverifiable on-chain by construction and is a client-side/voter-side concern. |
| Evidence convention | Stays event-only, not stored in persistent state. `dispute` and `register` (once retrofitted; see below) gain an optional evidence hint included in their events, so supporting arguments are indexable without contract storage bloat. Not required, never validated. |

## Reasoning

**Why a hash, not a plain URI.** A URI is a pointer that can rot or be
silently edited after the fact: nothing stops content at a URL from
changing between when an early voter reads it and when a late voter does. A
content hash pins the exact bytes at assertion-creation time; anyone can
fetch the claim from anywhere, it doesn't matter if it's IPFS, a integrator's
own server, or a GitHub gist, and confirm locally that it hashes to what's
on-chain. This is the same content-addressing pattern IPFS CIDs use, and
it's already precedented in this exact crate: `PolicySnapshotV2.policy_hash`
already does this for deployment parameters. Reusing that pattern here means
no new cryptographic primitive, no new audit surface beyond what's already
been reviewed for `policy_hash`.

**Why not a structured on-chain schema.** A freelance milestone claim, an
insurance claim, and a sports-result claim have nothing structurally in
common beyond "here is a proposition, and a bond backs an assertion about
it." Encoding that variety on-chain would mean either a schema so generic
it carries no real validation value, or a schema so specific it only fits
one integrator's domain. The hash approach pushes structure to where it
belongs: the integrator's own off-chain document, in whatever shape fits
their use case, while the contract's only job is pinning it immutably.

**Why the contract can't validate the hash's content.** This is a hard
limit, not a design choice: a Soroban contract has no network access and
cannot fetch arbitrary off-chain data. The only thing `assert_outcome` can
meaningfully check is that a hash was actually supplied (non-zero), not that
it corresponds to real, comprehensible content. A voter's own client is
responsible for fetching the claimed content, hashing it, and confirming the
match before a human decides how to vote, the same way a browser
verifying a subresource-integrity hash doesn't guarantee the *content* makes
sense, only that it's the content the page author committed to.

**Why evidence stays off-chain and event-only.** Evidence (why a disputer
thinks an assertion is wrong, why a voter revealed the way they did) is
inherently unstructured, arbitrarily sized human argumentation. Storing it
in persistent contract state would be expensive and unbounded in a way the
claim hash isn't (one hash per assertion is a fixed, small cost; evidence
could be arbitrarily long and arbitrarily frequent). Events are the right
fit: cheap, already indexed by the events every v2 write emits (#72's TTL
issue already requires this event discipline for a different reason), and
naturally queryable by an off-chain indexer without bloating storage that
has to carry a TTL.

## What changes in already-merged code

`AssertionV2` (#64) needs one new field: `claim_hash: BytesN<32>`, supplied
as a new `assert_outcome` (#65) parameter and validated non-zero. An
optional `claim_uri: Option<...>` could ride alongside it in the `Asserted`
event only (not stored, matching the evidence convention above), since a
`String`/`Bytes` type wrapped in `Option` is a built-in type, not a custom
enum, so it doesn't hit the `Option<EnumType>` limitation #64 and #66 already
documented.

`dispute` and `register` (#66) each gain an optional evidence hint parameter,
included in their existing `Disputed` / `PositionFunded` events, not in the
`Position` or `Resolution` records themselves.

None of this is implemented by this issue. A follow-up implementation issue,
opened once this design is reviewed and accepted, makes these changes against
the already-merged #64/#65/#66 code, the same way #64's own "Future
implementation work" list became #64-#71 after V2_RESOLUTION.md was
accepted.

## Alternatives considered

| Alternative | Why it is not recommended |
| --- | --- |
| Off-chain registry only (v1's model), no on-chain anchor | Doesn't give third-party voters any way to confirm they're all evaluating the same claim; directly contradicts the reason this issue exists. |
| Plain URI, no hash | A URI can change or disappear after voters have already committed capital based on what it showed at the time. |
| Full structured on-chain claim schema | Doesn't generalize across integrator domains; turns a generic primitive into an opinionated format; meaningfully larger audit and storage surface for no integrity benefit over a hash. |
| Contract-side content validation | Not possible: Soroban contracts have no network access to fetch and check off-chain content. |
| Evidence stored in persistent state | Unbounded, arbitrarily-sized human text is a poor fit for storage that carries a TTL and a per-byte cost; events already solve the indexability need without that cost. |

## Questions for review

1. Should `claim_uri` be validated for basic well-formedness (e.g., a
   length cap) even though its content is never verified, purely to bound
   event payload size? Leaning yes, a generous but finite cap (a exact
   number is an implementation-issue detail, not a decision to lock in
   here).
2. Should the evidence hint on `dispute`/`register` be a hash (pointing at
   off-chain evidence, mirroring the claim) or a plain URI (cheaper, but
   without the same tamper-evidence guarantee)? Leaning toward a plain URI:
   evidence is inherently supplementary and informal, not something a voter
   is trusting the same way they trust the claim itself, so the stronger
   hash guarantee is likely not worth the added complexity here.

Until these are resolved and this proposal is accepted, no implementation
issue should treat the "What changes in already-merged code" section above
as final.

# Security policy

## Status

Tholos has not had an external security audit. It has undergone one internal
review pass, which found and fixed a real reentrancy vulnerability (see
[CHANGELOG.md](docs/src/CHANGELOG.md) and the "Security notes" section of
[CONTRACT.md](docs/src/CONTRACT.md)). Treat it as pre-production software: appropriate for
testnet use and further review, not for deployments securing meaningful value on
mainnet until it has been audited.

## Reporting a vulnerability

**Do not open a public GitHub issue for a security vulnerability.**

Report it privately via [GitHub's private vulnerability reporting](https://github.com/drydocs/tholos/security/advisories/new)
on this repository. Include:

- A description of the vulnerability and its impact
- Steps to reproduce, or a proof of concept
- The affected contract(s) and function(s)
- A suggested fix, if you have one

You should expect an initial response within 7 days. Please allow time for the
issue to be triaged and, where applicable, patched before any public disclosure.

## Scope

In scope: `contracts/tholos`, `contracts/tholos-v2`, and `contracts/asserter-consumer`.

`contracts/asserter-consumer` is, like `contracts/demo-consumer`, an integration
example rather than a production deployment — but it demonstrates a materially
different, more security-sensitive pattern. Where `demo-consumer` has the end
user sign and authorize the assertion directly, `asserter-consumer` uses the
`authorize_as_current_contract` pattern from
[INTEGRATION.md](docs/src/INTEGRATION.md), where the contract self-authorizes
a fund transfer on its own behalf with no human signer in the loop. That
self-authorization construction is exactly what a real integrator is expected
to copy into production, so a vulnerability in this reference implementation
carries direct downstream security impact even though the example itself
never holds real value. `demo-consumer`'s pattern has no comparable surface to
get wrong, which is why it stays out of scope below.

Out of scope: third-party dependencies (`soroban-sdk`, the Stellar network
itself), and `contracts/demo-consumer`.

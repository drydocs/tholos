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

In scope: the contracts under `contracts/` in this repository. Out of scope:
third-party dependencies (`soroban-sdk`, the Stellar network itself), and the
`contracts/demo-consumer` example, which exists to validate integration patterns
and is not intended for production use on its own.

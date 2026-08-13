//! Off-chain helper for `register`/`reveal`'s salted vote commitment.
//! `register` only ever sees the commitment hash, never the underlying
//! choice/salt, so a caller (a script, a test identity, a real integrator)
//! has to compute `H(preimage)` itself before calling `register`, exactly
//! the same way `reveal` recomputes and checks it on-chain. This binary
//! does that computation using `contracts/tholos-v2`'s own
//! `VoteCommitmentPreimage` type (a path dependency, not a copy), so it
//! can never drift from what `reveal` actually verifies.
//!
//! A separate crate from `tholos-v2` itself, not `tholos-v2/src/bin/`:
//! `stellar contract build` invokes `cargo rustc --crate-type=cdylib`
//! scoped to one target per package, which errors on an ambiguous
//! "which target" choice if a package has both a lib and a bin. Keeping
//! this crate separate means `tholos-v2`'s own package still has exactly
//! one target, and this crate (no `[lib]`, no cdylib type at all) is
//! simply skipped by `stellar contract build`'s "every crate with
//! crate-type cdylib" selection.
//!
//! `scripts/testnet-load-v2.sh` is the only current caller.
//!
//! Usage:
//! ```text
//! cargo run -p compute-commitment -- \
//!   <network_id_hex> <contract_address> <policy_hash_hex> <assertion_id> \
//!   <voter_address> <true|false> <salt_hex>
//! ```
//! Prints the resulting commitment as a lowercase hex string on stdout.

use soroban_sdk::{xdr::ToXdr, Address, BytesN, Env, Symbol};
use std::env as std_env;
use tholos_v2::VoteCommitmentPreimage;

fn decode_hex_32(hex: &str) -> [u8; 32] {
    assert!(
        hex.len() == 64,
        "expected 64 hex characters (32 bytes), got {}",
        hex.len()
    );
    let mut bytes = [0u8; 32];
    for (i, byte) in bytes.iter_mut().enumerate() {
        let pair = &hex[i * 2..i * 2 + 2];
        *byte = u8::from_str_radix(pair, 16).expect("invalid hex digit");
    }
    bytes
}

fn encode_hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn main() {
    let args: Vec<String> = std_env::args().collect();
    if args.len() != 8 {
        eprintln!(
            "usage: compute-commitment <network_id_hex> <contract_address> \
             <policy_hash_hex> <assertion_id> <voter_address> <true|false> <salt_hex>"
        );
        std::process::exit(1);
    }
    let network_id_hex = &args[1];
    let contract_address_str = &args[2];
    let policy_hash_hex = &args[3];
    let assertion_id: u64 = args[4].parse().expect("assertion_id must be a u64");
    let voter_str = &args[5];
    let choice: bool = args[6].parse().expect("choice must be true or false");
    let salt_hex = &args[7];

    // A bare Env, never connected to any network: to_xdr()/sha256() are
    // pure computations over already-known values, the same reason
    // contracts/tholos-v2/src/test.rs's compute_commitment() test helper
    // can use one too.
    let env = Env::default();

    let preimage = VoteCommitmentPreimage {
        domain: Symbol::new(&env, "THOLOS_V2_VOTE"),
        network_id: BytesN::from_array(&env, &decode_hex_32(network_id_hex)),
        contract_address: Address::from_str(&env, contract_address_str),
        policy_hash: BytesN::from_array(&env, &decode_hex_32(policy_hash_hex)),
        assertion_id,
        // This proposal has exactly one weighted round (see ROUND in
        // contracts/tholos-v2/src/lib.rs); hardcoded here since ROUND
        // itself is a private constant not worth exposing just for this
        // tool.
        round: 0,
        voter: Address::from_str(&env, voter_str),
        choice,
        salt: BytesN::from_array(&env, &decode_hex_32(salt_hex)),
    };

    let commitment: BytesN<32> = env.crypto().sha256(&preimage.to_xdr(&env)).into();
    println!("{}", encode_hex(&commitment.to_array()));
}

#!/usr/bin/env python3
"""LODZ vault -- Anchor discriminator calculator and IDL cross-check.

Every discriminator here is computed with sha256. None of them is copied,
remembered or guessed. A guessed instruction discriminator does not fail
loudly: the program answers InstructionFallbackNotFound (101) at run time, on
chain, after the fee has been paid. That failure is recorded in
new_project_guide/references/solana/anchor-lessons.md and this script exists so
it cannot happen again.

Anchor 0.31 rules:

    instruction  sha256("global:<snake_case_name>")[:8]
    account      sha256("account:<PascalCaseName>")[:8]
    event        sha256("event:<PascalCaseName>")[:8]

Usage:

    python3 scripts/discriminators.py            # table, plus IDL cross-check
    python3 scripts/discriminators.py --json     # machine readable
    python3 scripts/discriminators.py --idl PATH # cross-check a specific IDL

Exit status is non-zero when the computed set and the IDL disagree, so this is
safe to wire into a check step. It never touches a cluster and never reads a
keypair.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import sys
from pathlib import Path

PACKAGE_ROOT = Path(__file__).resolve().parent.parent
DEFAULT_IDL = PACKAGE_ROOT / "target" / "idl" / "lodz_vault.json"

# ---------------------------------------------------------------------------
# The program's public surface. Source of truth is programs/lodz-vault/src.
# ---------------------------------------------------------------------------

INSTRUCTIONS = [
    # setup
    "initialize_vault",
    "initialize_bond_vault",
    "register_adit",
    "open_stope",
    "register_seam",
    # circuit breaker
    "pause_vault",
    "unpause_vault",
    # authority handover
    "propose_authority",
    "accept_authority",
    # depositors
    "deposit",
    "request_redemption",
    "claim_redemption",
    # keepers
    "bond_keeper",
    "unbond_keeper",
    "update_seam_allocation",
    "accrue_yield",
    # enforcement
    "slash_keeper",
]

ACCOUNTS = [
    "VaultConfig",
    "Adit",
    "Stope",
    "Seam",
    "Miner",
    "Orecart",
    "OrecartQueue",
    "Keeper",
]

EVENTS = [
    "VaultInitialized",
    "BondVaultInitialized",
    "AditRegistered",
    "StopeOpened",
    "SeamRegistered",
    "SeamRebalanced",
    "Deposit",
    "YieldAccrued",
    "RedemptionRequested",
    "RedemptionClaimed",
    "KeeperBonded",
    "KeeperUnbonded",
    "KeeperSlashed",
    "VaultPauseChanged",
    "AuthorityTransferProposed",
    "AuthorityTransferAccepted",
]

# `space = 8 + LEN`. Asserted against the structs by
# `state::tests::account_sizes_match_the_readme_table`.
ACCOUNT_LEN = {
    "VaultConfig": 288,
    "Adit": 200,
    "Stope": 184,
    "Seam": 256,
    "Miner": 184,
    "Orecart": 192,
    "OrecartQueue": 104,
    "Keeper": 120,
}

# PDA seeds. Numeric seeds are little-endian everywhere: on-chain, in the SDK,
# in the service indexer and here.
PDA_SEEDS = [
    ("vault_config", ['b"vault_config"']),
    ("adit", ['b"adit"', "asset_mint (32)"]),
    ("adit_vault", ['b"adit_vault"', "asset_mint (32)"]),
    ("bond_vault", ['b"bond_vault"']),
    ("stope", ['b"stope"', "stope_id (u8 LE)"]),
    ("seam", ['b"seam"', "seam_id (u16 LE)"]),
    ("miner", ['b"miner"', "owner (32)", "stope_id (u8 LE)"]),
    # stope_id is required here. The counter that `ticket_index` must match,
    # Miner::ticket_count, lives on a per-stope account, so without stope_id in
    # this seed a depositor holding two stopes resolves both positions to the
    # same ticket address: the second one can never be redeemed, and the
    # counter never advances because it only advances on success. Measured on
    # devnet 2026-08-16.
    ("orecart", ['b"orecart"', "owner (32)", "stope_id (u8 LE)", "ticket_index (u32 LE)"]),
    ("orecart_queue", ['b"orecart_queue"', "stope_id (u8 LE)"]),
    ("keeper", ['b"keeper"', "authority (32)"]),
]


def discriminator(prefix: str, name: str) -> bytes:
    return hashlib.sha256(f"{prefix}:{name}".encode("utf-8")).digest()[:8]


def as_row(disc: bytes) -> tuple[list[int], str]:
    return list(disc), disc.hex()


def compute() -> dict:
    return {
        "instructions": {n: as_row(discriminator("global", n))[0] for n in INSTRUCTIONS},
        "accounts": {n: as_row(discriminator("account", n))[0] for n in ACCOUNTS},
        "events": {n: as_row(discriminator("event", n))[0] for n in EVENTS},
    }


def print_group(title: str, prefix: str, names: list[str]) -> None:
    width = max(len(n) for n in names)
    print(f"\n{title}  (sha256(\"{prefix}:<name>\")[:8])")
    print("-" * (width + 62))
    for name in names:
        disc = discriminator(prefix, name)
        byte_list = ", ".join(str(b) for b in disc)
        print(f"  {name:<{width}}  {disc.hex()}  [{byte_list}]")


def print_sizes() -> None:
    width = max(len(n) for n in ACCOUNT_LEN)
    print("\nAccount sizes  (space = 8 discriminator + LEN)")
    print("-" * (width + 34))
    total = 0
    for name, length in ACCOUNT_LEN.items():
        space = 8 + length
        total += space
        print(f"  {name:<{width}}  LEN {length:>4}   space {space:>4}")
    print(f"  {'':<{width}}  {'':>8}   total {total:>4}")


def print_seeds() -> None:
    width = max(len(n) for n, _ in PDA_SEEDS)
    print("\nPDA seeds  (numeric seeds are ALWAYS little-endian)")
    print("-" * (width + 60))
    for name, seeds in PDA_SEEDS:
        print(f"  {name:<{width}}  [{', '.join(seeds)}]")


def cross_check(idl_path: Path) -> int:
    """Compare the computed set against a generated IDL. Returns an exit code."""
    if not idl_path.exists():
        print(f"\nIDL cross-check SKIPPED: {idl_path} not found (run `anchor build`).")
        return 0

    idl = json.loads(idl_path.read_text())
    failures: list[str] = []

    def check(kind: str, prefix: str, expected_names: list[str]) -> None:
        entries = idl.get(kind, [])
        by_name = {e["name"]: e for e in entries}

        missing = [n for n in expected_names if n not in by_name]
        extra = [n for n in by_name if n not in expected_names]
        for name in missing:
            failures.append(f"{kind}: {name} is in this script but not in the IDL")
        for name in extra:
            failures.append(f"{kind}: {name} is in the IDL but not in this script")

        for name in expected_names:
            entry = by_name.get(name)
            if entry is None:
                continue
            # Anchor 0.31 emits the discriminator explicitly. Older IDLs do not.
            idl_disc = entry.get("discriminator")
            if idl_disc is None:
                failures.append(f"{kind}: {name} has no discriminator in the IDL")
                continue
            computed = list(discriminator(prefix, name))
            if list(idl_disc) != computed:
                failures.append(
                    f"{kind}: {name} IDL {list(idl_disc)} != computed {computed}"
                )

    check("instructions", "global", INSTRUCTIONS)
    check("accounts", "account", ACCOUNTS)
    check("events", "event", EVENTS)

    print(f"\nIDL cross-check against {idl_path}")
    print(f"  program address : {idl.get('address')}")
    print(f"  instructions    : {len(idl.get('instructions', []))}")
    print(f"  accounts        : {len(idl.get('accounts', []))}")
    print(f"  events          : {len(idl.get('events', []))}")
    print(f"  errors          : {len(idl.get('errors', []))}")
    print(f"  types           : {len(idl.get('types', []))}")

    if failures:
        print("\n  RESULT: FAIL")
        for line in failures:
            print(f"    - {line}")
        return 1

    print("\n  RESULT: PASS -- every computed discriminator matches the IDL.")
    return 0


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--json", action="store_true", help="emit JSON only")
    parser.add_argument(
        "--idl",
        type=Path,
        default=DEFAULT_IDL,
        help="IDL to cross-check against (default: target/idl/lodz_vault.json)",
    )
    args = parser.parse_args()

    if args.json:
        print(json.dumps(compute(), indent=2))
        return 0

    print("LODZ vault -- Anchor discriminators (computed with sha256, never guessed)")
    print_group("Instructions", "global", INSTRUCTIONS)
    print_group("Accounts", "account", ACCOUNTS)
    print_group("Events", "event", EVENTS)
    print_sizes()
    print_seeds()
    return cross_check(args.idl)


if __name__ == "__main__":
    sys.exit(main())

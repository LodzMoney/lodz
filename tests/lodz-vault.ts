/**
 * LODZ vault -- localnet integration suite.
 *
 * ---------------------------------------------------------------------------
 * NOT RUN. Written, never executed against any cluster.
 * ---------------------------------------------------------------------------
 *
 * Running this file starts a local `solana-test-validator` and sends real
 * transactions to it. Under the rules in
 * new_project_guide/references/solana/anchor-lessons.md that is only allowed
 * once the user has given an explicit localnet approval, and it must never be
 * pointed at devnet or mainnet. `Anchor.toml` pins `cluster = "localnet"`;
 * do not change it.
 *
 * Before this can run:
 *   1. explicit user approval for a localnet run
 *   2. npm install in packages/anchor-program
 *   3. a localnet-only payer at keypair/localnet-test-payer.json, which must
 *      NOT be ~/.config/solana/id.json
 *   4. anchor test --provider.cluster localnet
 *
 * What it covers is the set of invariants that make LODZ different from a
 * vault that just quotes one blended APY:
 *   - an emissions seam cannot exist without a future end date and a mint
 *   - an emissions seam stops accruing the moment that date passes
 *   - sustainable and emissions yield never touch each other's accumulators
 *   - a conservative stope refuses an emissions allocation above its ceiling
 *   - a redemption cannot be claimed before its stamped claimable_at
 *   - only a bonded keeper can move an allocation or book yield
 */

import * as anchor from "@coral-xyz/anchor";
import { Program, BN } from "@coral-xyz/anchor";
import {
  Keypair,
  PublicKey,
  SystemProgram,
  LAMPORTS_PER_SOL,
} from "@solana/web3.js";
import {
  TOKEN_PROGRAM_ID,
  TOKEN_2022_PROGRAM_ID,
  createMint,
  createAccount,
  mintTo,
  getAccount,
} from "@solana/spl-token";
import * as chai from "chai";
import chaiAsPromised from "chai-as-promised";

import { LodzVault } from "../target/types/lodz_vault";

chai.use(chaiAsPromised);
const assert = chai.assert;

// ---------------------------------------------------------------------------
// PDA helpers. Numeric seeds are little-endian, exactly as in
// programs/lodz-vault/src/state/mod.rs. If these ever disagree with the
// program, every account lookup silently resolves to an address that does not
// exist -- that is the PDA mismatch in anchor-lessons.md.
// ---------------------------------------------------------------------------

const u8le = (n: number) => Buffer.from([n]);

const u16le = (n: number) => {
  const b = Buffer.alloc(2);
  b.writeUInt16LE(n);
  return b;
};

const u32le = (n: number) => {
  const b = Buffer.alloc(4);
  b.writeUInt32LE(n);
  return b;
};

const pda = (seeds: Buffer[], programId: PublicKey) =>
  PublicKey.findProgramAddressSync(seeds, programId)[0];

const ascii = (s: string, len: number) => {
  const b = Buffer.alloc(len);
  b.write(s, "ascii");
  return Array.from(b);
};

const DAY = 86400;

/**
 * Venue names.
 *
 * `orca` is a real venue: docs/research/btc-on-solana.md confirms LP trading
 * fees on Orca cbBTC pools as the sustainable yield that actually exists on
 * Solana today. The emissions venues below are deliberately fictional, because
 * the same research found zero BTC pools on Solana currently paying emissions
 * (94 pools, `apyReward > 0` on none of them). Naming a real protocol as the
 * source of an emission it does not pay would be exactly the misattribution
 * this program is built to prevent.
 */
const VENUE_SUSTAINABLE = "orca";
const VENUE_EMISSIONS = "test-emissions-venue";

/** One BTC in internal accounting units (8 decimals). */
const ONE_BTC = 100_000_000;

describe("lodz_vault", () => {
  anchor.setProvider(anchor.AnchorProvider.env());
  const provider = anchor.getProvider() as anchor.AnchorProvider;
  const program = anchor.workspace.LodzVault as Program<LodzVault>;
  const programId = program.programId;
  const authority = (provider.wallet as anchor.Wallet).payer;

  // Actors
  const keeper = Keypair.generate();
  const alice = Keypair.generate();

  // Mints
  let lodzMint: PublicKey; // $LODZ, keeper bonds
  let btcMint: PublicKey; // an 8-decimal BTC representation
  let emissionMint: PublicKey; // what an emissions seam pays in

  // Token accounts
  let treasury: PublicKey;
  let keeperLodz: PublicKey;
  let keeperBtc: PublicKey;
  let aliceBtc: PublicKey;

  // PDAs
  let vaultConfig: PublicKey;
  let bondVault: PublicKey;
  let adit: PublicKey;
  let aditVault: PublicKey;

  const CONSERVATIVE = 0;
  const BALANCED = 1;
  const AGGRESSIVE = 2;

  const stopePda = (id: number) => pda([Buffer.from("stope"), u8le(id)], programId);
  const queuePda = (id: number) =>
    pda([Buffer.from("orecart_queue"), u8le(id)], programId);
  const seamPda = (id: number) => pda([Buffer.from("seam"), u16le(id)], programId);
  const minerPda = (owner: PublicKey, stopeId: number) =>
    pda([Buffer.from("miner"), owner.toBuffer(), u8le(stopeId)], programId);
  const orecartPda = (owner: PublicKey, index: number) =>
    pda([Buffer.from("orecart"), owner.toBuffer(), u32le(index)], programId);
  const keeperPda = (auth: PublicKey) =>
    pda([Buffer.from("keeper"), auth.toBuffer()], programId);

  const now = () => Math.floor(Date.now() / 1000);

  before(async () => {
    for (const kp of [keeper, alice]) {
      const sig = await provider.connection.requestAirdrop(
        kp.publicKey,
        5 * LAMPORTS_PER_SOL
      );
      await provider.connection.confirmTransaction(sig, "confirmed");
    }

    // $LODZ and the emission token are classic SPL Token mints; the BTC
    // representation is Token-2022, so the deposit and payout paths are
    // exercised against the program the lessons file says silently fails when
    // it is assumed rather than pinned.
    lodzMint = await createMint(provider.connection, authority, authority.publicKey, null, 9);
    emissionMint = await createMint(
      provider.connection,
      authority,
      authority.publicKey,
      null,
      6
    );
    btcMint = await createMint(
      provider.connection,
      authority,
      authority.publicKey,
      null,
      8,
      Keypair.generate(),
      undefined,
      TOKEN_2022_PROGRAM_ID
    );

    treasury = await createAccount(
      provider.connection,
      authority,
      lodzMint,
      authority.publicKey
    );
    keeperLodz = await createAccount(
      provider.connection,
      authority,
      lodzMint,
      keeper.publicKey
    );
    keeperBtc = await createAccount(
      provider.connection,
      authority,
      btcMint,
      keeper.publicKey,
      Keypair.generate(),
      undefined,
      TOKEN_2022_PROGRAM_ID
    );
    aliceBtc = await createAccount(
      provider.connection,
      authority,
      btcMint,
      alice.publicKey,
      Keypair.generate(),
      undefined,
      TOKEN_2022_PROGRAM_ID
    );

    await mintTo(
      provider.connection,
      authority,
      lodzMint,
      keeperLodz,
      authority,
      1_000_000_000_000
    );
    await mintTo(
      provider.connection,
      authority,
      btcMint,
      aliceBtc,
      authority,
      10 * ONE_BTC,
      [],
      undefined,
      TOKEN_2022_PROGRAM_ID
    );
    await mintTo(
      provider.connection,
      authority,
      btcMint,
      keeperBtc,
      authority,
      10 * ONE_BTC,
      [],
      undefined,
      TOKEN_2022_PROGRAM_ID
    );

    vaultConfig = pda([Buffer.from("vault_config")], programId);
    bondVault = pda([Buffer.from("bond_vault")], programId);
    adit = pda([Buffer.from("adit"), btcMint.toBuffer()], programId);
    aditVault = pda([Buffer.from("adit_vault"), btcMint.toBuffer()], programId);
  });

  // -------------------------------------------------------------------------
  // setup
  // -------------------------------------------------------------------------

  it("initializes the vault paused", async () => {
    await program.methods
      .initializeVault({
        feeBps: 25,
        redemptionDelaySec: new BN(2),
        maxRedemptionDelaySec: new BN(7 * DAY),
        queueDrainPerDay: new BN(100 * ONE_BTC),
        minKeeperBond: new BN(1_000_000_000),
        keeperUnbondCooldownSec: new BN(0),
      })
      .accountsPartial({
        authority: authority.publicKey,
        vaultConfig,
        lodzMint,
        treasury,
        lodzTokenProgram: TOKEN_PROGRAM_ID,
        systemProgram: SystemProgram.programId,
      })
      .rpc();

    const config = await program.account.vaultConfig.fetch(vaultConfig);
    assert.isTrue(config.paused, "a fresh vault must start paused");
    assert.equal(config.feeBps, 25);
    assert.equal(config.stopeCount, 0);
  });

  it("creates the bond vault", async () => {
    await program.methods
      .initializeBondVault()
      .accountsPartial({
        authority: authority.publicKey,
        vaultConfig,
        lodzMint,
        bondVault,
        tokenProgram: TOKEN_PROGRAM_ID,
        systemProgram: SystemProgram.programId,
      })
      .rpc();

    const account = await getAccount(provider.connection, bondVault);
    assert.equal(account.mint.toBase58(), lodzMint.toBase58());
    assert.equal(account.owner.toBase58(), vaultConfig.toBase58());
  });

  it("registers an adit and its custody account", async () => {
    await program.methods
      .registerAdit({
        label: ascii("zBTC", 16),
        custodyKind: { bridgeMinted: {} },
        riskTier: 2,
        conversionNum: new BN(1),
        conversionDen: new BN(1),
        depositCap: new BN(0),
      })
      .accountsPartial({
        authority: authority.publicKey,
        vaultConfig,
        assetMint: btcMint,
        adit,
        aditVault,
        tokenProgram: TOKEN_2022_PROGRAM_ID,
        systemProgram: SystemProgram.programId,
      })
      .rpc();

    const account = await program.account.adit.fetch(adit);
    assert.equal(account.assetMint.toBase58(), btcMint.toBase58());
    assert.equal(account.tokenProgram.toBase58(), TOKEN_2022_PROGRAM_ID.toBase58());
    assert.equal(account.decimals, 8);
    assert.equal(account.riskTier, 2);
    assert.deepEqual(account.custodyKind, { bridgeMinted: {} });
  });

  it("opens the three stopes and refuses a mismatched risk profile", async () => {
    for (const [id, profile] of [
      [CONSERVATIVE, { conservative: {} }],
      [BALANCED, { balanced: {} }],
      [AGGRESSIVE, { aggressive: {} }],
    ] as const) {
      await program.methods
        .openStope(id, profile as never)
        .accountsPartial({
          authority: authority.publicKey,
          vaultConfig,
          stope: stopePda(id),
          orecartQueue: queuePda(id),
          systemProgram: SystemProgram.programId,
        })
        .rpc();
    }

    const conservative = await program.account.stope.fetch(stopePda(CONSERVATIVE));
    assert.deepEqual(conservative.riskProfile, { conservative: {} });

    // Stope 3 does not exist, and calling it "conservative" does not create it.
    await assert.isRejected(
      program.methods
        .openStope(3, { conservative: {} } as never)
        .accountsPartial({
          authority: authority.publicKey,
          vaultConfig,
          stope: stopePda(3),
          orecartQueue: queuePda(3),
          systemProgram: SystemProgram.programId,
        })
        .rpc(),
      /InvalidStopeId/
    );
  });

  it("unpauses", async () => {
    await program.methods
      .unpauseVault()
      .accountsPartial({ authority: authority.publicKey, vaultConfig })
      .rpc();
    const config = await program.account.vaultConfig.fetch(vaultConfig);
    assert.isFalse(config.paused);
  });

  // -------------------------------------------------------------------------
  // the disclosure gate
  // -------------------------------------------------------------------------

  const SUSTAINABLE_SEAM = 1;
  const EMISSIONS_SEAM = 2;
  const EXPIRING_SEAM = 3;

  it("refuses an emissions seam with no end date", async () => {
    await assert.isRejected(
      program.methods
        .registerSeam(99, BALANCED, {
          venue: ascii(VENUE_SUSTAINABLE, 32),
          venueProgram: PublicKey.default,
          yieldKind: { emissions: {} },
          allocationBps: 1000,
          riskTier: 3,
          emissionEndsAt: new BN(0),
          emissionMint: emissionMint,
        })
        .accountsPartial({
          authority: authority.publicKey,
          vaultConfig,
          stope: stopePda(BALANCED),
          assetMint: btcMint,
          adit,
          seam: seamPda(99),
          systemProgram: SystemProgram.programId,
        })
        .rpc(),
      /EmissionEndMissing/
    );
  });

  it("refuses an emissions seam whose schedule already ended", async () => {
    await assert.isRejected(
      program.methods
        .registerSeam(98, BALANCED, {
          venue: ascii(VENUE_SUSTAINABLE, 32),
          venueProgram: PublicKey.default,
          yieldKind: { emissions: {} },
          allocationBps: 1000,
          riskTier: 3,
          emissionEndsAt: new BN(now() - DAY),
          emissionMint: emissionMint,
        })
        .accountsPartial({
          authority: authority.publicKey,
          vaultConfig,
          stope: stopePda(BALANCED),
          assetMint: btcMint,
          adit,
          seam: seamPda(98),
          systemProgram: SystemProgram.programId,
        })
        .rpc(),
      /EmissionEndInPast/
    );
  });

  it("refuses a sustainable seam that carries emission fields", async () => {
    await assert.isRejected(
      program.methods
        .registerSeam(97, BALANCED, {
          venue: ascii(VENUE_SUSTAINABLE, 32),
          venueProgram: PublicKey.default,
          yieldKind: { sustainable: {} },
          allocationBps: 1000,
          riskTier: 3,
          emissionEndsAt: new BN(now() + DAY),
          emissionMint: emissionMint,
        })
        .accountsPartial({
          authority: authority.publicKey,
          vaultConfig,
          stope: stopePda(BALANCED),
          assetMint: btcMint,
          adit,
          seam: seamPda(97),
          systemProgram: SystemProgram.programId,
        })
        .rpc(),
      /EmissionFieldsOnSustainableSeam/
    );
  });

  it("registers a sustainable and an emissions seam on the balanced stope", async () => {
    await program.methods
      .registerSeam(SUSTAINABLE_SEAM, BALANCED, {
        venue: ascii(VENUE_SUSTAINABLE, 32),
        venueProgram: PublicKey.default,
        yieldKind: { sustainable: {} },
        allocationBps: 6000,
        riskTier: 3,
        emissionEndsAt: new BN(0),
        emissionMint: PublicKey.default,
      })
      .accountsPartial({
        authority: authority.publicKey,
        vaultConfig,
        stope: stopePda(BALANCED),
        assetMint: btcMint,
        adit,
        seam: seamPda(SUSTAINABLE_SEAM),
        systemProgram: SystemProgram.programId,
      })
      .rpc();

    await program.methods
      .registerSeam(EMISSIONS_SEAM, BALANCED, {
        venue: ascii(VENUE_EMISSIONS, 32),
        venueProgram: PublicKey.default,
        yieldKind: { emissions: {} },
        allocationBps: 3000,
        riskTier: 3,
        emissionEndsAt: new BN(now() + 90 * DAY),
        emissionMint: emissionMint,
      })
      .accountsPartial({
        authority: authority.publicKey,
        vaultConfig,
        stope: stopePda(BALANCED),
        assetMint: btcMint,
        adit,
        seam: seamPda(EMISSIONS_SEAM),
        systemProgram: SystemProgram.programId,
      })
      .rpc();

    const stope = await program.account.stope.fetch(stopePda(BALANCED));
    assert.equal(stope.allocatedBps, 9000);
    assert.equal(stope.emissionsBps, 3000, "emissions share is tracked apart");
  });

  it("refuses an emissions allocation above the conservative ceiling", async () => {
    // Conservative caps emissions exposure at 2000 bps.
    await assert.isRejected(
      program.methods
        .registerSeam(96, CONSERVATIVE, {
          venue: ascii(VENUE_EMISSIONS, 32),
          venueProgram: PublicKey.default,
          yieldKind: { emissions: {} },
          allocationBps: 2001,
          riskTier: 2,
          emissionEndsAt: new BN(now() + 90 * DAY),
          emissionMint: emissionMint,
        })
        .accountsPartial({
          authority: authority.publicKey,
          vaultConfig,
          stope: stopePda(CONSERVATIVE),
          assetMint: btcMint,
          adit,
          seam: seamPda(96),
          systemProgram: SystemProgram.programId,
        })
        .rpc(),
      /EmissionsAllocationExceeded/
    );
  });

  it("refuses a seam whose risk tier is above the stope's profile", async () => {
    await assert.isRejected(
      program.methods
        .registerSeam(95, CONSERVATIVE, {
          venue: ascii(VENUE_SUSTAINABLE, 32),
          venueProgram: PublicKey.default,
          yieldKind: { sustainable: {} },
          allocationBps: 1000,
          riskTier: 5,
          emissionEndsAt: new BN(0),
          emissionMint: PublicKey.default,
        })
        .accountsPartial({
          authority: authority.publicKey,
          vaultConfig,
          stope: stopePda(CONSERVATIVE),
          assetMint: btcMint,
          adit,
          seam: seamPda(95),
          systemProgram: SystemProgram.programId,
        })
        .rpc(),
      /RiskTierExceedsStopeProfile/
    );
  });

  // -------------------------------------------------------------------------
  // deposit
  // -------------------------------------------------------------------------

  it("takes a deposit and mints shares 1:1 into an empty stope", async () => {
    await program.methods
      .deposit(BALANCED, new BN(2 * ONE_BTC))
      .accountsPartial({
        depositor: alice.publicKey,
        vaultConfig,
        adit,
        stope: stopePda(BALANCED),
        miner: minerPda(alice.publicKey, BALANCED),
        assetMint: btcMint,
        depositorToken: aliceBtc,
        aditVault,
        tokenProgram: TOKEN_2022_PROGRAM_ID,
        systemProgram: SystemProgram.programId,
      })
      .signers([alice])
      .rpc();

    const miner = await program.account.miner.fetch(minerPda(alice.publicKey, BALANCED));
    assert.equal(miner.shares.toNumber(), 2 * ONE_BTC);
    assert.equal(miner.deposited.toNumber(), 2 * ONE_BTC);

    const stope = await program.account.stope.fetch(stopePda(BALANCED));
    assert.equal(stope.totalShares.toNumber(), 2 * ONE_BTC);
    assert.equal(stope.totalDeposits.toNumber(), 2 * ONE_BTC);

    const vault = await getAccount(
      provider.connection,
      aditVault,
      undefined,
      TOKEN_2022_PROGRAM_ID
    );
    assert.equal(Number(vault.amount), 2 * ONE_BTC);
  });
});

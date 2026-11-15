# OotleEscrow

**Trust-minimized escrow for two-party trades, as a Tari Ootle template.**

Two strangers trading over the internet — crypto OTC in a Telegram or Discord
group, a freelance gig, an in-game item — normally have to trust each other or
pay a human middleman. OotleEscrow replaces the middleman with a small,
auditable Ootle component: the buyer locks the agreed payment in, and from
that moment the money can only ever move to the seller or back to the buyer,
under rules neither side can bend.

*Built for the Tari November Build Contest ("MVP"). Development used an AI
coding agent, as the contest rules expressly allow; the code is MIT-licensed
and every claim below is checkable with the test suites in this repo.*

## Try it first (no wallet, no install)

Open `demo/index.html` in a browser (or serve the `demo/` folder statically).
It is a **walletless simulation** of the template — the default judging path:

- a JavaScript state machine that mirrors the Rust template rule-for-rule
  (same methods, same transitions, same rejection messages),
- guided walkthroughs: happy path, wrong-signer rejection, dispute +
  arbitrator ruling, and deadline timeout refund,
- every click shows the Ootle transaction it maps to and the event emitted.

## How a trade works

One component instance per trade. The terms (parties, payment resource,
exact amount, deadline epoch, arbitrator) are fixed at creation; anyone can
read them back with `info()` before acting.

```
AwaitingFunding ──fund──▶ Funded ──release──▶ Released   (seller paid)
                             ├──refund──────────▶ Refunded   (buyer repaid)
                             ├──claim_timeout_refund (≥ deadline)──▶ Refunded
                             └──dispute──▶ Disputed ──resolve──▶ Resolved
                                                              (arbitrator picks the side)
```

| Method | Who | What it does |
|---|---|---|
| `new(...)` | either party | Creates the escrow with the agreed terms. |
| `fund(bucket)` | buyer | Locks exactly the agreed amount of the agreed resource. Partial/excess/wrong-resource payments are rejected. |
| `release()` | buyer | "I received what I paid for" — pays the seller in full. |
| `refund()` | seller | Seller backs out — repays the buyer in full. |
| `claim_timeout_refund()` | buyer | After the deadline epoch, reclaims the payment if the trade never completed. |
| `dispute()` | buyer or seller | Freezes the funds; only the arbitrator can settle now. |
| `resolve(to_seller)` | arbitrator | Awards the full payment to the seller or back to the buyer. |
| `info()` | anyone | Full terms + current state. |

### Security properties

- **Signer-based authorization.** Every restricted method checks the public
  key that actually signed the transaction
  (`CallerContext::transaction_signer_public_key`) against the key recorded
  at creation. Addresses passed as arguments are never trusted — arguments
  can be spoofed, signatures cannot. The component's engine-level access
  rules are allow-all (all three parties must be able to call in), so these
  signer checks are the authorization layer — and they are exactly what the
  engine tests exercise, including rejection of calls signed by the wrong
  party or by a stranger.
- **Undirectable payouts.** Payouts are withdrawn and deposited *by the
  component itself* into the counterparty account recorded at creation,
  inside the same transaction. A caller cannot divert a payout into their
  own account — the engine tests prove this by having the buyer call
  `release()` and asserting the money lands with the seller.
- **Exact-amount funding.** The escrow accepts one funding of exactly the
  agreed amount in exactly the agreed resource, so terms can't drift after
  the deal is struck.
- **Atomic rejection.** Every invalid transition panics, rolling the whole
  transaction back; a failed call can never leave partial state or move funds.
- **Deadline safety.** If the seller never delivers, the buyer's money is
  recoverable in full at the deadline epoch — no arbitrator needed.
- **Events** (`TradeCreated`, `Funded`, `Released`, `Refunded`, `Disputed`,
  `Resolved`) are emitted for indexers and front-ends.

## Repository layout

```
src/lib.rs          the TradeEscrow template (Rust, tari_template_lib 0.40)
tests/test.rs       engine tests — the template running on the real Ootle
                    engine in-process (tari_template_test_tooling)
demo/escrow.js      the simulation state machine (mirrors the template)
demo/index.html     the walletless demo page (no build step)
demo/parity.test.mjs  simulation tests — the same scenarios as the engine tests
rust-toolchain.toml pinned toolchain (Rust 1.100.0 + wasm32 target)
```

## Build and test

Prerequisites: the pinned Rust toolchain (rustup reads `rust-toolchain.toml`)
with the `wasm32-unknown-unknown` target.

```sh
# Compile the template to WASM (what gets published on-chain)
cargo build --release --target wasm32-unknown-unknown

# Run the engine tests (24 scenarios: every transition + every rejection)
cargo test

# Run the simulation parity tests (27 scenarios, same rules)
node --test demo/parity.test.mjs
```

## Honest limits

- **One trade, one resource, full amount.** No partial releases, milestone
  splits, or multi-asset trades — deliberately: a small rule set is what
  makes this auditable enough to trust with a stranger's money.
- **The arbitrator is a named, trusted human** (or service) chosen by both
  parties at creation. The template enforces *that only they can rule*; it
  cannot make them honest. Pick arbitrators with a reputation to lose.
- **Creation-time setup matters.** The creator supplies both parties' keys
  and payout accounts. Counterparties should verify `info()` before
  funding — if the terms on-chain aren't the terms agreed off-chain, don't
  fund. A mismatched key/account pair can only misdirect that trade's own
  payout, and it is visible in `info()` before any money moves.
- **The demo is a simulation.** It moves no real funds; it exists so judges
  and users can verify the rules without a wallet. The on-chain behaviour
  is covered by the engine tests, not by the demo.
- The compiled template is ~130 KiB of WASM, above the ~96 KiB point where
  mainnet publish fees start growing quadratically. Irrelevant for testnet
  use; a size-optimization pass would be worthwhile before a mainnet launch.

## License

MIT — see `LICENSE`.

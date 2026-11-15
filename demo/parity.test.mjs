// Parity tests for the walletless simulation (demo/escrow.js).
//
// These scenarios mirror, one for one, the Rust engine tests in
// ../tests/test.rs: same flows, same rejection messages. If a rule changes
// in the template, it must change here too (and vice versa).
//
// Run: node --test demo/

import { test } from "node:test";
import assert from "node:assert/strict";
import { TradeEscrow, Status, ERR, EscrowError } from "./escrow.js";

const BUYER = "buyer-key";
const SELLER = "seller-key";
const ARBITRATOR = "arbitrator-key";
const STRANGER = "stranger-key";
const XTM = "XTM";
const AMOUNT = 1_000;
const DEADLINE = 100;

const BUYER_ACCOUNT = "buyer-account";
const SELLER_ACCOUNT = "seller-account";

function newTrade(epoch = 10) {
  return new TradeEscrow({
    buyer: BUYER,
    seller: SELLER,
    arbitrator: ARBITRATOR,
    buyerAccount: BUYER_ACCOUNT,
    sellerAccount: SELLER_ACCOUNT,
    resource: XTM,
    amount: AMOUNT,
    deadlineEpoch: DEADLINE,
    epoch,
  });
}

function rawTrade(overrides) {
  return new TradeEscrow({
    buyer: BUYER,
    seller: SELLER,
    arbitrator: ARBITRATOR,
    buyerAccount: BUYER_ACCOUNT,
    sellerAccount: SELLER_ACCOUNT,
    resource: XTM,
    amount: AMOUNT,
    deadlineEpoch: DEADLINE,
    epoch: 10,
    ...overrides,
  });
}

function fundedTrade() {
  const t = newTrade();
  t.fund({ signer: BUYER, resource: XTM, amount: AMOUNT });
  return t;
}

function rejects(fn, message) {
  assert.throws(fn, (e) => e instanceof EscrowError && e.message === message);
}

// ---------- creation ----------

test("create: starts awaiting funding with an empty vault", () => {
  const info = newTrade().info();
  assert.equal(info.status, Status.AwaitingFunding);
  assert.equal(info.vaultBalance, 0);
  assert.equal(info.amount, AMOUNT);
  assert.equal(info.deadlineEpoch, DEADLINE);
});

test("create: buyer and seller must differ", () => {
  rejects(() => rawTrade({ seller: BUYER }), ERR.buyerMustDiffer);
});

test("create: arbitrator must differ from both parties", () => {
  rejects(() => rawTrade({ arbitrator: BUYER }), ERR.arbitratorMustDiffer);
  rejects(() => rawTrade({ arbitrator: SELLER }), ERR.arbitratorMustDiffer);
});

test("create: amount must be positive", () => {
  rejects(() => rawTrade({ amount: 0 }), ERR.amountPositive);
});

test("create: buyer and seller accounts must differ", () => {
  rejects(() => rawTrade({ sellerAccount: BUYER_ACCOUNT }), ERR.accountsMustDiffer);
});

test("create: deadline must be in the future", () => {
  rejects(() => newTrade(100), ERR.deadlineFuture);
  rejects(() => newTrade(101), ERR.deadlineFuture);
});

// ---------- funding ----------

test("fund: happy path locks the agreed amount", () => {
  const t = fundedTrade();
  assert.equal(t.info().status, Status.Funded);
  assert.equal(t.info().vaultBalance, AMOUNT);
});

test("fund: only the buyer can fund", () => {
  const t = newTrade();
  rejects(() => t.fund({ signer: SELLER, resource: XTM, amount: AMOUNT }), ERR.onlyBuyerFund);
  rejects(() => t.fund({ signer: STRANGER, resource: XTM, amount: AMOUNT }), ERR.onlyBuyerFund);
});

test("fund: wrong resource is rejected", () => {
  const t = newTrade();
  rejects(() => t.fund({ signer: BUYER, resource: "OTHER", amount: AMOUNT }), ERR.wrongResource);
});

test("fund: partial or excess payment is rejected", () => {
  const t = newTrade();
  rejects(() => t.fund({ signer: BUYER, resource: XTM, amount: AMOUNT - 1 }), ERR.wrongAmount);
  rejects(() => t.fund({ signer: BUYER, resource: XTM, amount: AMOUNT + 1 }), ERR.wrongAmount);
});

test("fund: cannot fund twice", () => {
  const t = fundedTrade();
  rejects(() => t.fund({ signer: BUYER, resource: XTM, amount: AMOUNT }), ERR.notAwaitingFunding);
});

// ---------- release ----------

test("release: buyer releases and the seller is paid in full", () => {
  const t = fundedTrade();
  t.release({ signer: BUYER });
  assert.equal(t.info().status, Status.Released);
  assert.equal(t.info().vaultBalance, 0);
  assert.equal(t.paidTo[SELLER], AMOUNT);
  assert.equal(t.paidTo[BUYER], 0);
});

test("release: only the buyer can release", () => {
  const t = fundedTrade();
  rejects(() => t.release({ signer: SELLER }), ERR.onlyBuyerRelease);
  rejects(() => t.release({ signer: ARBITRATOR }), ERR.onlyBuyerRelease);
});

test("release: cannot release before funding", () => {
  const t = newTrade();
  rejects(() => t.release({ signer: BUYER }), ERR.notFunded);
});

// ---------- refunds ----------

test("refund: seller can voluntarily refund while funded", () => {
  const t = fundedTrade();
  t.refund({ signer: SELLER });
  assert.equal(t.info().status, Status.Refunded);
  assert.equal(t.paidTo[BUYER], AMOUNT);
});

test("refund: only the seller can refund voluntarily", () => {
  const t = fundedTrade();
  rejects(() => t.refund({ signer: BUYER }), ERR.onlySellerRefund);
});

test("timeout refund: buyer reclaims at the deadline epoch", () => {
  const t = fundedTrade();
  t.claimTimeoutRefund({ signer: BUYER, epoch: DEADLINE });
  assert.equal(t.info().status, Status.Refunded);
  assert.equal(t.paidTo[BUYER], AMOUNT);
});

test("timeout refund: rejected before the deadline", () => {
  const t = fundedTrade();
  rejects(() => t.claimTimeoutRefund({ signer: BUYER, epoch: DEADLINE - 1 }), ERR.deadlineNotReached);
});

test("timeout refund: only the buyer can claim it", () => {
  const t = fundedTrade();
  rejects(() => t.claimTimeoutRefund({ signer: SELLER, epoch: DEADLINE + 5 }), ERR.onlyBuyerTimeout);
});

// ---------- disputes ----------

test("dispute: either party can open a dispute while funded", () => {
  const byBuyer = fundedTrade();
  byBuyer.dispute({ signer: BUYER });
  assert.equal(byBuyer.info().status, Status.Disputed);
  assert.equal(byBuyer.info().vaultBalance, AMOUNT); // funds stay locked

  const bySeller = fundedTrade();
  bySeller.dispute({ signer: SELLER });
  assert.equal(bySeller.info().status, Status.Disputed);
});

test("dispute: third parties cannot open a dispute", () => {
  const t = fundedTrade();
  rejects(() => t.dispute({ signer: ARBITRATOR }), ERR.onlyPartyDispute);
  rejects(() => t.dispute({ signer: STRANGER }), ERR.onlyPartyDispute);
});

test("dispute: locks out release and refunds until resolved", () => {
  const t = fundedTrade();
  t.dispute({ signer: SELLER });
  rejects(() => t.release({ signer: BUYER }), ERR.notFunded);
  rejects(() => t.refund({ signer: SELLER }), ERR.notFunded);
  rejects(() => t.claimTimeoutRefund({ signer: BUYER, epoch: DEADLINE + 5 }), ERR.notFunded);
});

test("resolve: arbitrator can award the seller", () => {
  const t = fundedTrade();
  t.dispute({ signer: BUYER });
  t.resolve({ signer: ARBITRATOR, toSeller: true });
  assert.equal(t.info().status, Status.Resolved);
  assert.equal(t.paidTo[SELLER], AMOUNT);
});

test("resolve: arbitrator can award the buyer", () => {
  const t = fundedTrade();
  t.dispute({ signer: SELLER });
  t.resolve({ signer: ARBITRATOR, toSeller: false });
  assert.equal(t.info().status, Status.Resolved);
  assert.equal(t.paidTo[BUYER], AMOUNT);
});

test("resolve: only the arbitrator can resolve", () => {
  const t = fundedTrade();
  t.dispute({ signer: BUYER });
  rejects(() => t.resolve({ signer: BUYER, toSeller: true }), ERR.onlyArbitratorResolve);
  rejects(() => t.resolve({ signer: SELLER, toSeller: true }), ERR.onlyArbitratorResolve);
});

test("resolve: cannot resolve an undisputed escrow", () => {
  const t = fundedTrade();
  rejects(() => t.resolve({ signer: ARBITRATOR, toSeller: true }), ERR.notDisputed);
});

// ---------- terminal states ----------

test("terminal: a released escrow rejects every further action", () => {
  const t = fundedTrade();
  t.release({ signer: BUYER });
  rejects(() => t.fund({ signer: BUYER, resource: XTM, amount: AMOUNT }), ERR.notAwaitingFunding);
  rejects(() => t.dispute({ signer: BUYER }), ERR.notFunded);
  rejects(() => t.refund({ signer: SELLER }), ERR.notFunded);
});

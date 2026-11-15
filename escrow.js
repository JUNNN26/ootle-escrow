// OotleEscrow — walletless simulation of the TradeEscrow template.
//
// This module mirrors the on-chain template's state machine rule-for-rule:
// same method names, same state transitions, and the same rejection messages
// as the Rust template in ../src/lib.rs. The demo page (index.html) drives
// this machine; parity.test.mjs pins the behaviour down with tests.
//
// DEMO MODE: no wallet, no funds, no chain. Balances are simulated numbers.
// "Signer" is whichever party the UI says is acting — on-chain, the engine
// verifies the transaction signature against the stored public keys.

export const Status = Object.freeze({
  AwaitingFunding: "AwaitingFunding",
  Funded: "Funded",
  Released: "Released",
  Refunded: "Refunded",
  Disputed: "Disputed",
  Resolved: "Resolved",
});

// Rejection messages — kept identical to the Rust template's assert messages.
export const ERR = Object.freeze({
  buyerMustDiffer: "Buyer and seller must be different parties",
  arbitratorMustDiffer: "Arbitrator must differ from buyer and seller",
  amountPositive: "Escrow amount must be positive",
  accountsMustDiffer: "Buyer and seller accounts must be different",
  deadlineFuture: "Deadline epoch must be in the future",
  onlyBuyerFund: "Only the buyer can fund the escrow",
  notAwaitingFunding: "Escrow is not awaiting funding",
  wrongResource: "Payment must be in the agreed resource",
  wrongAmount: "Payment amount must equal the agreed escrow amount",
  onlyBuyerRelease: "Only the buyer can release the escrow",
  onlySellerRefund: "Only the seller can refund before the deadline",
  notFunded: "Escrow is not funded",
  onlyBuyerTimeout: "Only the buyer can claim a timeout refund",
  deadlineNotReached: "Deadline has not been reached yet",
  onlyPartyDispute: "Only the buyer or seller can open a dispute",
  onlyArbitratorResolve: "Only the arbitrator can resolve a dispute",
  notDisputed: "Escrow is not disputed",
});

export class EscrowError extends Error {}

function reject(message) {
  throw new EscrowError(message);
}

export class TradeEscrow {
  /**
   * Mirrors TradeEscrow::new(buyer, seller, arbitrator, buyer_account,
   * seller_account, resource, amount, deadline_epoch). On-chain, payouts are
   * deposited directly into the stored accounts by the component itself; the
   * sim models that as a credit to the account's owner in `paidTo`.
   * `epoch` is the chain epoch at creation time (the demo's epoch control).
   */
  constructor({ buyer, seller, arbitrator, buyerAccount, sellerAccount, resource, amount, deadlineEpoch, epoch }) {
    if (buyer === seller) reject(ERR.buyerMustDiffer);
    if (arbitrator === buyer || arbitrator === seller) reject(ERR.arbitratorMustDiffer);
    if (!(amount > 0)) reject(ERR.amountPositive);
    if (buyerAccount === sellerAccount) reject(ERR.accountsMustDiffer);
    if (!(deadlineEpoch > epoch)) reject(ERR.deadlineFuture);

    this.buyer = buyer;
    this.seller = seller;
    this.arbitrator = arbitrator;
    this.buyerAccount = buyerAccount;
    this.sellerAccount = sellerAccount;
    this.resource = resource;
    this.amount = amount;
    this.deadlineEpoch = deadlineEpoch;
    this.status = Status.AwaitingFunding;
    this.vaultBalance = 0;
    // Simulated ledger: how much each party has been paid out of the escrow.
    this.paidTo = { [buyer]: 0, [seller]: 0 };
    this.events = [{ name: "TradeCreated", fields: { amount: String(amount), deadline_epoch: String(deadlineEpoch) } }];
  }

  /** Mirrors fund(bucket): buyer-only, exact resource and amount, once. */
  fund({ signer, resource, amount }) {
    if (signer !== this.buyer) reject(ERR.onlyBuyerFund);
    if (this.status !== Status.AwaitingFunding) reject(ERR.notAwaitingFunding);
    if (resource !== this.resource) reject(ERR.wrongResource);
    if (amount !== this.amount) reject(ERR.wrongAmount);
    this.vaultBalance = amount;
    this.status = Status.Funded;
    this.events.push({ name: "Funded", fields: { amount: String(amount) } });
  }

  /** Mirrors release(): buyer-only, pays the seller in full. */
  release({ signer }) {
    if (signer !== this.buyer) reject(ERR.onlyBuyerRelease);
    if (this.status !== Status.Funded) reject(ERR.notFunded);
    this.#payOut(this.seller);
    this.status = Status.Released;
    this.events.push({ name: "Released", fields: { amount: String(this.amount) } });
  }

  /** Mirrors refund(): seller-only voluntary return while funded. */
  refund({ signer }) {
    if (signer !== this.seller) reject(ERR.onlySellerRefund);
    if (this.status !== Status.Funded) reject(ERR.notFunded);
    this.#payOut(this.buyer);
    this.status = Status.Refunded;
    this.events.push({ name: "Refunded", fields: { amount: String(this.amount) } });
  }

  /** Mirrors claim_timeout_refund(): buyer-only, once the deadline epoch is reached. */
  claimTimeoutRefund({ signer, epoch }) {
    if (signer !== this.buyer) reject(ERR.onlyBuyerTimeout);
    if (this.status !== Status.Funded) reject(ERR.notFunded);
    if (epoch < this.deadlineEpoch) reject(ERR.deadlineNotReached);
    this.#payOut(this.buyer);
    this.status = Status.Refunded;
    this.events.push({ name: "Refunded", fields: { amount: String(this.amount) } });
  }

  /** Mirrors dispute(): buyer or seller, while funded. Funds stay locked. */
  dispute({ signer }) {
    if (signer !== this.buyer && signer !== this.seller) reject(ERR.onlyPartyDispute);
    if (this.status !== Status.Funded) reject(ERR.notFunded);
    this.status = Status.Disputed;
    this.events.push({ name: "Disputed", fields: { by: signer } });
  }

  /** Mirrors resolve(to_seller): arbitrator-only, settles a disputed escrow. */
  resolve({ signer, toSeller }) {
    if (signer !== this.arbitrator) reject(ERR.onlyArbitratorResolve);
    if (this.status !== Status.Disputed) reject(ERR.notDisputed);
    this.#payOut(toSeller ? this.seller : this.buyer);
    this.status = Status.Resolved;
    this.events.push({
      name: "Resolved",
      fields: { to_seller: String(toSeller), amount: String(this.amount) },
    });
  }

  /** Mirrors info(): a read-only snapshot of the trade. */
  info() {
    return {
      buyer: this.buyer,
      seller: this.seller,
      arbitrator: this.arbitrator,
      buyerAccount: this.buyerAccount,
      sellerAccount: this.sellerAccount,
      resource: this.resource,
      amount: this.amount,
      deadlineEpoch: this.deadlineEpoch,
      status: this.status,
      vaultBalance: this.vaultBalance,
    };
  }

  #payOut(party) {
    this.paidTo[party] += this.vaultBalance;
    this.vaultBalance = 0;
  }
}

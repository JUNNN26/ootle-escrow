// OotleEscrow — a trust-minimized escrow template for two-party trades on Tari Ootle.
//
// Two strangers trading over the internet (crypto OTC in a chat group, a freelance
// gig, an in-game item) normally have to trust each other or a human middleman.
// This template replaces the middleman: the buyer locks the agreed payment into
// the component, and the funds can only ever move to the seller (on the buyer's
// release, or an arbitrator's ruling) or back to the buyer (seller refund, timeout
// refund, or an arbitrator's ruling). Nobody else can touch them, and no party
// can redirect a payout — payouts are deposited straight into the counterparty
// account recorded at creation time, inside the same transaction.
//
// Lifecycle:  AwaitingFunding --fund--> Funded --release--> Released
//                                    |--refund / timeout--> Refunded
//                                    |--dispute--> Disputed --resolve--> Resolved
//
// Parties are identified by the public key that signs their transactions
// (checked against `CallerContext::transaction_signer_public_key`), never by
// an address passed in as an argument — arguments can be spoofed, signatures
// cannot.

use tari_template_lib::prelude::*;

/// Lifecycle states of a trade. `Released`, `Refunded` and `Resolved` are
/// terminal: once reached, every mutating method rejects.
#[derive(Debug, Clone, Copy, PartialEq, Eq, minicbor::Encode, minicbor::Decode, minicbor::CborLen)]
pub enum EscrowStatus {
    #[n(0)]
    AwaitingFunding,
    #[n(1)]
    Funded,
    #[n(2)]
    Released,
    #[n(3)]
    Refunded,
    #[n(4)]
    Disputed,
    #[n(5)]
    Resolved,
}

/// Read-only snapshot of a trade, returned by `TradeEscrow::info()`. Lets
/// either party verify the exact terms before funding or acting.
#[derive(Debug, Clone, minicbor::Encode, minicbor::Decode, minicbor::CborLen)]
pub struct TradeInfo {
    #[n(0)]
    pub buyer: RistrettoPublicKeyBytes,
    #[n(1)]
    pub seller: RistrettoPublicKeyBytes,
    #[n(2)]
    pub arbitrator: RistrettoPublicKeyBytes,
    #[n(3)]
    pub buyer_account: ComponentAddress,
    #[n(4)]
    pub seller_account: ComponentAddress,
    #[n(5)]
    pub payment_resource: ResourceAddress,
    #[n(6)]
    pub amount: Amount,
    #[n(7)]
    pub deadline_epoch: u64,
    #[n(8)]
    pub status: EscrowStatus,
    #[n(9)]
    pub vault_balance: Amount,
}

#[template]
pub mod ootle_escrow {
    use super::*;

    pub struct TradeEscrow {
        buyer: RistrettoPublicKeyBytes,
        seller: RistrettoPublicKeyBytes,
        arbitrator: RistrettoPublicKeyBytes,
        // Payouts are deposited into these accounts by the component itself,
        // so a caller can never divert funds into their own account.
        buyer_account: ComponentAddress,
        seller_account: ComponentAddress,
        payment_resource: ResourceAddress,
        amount: Amount,
        deadline_epoch: u64,
        status: EscrowStatus,
        vault: Vault,
    }

    impl TradeEscrow {
        /// Creates a new, unfunded escrow for one trade. Either party may
        /// create it; the terms are fixed here and the counterparty verifies
        /// them via `info()` before the buyer funds.
        ///
        /// * `buyer` / `seller` / `arbitrator` — signing public keys of the parties.
        /// * `buyer_account` / `seller_account` — accounts payouts are deposited into.
        /// * `payment_resource` — the fungible resource being traded (e.g. XTM).
        /// * `amount` — the exact agreed payment; funding must match it exactly.
        /// * `deadline_epoch` — if the trade is still funded at this epoch, the
        ///   buyer may reclaim the payment in full.
        pub fn new(
            buyer: RistrettoPublicKeyBytes,
            seller: RistrettoPublicKeyBytes,
            arbitrator: RistrettoPublicKeyBytes,
            buyer_account: ComponentAddress,
            seller_account: ComponentAddress,
            payment_resource: ResourceAddress,
            amount: Amount,
            deadline_epoch: u64,
        ) -> Component<Self> {
            assert_ne!(buyer, seller, "Buyer and seller must be different parties");
            assert!(
                arbitrator != buyer && arbitrator != seller,
                "Arbitrator must differ from buyer and seller"
            );
            assert!(amount.is_positive(), "Escrow amount must be positive");
            assert_ne!(
                buyer_account, seller_account,
                "Buyer and seller accounts must be different"
            );
            assert!(
                deadline_epoch > Consensus::current_epoch(),
                "Deadline epoch must be in the future"
            );

            emit_event("TradeCreated", [
                ("amount", amount.to_string()),
                ("deadline_epoch", deadline_epoch.to_string()),
            ]);

            Component::new(Self {
                buyer,
                seller,
                arbitrator,
                buyer_account,
                seller_account,
                payment_resource,
                amount,
                deadline_epoch,
                status: EscrowStatus::AwaitingFunding,
                vault: Vault::new_empty(payment_resource),
            })
            // Methods must be callable by all three parties (and `info` by
            // anyone), so the component-level rule is allow-all. This is NOT
            // the authorization layer: every restricted method re-checks the
            // transaction signer's public key against the party recorded
            // above, and rejects anyone else. (The engine default is
            // deny-all, which would lock out the seller and arbitrator.)
            .with_access_rules(AccessRules::allow_all())
            .create()
        }

        /// Locks the agreed payment into the escrow. Buyer only, exactly once,
        /// and the bucket must be exactly the agreed resource and amount —
        /// partial or excess funding is rejected so the terms can never drift.
        pub fn fund(&mut self, payment: Bucket) {
            assert!(
                Self::signer() == self.buyer,
                "Only the buyer can fund the escrow"
            );
            assert!(
                self.status == EscrowStatus::AwaitingFunding,
                "Escrow is not awaiting funding"
            );
            assert_eq!(
                payment.resource_address(),
                self.payment_resource,
                "Payment must be in the agreed resource"
            );
            assert_eq!(
                payment.amount(),
                self.amount,
                "Payment amount must equal the agreed escrow amount"
            );

            self.vault.deposit(payment);
            self.status = EscrowStatus::Funded;
            emit_event("Funded", [("amount", self.amount.to_string())]);
        }

        /// Releases the full payment to the seller. Buyer only: this is the
        /// buyer confirming "I received what I paid for".
        pub fn release(&mut self) {
            assert!(
                Self::signer() == self.buyer,
                "Only the buyer can release the escrow"
            );
            assert!(self.status == EscrowStatus::Funded, "Escrow is not funded");

            self.payout(self.seller_account);
            self.status = EscrowStatus::Released;
            emit_event("Released", [("amount", self.amount.to_string())]);
        }

        /// Voluntarily returns the full payment to the buyer. Seller only:
        /// the seller backing out of the trade before delivery.
        pub fn refund(&mut self) {
            assert!(
                Self::signer() == self.seller,
                "Only the seller can refund before the deadline"
            );
            assert!(self.status == EscrowStatus::Funded, "Escrow is not funded");

            self.payout(self.buyer_account);
            self.status = EscrowStatus::Refunded;
            emit_event("Refunded", [("amount", self.amount.to_string())]);
        }

        /// Reclaims the full payment once the deadline epoch has been reached
        /// and the trade was never completed. Buyer only — this protects the
        /// buyer against a seller who takes the deal and never delivers.
        pub fn claim_timeout_refund(&mut self) {
            assert!(
                Self::signer() == self.buyer,
                "Only the buyer can claim a timeout refund"
            );
            assert!(self.status == EscrowStatus::Funded, "Escrow is not funded");
            assert!(
                Consensus::current_epoch() >= self.deadline_epoch,
                "Deadline has not been reached yet"
            );

            self.payout(self.buyer_account);
            self.status = EscrowStatus::Refunded;
            emit_event("Refunded", [("amount", self.amount.to_string())]);
        }

        /// Opens a dispute, freezing the funds in place until the arbitrator
        /// rules. Buyer or seller only. While disputed, release and refunds
        /// are locked out — neither party can unilaterally settle any more.
        pub fn dispute(&mut self) {
            let signer = Self::signer();
            assert!(
                signer == self.buyer || signer == self.seller,
                "Only the buyer or seller can open a dispute"
            );
            assert!(self.status == EscrowStatus::Funded, "Escrow is not funded");

            self.status = EscrowStatus::Disputed;
            emit_event("Disputed", [
                ("by", if signer == self.buyer { "buyer" } else { "seller" }.to_string()),
            ]);
        }

        /// Settles a dispute: the full payment goes to the seller
        /// (`to_seller = true`) or back to the buyer (`false`). Arbitrator only.
        pub fn resolve(&mut self, to_seller: bool) {
            assert!(
                Self::signer() == self.arbitrator,
                "Only the arbitrator can resolve a dispute"
            );
            assert!(
                self.status == EscrowStatus::Disputed,
                "Escrow is not disputed"
            );

            self.payout(if to_seller {
                self.seller_account
            } else {
                self.buyer_account
            });
            self.status = EscrowStatus::Resolved;
            emit_event("Resolved", [
                ("to_seller", to_seller.to_string()),
                ("amount", self.amount.to_string()),
            ]);
        }

        /// Returns the full terms and current state of the trade.
        pub fn info(&self) -> TradeInfo {
            TradeInfo {
                buyer: self.buyer,
                seller: self.seller,
                arbitrator: self.arbitrator,
                buyer_account: self.buyer_account,
                seller_account: self.seller_account,
                payment_resource: self.payment_resource,
                amount: self.amount,
                deadline_epoch: self.deadline_epoch,
                status: self.status,
                vault_balance: self.vault.balance(),
            }
        }

        /// The public key that signed the transaction currently executing.
        fn signer() -> RistrettoPublicKeyBytes {
            CallerContext::transaction_signer_public_key()
        }

        /// Withdraws the entire vault and deposits it directly into `account`.
        /// Depositing inside the component (rather than returning a bucket to
        /// the caller) is what makes payouts undivertable.
        fn payout(&mut self, account: ComponentAddress) {
            let bucket = self.vault.withdraw_all();
            ComponentManager::get(account).invoke("deposit", args![bucket]);
        }
    }
}

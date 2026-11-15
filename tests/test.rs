// Engine tests for the TradeEscrow template, run against the real Ootle
// engine in-process via tari_template_test_tooling.
//
// These scenarios mirror, one for one, the simulation parity tests in
// demo/parity.test.mjs: same flows, same rejection messages. If a rule
// changes in the template, it must change in both suites.
//
// Fees are disabled by default in the test harness, so account balances can
// be asserted exactly. All amounts are in the native token's smallest unit.

use tari_template_lib::types::constants::TARI_TOKEN;
use tari_template_lib::types::crypto::RistrettoPublicKeyBytes;
use tari_template_lib::types::{Amount, ComponentAddress, NonFungibleAddress, ResourceAddress};
use tari_template_test_tooling::byte_type::ToByteType;
use tari_template_test_tooling::crypto::{RistrettoPublicKey, RistrettoSecretKey};
use tari_template_test_tooling::engine_types::virtual_substate::{VirtualSubstate, VirtualSubstateId};
use tari_template_test_tooling::support::assert_error::assert_reject_reason;
use tari_template_test_tooling::transaction::args;
use tari_template_test_tooling::TemplateTest;

const TEMPLATE_NAME: &str = "TradeEscrow";
const AMOUNT: u64 = 1_000;
const DEADLINE: u64 = 100;
const INITIAL: u64 = TemplateTest::FUNDED_ACCOUNT_INITIAL_BALANCE;

// Mirror of the template's public types (integration tests cannot link the
// cdylib crate itself, so the CBOR layout is duplicated here — field order
// and variant indices must match src/lib.rs).
#[derive(Debug, Clone, Copy, PartialEq, Eq, minicbor::Encode, minicbor::Decode, minicbor::CborLen)]
enum EscrowStatus {
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

#[derive(Debug, Clone, minicbor::Encode, minicbor::Decode, minicbor::CborLen)]
struct TradeInfo {
    #[n(0)]
    buyer: RistrettoPublicKeyBytes,
    #[n(1)]
    seller: RistrettoPublicKeyBytes,
    #[n(2)]
    arbitrator: RistrettoPublicKeyBytes,
    #[n(3)]
    buyer_account: ComponentAddress,
    #[n(4)]
    seller_account: ComponentAddress,
    #[n(5)]
    payment_resource: ResourceAddress,
    #[n(6)]
    amount: Amount,
    #[n(7)]
    deadline_epoch: u64,
    #[n(8)]
    status: EscrowStatus,
    #[n(9)]
    vault_balance: Amount,
}

struct Party {
    account: ComponentAddress,
    proof: NonFungibleAddress,
    secret_key: RistrettoSecretKey,
    public_key: RistrettoPublicKey,
}

impl Party {
    fn pk_bytes(&self) -> RistrettoPublicKeyBytes {
        self.public_key.to_byte_type()
    }
}

struct Setup {
    test: TemplateTest,
    buyer: Party,
    seller: Party,
    arbitrator: Party,
    stranger: Party,
}

/// Selects which party signs a call. Passed by value: helpers receive
/// `&mut Setup`, so callers cannot also pass a `&Party` borrowed from it.
#[derive(Clone, Copy)]
enum Who {
    Buyer,
    Seller,
    Arbitrator,
    Stranger,
}

impl Setup {
    fn party(&self, who: Who) -> &Party {
        match who {
            Who::Buyer => &self.buyer,
            Who::Seller => &self.seller,
            Who::Arbitrator => &self.arbitrator,
            Who::Stranger => &self.stranger,
        }
    }
}

fn make_party(test: &mut TemplateTest) -> Party {
    let (account, proof, secret_key, public_key) = test.create_funded_account_with_keypair();
    Party {
        account,
        proof,
        secret_key,
        public_key,
    }
}

fn setup() -> Setup {
    let mut test = TemplateTest::my_crate();
    let buyer = make_party(&mut test);
    let seller = make_party(&mut test);
    let arbitrator = make_party(&mut test);
    let stranger = make_party(&mut test);
    Setup {
        test,
        buyer,
        seller,
        arbitrator,
        stranger,
    }
}

/// Calls TradeEscrow::new with the standard terms, sealed by the buyer.
fn create_escrow(s: &mut Setup, amount: u64, deadline: u64) -> ComponentAddress {
    let template = s.test.get_template_address(TEMPLATE_NAME);
    // Bind every argument before opening the transaction builder: the
    // builder holds a mutable borrow of the harness (`s.test`), which
    // conflicts with any other borrow of `s`.
    let (buyer_pk, seller_pk, arbitrator_pk) = (
        s.buyer.pk_bytes(),
        s.seller.pk_bytes(),
        s.arbitrator.pk_bytes(),
    );
    let (buyer_account, seller_account) = (s.buyer.account, s.seller.account);
    let (secret_key, proof) = (s.buyer.secret_key.clone(), s.buyer.proof.clone());
    let result = s.test.execute_expect_success(
        s.test
            .transaction()
            .call_function(
                template,
                "new",
                args![
                    buyer_pk,
                    seller_pk,
                    arbitrator_pk,
                    buyer_account,
                    seller_account,
                    TARI_TOKEN,
                    amount,
                    deadline
                ],
            )
            .build_and_seal(&secret_key),
        vec![proof],
    );
    result
        .finalize
        .result
        .accept()
        .unwrap()
        .up_iter()
        .find(|(id, sub)| {
            id.is_component()
                && *sub.substate_value().component().unwrap().template_address() == template
        })
        .map(|(id, _)| id.as_component_address().unwrap())
        .unwrap()
}

/// Attempts TradeEscrow::new with custom terms and asserts the rejection message.
fn create_escrow_expect_failure(
    s: &mut Setup,
    buyer_pk: RistrettoPublicKeyBytes,
    seller_pk: RistrettoPublicKeyBytes,
    arbitrator_pk: RistrettoPublicKeyBytes,
    buyer_account: ComponentAddress,
    seller_account: ComponentAddress,
    amount: u64,
    deadline: u64,
    expected_reason: &str,
) {
    let template = s.test.get_template_address(TEMPLATE_NAME);
    let (secret_key, proof) = (s.buyer.secret_key.clone(), s.buyer.proof.clone());
    let reason = s.test.execute_expect_failure(
        s.test
            .transaction()
            .call_function(
                template,
                "new",
                args![
                    buyer_pk,
                    seller_pk,
                    arbitrator_pk,
                    buyer_account,
                    seller_account,
                    TARI_TOKEN,
                    amount,
                    deadline
                ],
            )
            .build_and_seal(&secret_key),
        vec![proof],
    );
    assert_reject_reason(reason, expected_reason);
}

fn fund(s: &mut Setup, escrow: ComponentAddress, amount: u64) {
    let (account, secret_key, proof) = (
        s.buyer.account,
        s.buyer.secret_key.clone(),
        s.buyer.proof.clone(),
    );
    s.test.execute_expect_success(
        s.test
            .transaction()
            .call_method(account, "withdraw", args![TARI_TOKEN, amount])
            .put_last_instruction_output_on_workspace("payment")
            .call_method(escrow, "fund", args![Workspace("payment")])
            .build_and_seal(&secret_key),
        vec![proof],
    );
}

fn funded_escrow(s: &mut Setup) -> ComponentAddress {
    let escrow = create_escrow(s, AMOUNT, DEADLINE);
    fund(s, escrow, AMOUNT);
    escrow
}

/// Calls a no-argument escrow method, sealed by the given party.
fn call_no_args(s: &mut Setup, escrow: ComponentAddress, method: &str, caller: Who) {
    let party = s.party(caller);
    let (secret_key, proof) = (party.secret_key.clone(), party.proof.clone());
    s.test.execute_expect_success(
        s.test
            .transaction()
            .call_method(escrow, method, args![])
            .build_and_seal(&secret_key),
        vec![proof],
    );
}

/// Calls a no-argument escrow method expecting rejection with the given message.
fn call_no_args_expect_failure(
    s: &mut Setup,
    escrow: ComponentAddress,
    method: &str,
    caller: Who,
    expected_reason: &str,
) {
    let party = s.party(caller);
    let (secret_key, proof) = (party.secret_key.clone(), party.proof.clone());
    let reason = s.test.execute_expect_failure(
        s.test
            .transaction()
            .call_method(escrow, method, args![])
            .build_and_seal(&secret_key),
        vec![proof],
    );
    assert_reject_reason(reason, expected_reason);
}

fn balance_of(s: &Setup, account: ComponentAddress) -> Amount {
    let vaults = s
        .test
        .read_only_state_store()
        .get_vaults_for_account(account)
        .unwrap();
    vaults
        .get(&TARI_TOKEN)
        .map(|vault| vault.balance())
        .unwrap_or_default()
}

fn get_info(s: &mut Setup, escrow: ComponentAddress) -> TradeInfo {
    let (secret_key, proof) = (s.buyer.secret_key.clone(), s.buyer.proof.clone());
    s.test.execute_expect_success(
        s.test
            .transaction()
            .call_method(escrow, "info", args![])
            .build_and_seal(&secret_key),
        vec![proof],
    );
    s.test.expect_return::<TradeInfo>(0)
}

// ---------------------------------------------------------------------------
// Creation
// ---------------------------------------------------------------------------

#[test]
fn it_creates_with_the_agreed_terms() {
    let mut s = setup();
    let escrow = create_escrow(&mut s, AMOUNT, DEADLINE);

    let info = get_info(&mut s, escrow);
    assert_eq!(info.buyer, s.buyer.pk_bytes());
    assert_eq!(info.seller, s.seller.pk_bytes());
    assert_eq!(info.arbitrator, s.arbitrator.pk_bytes());
    assert_eq!(info.buyer_account, s.buyer.account);
    assert_eq!(info.seller_account, s.seller.account);
    assert_eq!(info.payment_resource, TARI_TOKEN);
    assert_eq!(info.amount, Amount::from(AMOUNT));
    assert_eq!(info.deadline_epoch, DEADLINE);
    assert_eq!(info.status, EscrowStatus::AwaitingFunding);
    assert_eq!(info.vault_balance, Amount::ZERO);
    // Creating the escrow moves no funds.
    assert_eq!(balance_of(&s, s.buyer.account), Amount::from(INITIAL));
}

#[test]
fn it_rejects_buyer_equal_seller() {
    let mut s = setup();
    let (buyer_pk, arb_pk, buyer_acc, seller_acc) = (
        s.buyer.pk_bytes(),
        s.arbitrator.pk_bytes(),
        s.buyer.account,
        s.seller.account,
    );
    create_escrow_expect_failure(
        &mut s,
        buyer_pk,
        buyer_pk, // seller == buyer
        arb_pk,
        buyer_acc,
        seller_acc,
        AMOUNT,
        DEADLINE,
        "Buyer and seller must be different parties",
    );
}

#[test]
fn it_rejects_arbitrator_equal_to_a_party() {
    let mut s = setup();
    let (buyer_pk, seller_pk, buyer_acc, seller_acc) = (
        s.buyer.pk_bytes(),
        s.seller.pk_bytes(),
        s.buyer.account,
        s.seller.account,
    );
    create_escrow_expect_failure(
        &mut s,
        buyer_pk,
        seller_pk,
        buyer_pk, // arbitrator == buyer
        buyer_acc,
        seller_acc,
        AMOUNT,
        DEADLINE,
        "Arbitrator must differ from buyer and seller",
    );
}

#[test]
fn it_rejects_zero_amount() {
    let mut s = setup();
    let (buyer_pk, seller_pk, arb_pk, buyer_acc, seller_acc) = (
        s.buyer.pk_bytes(),
        s.seller.pk_bytes(),
        s.arbitrator.pk_bytes(),
        s.buyer.account,
        s.seller.account,
    );
    create_escrow_expect_failure(
        &mut s,
        buyer_pk,
        seller_pk,
        arb_pk,
        buyer_acc,
        seller_acc,
        0,
        DEADLINE,
        "Escrow amount must be positive",
    );
}

#[test]
fn it_rejects_the_same_account_for_both_parties() {
    let mut s = setup();
    let (buyer_pk, seller_pk, arb_pk, buyer_acc) = (
        s.buyer.pk_bytes(),
        s.seller.pk_bytes(),
        s.arbitrator.pk_bytes(),
        s.buyer.account,
    );
    create_escrow_expect_failure(
        &mut s,
        buyer_pk,
        seller_pk,
        arb_pk,
        buyer_acc,
        buyer_acc, // seller account == buyer account
        AMOUNT,
        DEADLINE,
        "Buyer and seller accounts must be different",
    );
}

#[test]
fn it_rejects_a_deadline_not_in_the_future() {
    let mut s = setup();
    // The harness starts at epoch 0, so deadline 0 is not in the future.
    let (buyer_pk, seller_pk, arb_pk, buyer_acc, seller_acc) = (
        s.buyer.pk_bytes(),
        s.seller.pk_bytes(),
        s.arbitrator.pk_bytes(),
        s.buyer.account,
        s.seller.account,
    );
    create_escrow_expect_failure(
        &mut s,
        buyer_pk,
        seller_pk,
        arb_pk,
        buyer_acc,
        seller_acc,
        AMOUNT,
        0,
        "Deadline epoch must be in the future",
    );
}

// ---------------------------------------------------------------------------
// Funding
// ---------------------------------------------------------------------------

#[test]
fn it_funds_and_locks_the_payment() {
    let mut s = setup();
    let escrow = funded_escrow(&mut s);

    let info = get_info(&mut s, escrow);
    assert_eq!(info.status, EscrowStatus::Funded);
    assert_eq!(info.vault_balance, Amount::from(AMOUNT));
    assert_eq!(
        balance_of(&s, s.buyer.account),
        Amount::from(INITIAL - AMOUNT)
    );
    assert_eq!(balance_of(&s, s.seller.account), Amount::from(INITIAL));
}

#[test]
fn it_rejects_funding_from_a_non_buyer() {
    let mut s = setup();
    let escrow = create_escrow(&mut s, AMOUNT, DEADLINE);

    // The seller tries to fund with their own money — rejected by the signer check.
    let (account, secret_key, proof) = (
        s.seller.account,
        s.seller.secret_key.clone(),
        s.seller.proof.clone(),
    );
    let reason = s.test.execute_expect_failure(
        s.test
            .transaction()
            .call_method(account, "withdraw", args![TARI_TOKEN, AMOUNT])
            .put_last_instruction_output_on_workspace("payment")
            .call_method(escrow, "fund", args![Workspace("payment")])
            .build_and_seal(&secret_key),
        vec![proof],
    );
    assert_reject_reason(reason, "Only the buyer can fund the escrow");
    // The failed transaction rolled back: the seller still has everything.
    assert_eq!(balance_of(&s, s.seller.account), Amount::from(INITIAL));
}

#[test]
fn it_rejects_partial_or_excess_funding() {
    let mut s = setup();
    let escrow = create_escrow(&mut s, AMOUNT, DEADLINE);

    for bad_amount in [AMOUNT - 1, AMOUNT + 1] {
        let (account, secret_key, proof) = (
            s.buyer.account,
            s.buyer.secret_key.clone(),
            s.buyer.proof.clone(),
        );
        let reason = s.test.execute_expect_failure(
            s.test
                .transaction()
                .call_method(account, "withdraw", args![TARI_TOKEN, bad_amount])
                .put_last_instruction_output_on_workspace("payment")
                .call_method(escrow, "fund", args![Workspace("payment")])
                .build_and_seal(&secret_key),
            vec![proof],
        );
        assert_reject_reason(reason, "Payment amount must equal the agreed escrow amount");
    }
    assert_eq!(balance_of(&s, s.buyer.account), Amount::from(INITIAL));
}

#[test]
fn it_rejects_a_second_funding() {
    let mut s = setup();
    let escrow = funded_escrow(&mut s);

    let (account, secret_key, proof) = (
        s.buyer.account,
        s.buyer.secret_key.clone(),
        s.buyer.proof.clone(),
    );
    let reason = s.test.execute_expect_failure(
        s.test
            .transaction()
            .call_method(account, "withdraw", args![TARI_TOKEN, AMOUNT])
            .put_last_instruction_output_on_workspace("payment")
            .call_method(escrow, "fund", args![Workspace("payment")])
            .build_and_seal(&secret_key),
        vec![proof],
    );
    assert_reject_reason(reason, "Escrow is not awaiting funding");
}

// ---------------------------------------------------------------------------
// Release
// ---------------------------------------------------------------------------

#[test]
fn it_releases_the_payment_to_the_seller() {
    let mut s = setup();
    let escrow = funded_escrow(&mut s);

    call_no_args(&mut s, escrow, "release", Who::Buyer);

    // The buyer called release, yet the money landed with the seller:
    // payouts are deposited by the component and cannot be diverted.
    assert_eq!(
        balance_of(&s, s.seller.account),
        Amount::from(INITIAL + AMOUNT)
    );
    assert_eq!(
        balance_of(&s, s.buyer.account),
        Amount::from(INITIAL - AMOUNT)
    );
    let info = get_info(&mut s, escrow);
    assert_eq!(info.status, EscrowStatus::Released);
    assert_eq!(info.vault_balance, Amount::ZERO);
}

#[test]
fn it_rejects_release_from_a_non_buyer() {
    let mut s = setup();
    let escrow = funded_escrow(&mut s);
    call_no_args_expect_failure(
        &mut s,
        escrow,
        "release",
        Who::Seller,
        "Only the buyer can release the escrow",
    );
    call_no_args_expect_failure(
        &mut s,
        escrow,
        "release",
        Who::Arbitrator,
        "Only the buyer can release the escrow",
    );
}

#[test]
fn it_rejects_release_before_funding() {
    let mut s = setup();
    let escrow = create_escrow(&mut s, AMOUNT, DEADLINE);
    call_no_args_expect_failure(&mut s, escrow, "release", Who::Buyer, "Escrow is not funded");
}

// ---------------------------------------------------------------------------
// Refunds
// ---------------------------------------------------------------------------

#[test]
fn it_refunds_the_buyer_when_the_seller_backs_out() {
    let mut s = setup();
    let escrow = funded_escrow(&mut s);

    call_no_args(&mut s, escrow, "refund", Who::Seller);

    assert_eq!(balance_of(&s, s.buyer.account), Amount::from(INITIAL));
    assert_eq!(balance_of(&s, s.seller.account), Amount::from(INITIAL));
    assert_eq!(get_info(&mut s, escrow).status, EscrowStatus::Refunded);
}

#[test]
fn it_rejects_refund_from_the_buyer() {
    let mut s = setup();
    let escrow = funded_escrow(&mut s);
    call_no_args_expect_failure(
        &mut s,
        escrow,
        "refund",
        Who::Buyer,
        "Only the seller can refund before the deadline",
    );
}

#[test]
fn it_rejects_a_timeout_refund_before_the_deadline() {
    let mut s = setup();
    let escrow = funded_escrow(&mut s);
    call_no_args_expect_failure(
        &mut s,
        escrow,
        "claim_timeout_refund",
        Who::Buyer,
        "Deadline has not been reached yet",
    );
}

#[test]
fn it_claims_a_timeout_refund_at_the_deadline() {
    let mut s = setup();
    let escrow = funded_escrow(&mut s);

    // Advance the chain to the deadline epoch.
    s.test.set_virtual_substate(
        VirtualSubstateId::CurrentEpoch,
        VirtualSubstate::CurrentEpoch(DEADLINE),
    );
    call_no_args(&mut s, escrow, "claim_timeout_refund", Who::Buyer);

    assert_eq!(balance_of(&s, s.buyer.account), Amount::from(INITIAL));
    assert_eq!(get_info(&mut s, escrow).status, EscrowStatus::Refunded);
}

#[test]
fn it_rejects_a_timeout_claim_from_the_seller() {
    let mut s = setup();
    let escrow = funded_escrow(&mut s);
    s.test.set_virtual_substate(
        VirtualSubstateId::CurrentEpoch,
        VirtualSubstate::CurrentEpoch(DEADLINE),
    );
    call_no_args_expect_failure(
        &mut s,
        escrow,
        "claim_timeout_refund",
        Who::Seller,
        "Only the buyer can claim a timeout refund",
    );
}

// ---------------------------------------------------------------------------
// Disputes
// ---------------------------------------------------------------------------

#[test]
fn it_disputes_and_the_arbitrator_awards_the_seller() {
    let mut s = setup();
    let escrow = funded_escrow(&mut s);

    call_no_args(&mut s, escrow, "dispute", Who::Buyer);
    assert_eq!(get_info(&mut s, escrow).status, EscrowStatus::Disputed);
    // Funds stay locked while disputed.
    assert_eq!(
        get_info(&mut s, escrow).vault_balance,
        Amount::from(AMOUNT)
    );

    // Neither party can settle unilaterally any more.
    call_no_args_expect_failure(&mut s, escrow, "release", Who::Buyer, "Escrow is not funded");
    call_no_args_expect_failure(&mut s, escrow, "refund", Who::Seller, "Escrow is not funded");

    let (secret_key, proof) = (s.arbitrator.secret_key.clone(), s.arbitrator.proof.clone());
    s.test.execute_expect_success(
        s.test
            .transaction()
            .call_method(escrow, "resolve", args![true])
            .build_and_seal(&secret_key),
        vec![proof],
    );

    assert_eq!(
        balance_of(&s, s.seller.account),
        Amount::from(INITIAL + AMOUNT)
    );
    assert_eq!(get_info(&mut s, escrow).status, EscrowStatus::Resolved);
}

#[test]
fn it_disputes_and_the_arbitrator_awards_the_buyer() {
    let mut s = setup();
    let escrow = funded_escrow(&mut s);

    call_no_args(&mut s, escrow, "dispute", Who::Seller);
    let (secret_key, proof) = (s.arbitrator.secret_key.clone(), s.arbitrator.proof.clone());
    s.test.execute_expect_success(
        s.test
            .transaction()
            .call_method(escrow, "resolve", args![false])
            .build_and_seal(&secret_key),
        vec![proof],
    );

    assert_eq!(balance_of(&s, s.buyer.account), Amount::from(INITIAL));
    assert_eq!(balance_of(&s, s.seller.account), Amount::from(INITIAL));
    assert_eq!(get_info(&mut s, escrow).status, EscrowStatus::Resolved);
}

#[test]
fn it_rejects_a_dispute_from_a_third_party() {
    let mut s = setup();
    let escrow = funded_escrow(&mut s);
    call_no_args_expect_failure(
        &mut s,
        escrow,
        "dispute",
        Who::Arbitrator,
        "Only the buyer or seller can open a dispute",
    );
    call_no_args_expect_failure(
        &mut s,
        escrow,
        "dispute",
        Who::Stranger,
        "Only the buyer or seller can open a dispute",
    );
}

#[test]
fn it_rejects_resolution_from_a_non_arbitrator() {
    let mut s = setup();
    let escrow = funded_escrow(&mut s);
    call_no_args(&mut s, escrow, "dispute", Who::Buyer);

    let (secret_key, proof) = (s.buyer.secret_key.clone(), s.buyer.proof.clone());
    let reason = s.test.execute_expect_failure(
        s.test
            .transaction()
            .call_method(escrow, "resolve", args![true])
            .build_and_seal(&secret_key),
        vec![proof],
    );
    assert_reject_reason(reason, "Only the arbitrator can resolve a dispute");
}

#[test]
fn it_rejects_resolution_when_not_disputed() {
    let mut s = setup();
    let escrow = funded_escrow(&mut s);

    let (secret_key, proof) = (s.arbitrator.secret_key.clone(), s.arbitrator.proof.clone());
    let reason = s.test.execute_expect_failure(
        s.test
            .transaction()
            .call_method(escrow, "resolve", args![true])
            .build_and_seal(&secret_key),
        vec![proof],
    );
    assert_reject_reason(reason, "Escrow is not disputed");
}

// ---------------------------------------------------------------------------
// Terminal states
// ---------------------------------------------------------------------------

#[test]
fn it_rejects_everything_once_released() {
    let mut s = setup();
    let escrow = funded_escrow(&mut s);
    call_no_args(&mut s, escrow, "release", Who::Buyer);

    call_no_args_expect_failure(&mut s, escrow, "release", Who::Buyer, "Escrow is not funded");
    call_no_args_expect_failure(&mut s, escrow, "refund", Who::Seller, "Escrow is not funded");
    call_no_args_expect_failure(&mut s, escrow, "dispute", Who::Buyer, "Escrow is not funded");
    // Funding again is also impossible.
    let (account, secret_key, proof) = (
        s.buyer.account,
        s.buyer.secret_key.clone(),
        s.buyer.proof.clone(),
    );
    let reason = s.test.execute_expect_failure(
        s.test
            .transaction()
            .call_method(account, "withdraw", args![TARI_TOKEN, AMOUNT])
            .put_last_instruction_output_on_workspace("payment")
            .call_method(escrow, "fund", args![Workspace("payment")])
            .build_and_seal(&secret_key),
        vec![proof],
    );
    assert_reject_reason(reason, "Escrow is not awaiting funding");
}

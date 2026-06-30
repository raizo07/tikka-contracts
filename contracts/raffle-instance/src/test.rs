#![cfg(test)]

use super::*;
use ed25519_dalek::{Signer, SigningKey};
use soroban_sdk::{
    testutils::{Address as _, Ledger},
    token::StellarAssetClient,
    xdr::ToXdr,
    Address, Bytes, BytesN, Env, String,
};
use raffle_shared::{DEFAULT_CLAIM_LOCKUP_SECONDS, DEFAULT_SWAP_DEADLINE_SECONDS};

fn assert_drawing_lock_cleared(env: &Env, contract_id: &Address) {
    let is_set: bool = env.as_contract(contract_id, || {
        env.storage()
            .instance()
            .get(&crate::DataKey::DrawingLock)
            .unwrap_or(false)
    });
    assert!(!is_set, "DrawingLock must be cleared");
}

#[test]
fn test_oracle_fallback_with_ledger_delays() {
    let env = Env::default();
    env.mock_all_auths();

    // 1. Setup factory, admin, creator
    let factory = Address::generate(&env);
    let admin = Address::generate(&env);
    let creator = Address::generate(&env);
    let oracle = Address::generate(&env);
    let token_admin = Address::generate(&env);
    let payment_token = env
        .register_stellar_asset_contract_v2(token_admin.clone())
        .address();
    let token_client = StellarAssetClient::new(&env, &payment_token);
    token_client.mint(&creator, &100_000_000);

    let contract_id = env.register(RaffleInstance, ());
    let client = RaffleInstanceClient::new(&env, &contract_id);

    // 2. Initialize Raffle with External Randomness
    let config = RaffleConfig {
        description: String::from_str(&env, "Test Raffle"),
        end_time: 0,
        no_deadline: true,
        max_tickets: 10,
        max_tickets_per_tx: 10,
        min_tickets: 1,
        allow_multiple: true,
        ticket_price: 10_000,
        payment_token: payment_token.clone(),
        prize_amount: 10_000,
        prizes: soroban_sdk::vec![&env, 10000],
        randomness_source: RandomnessSource::External,
        oracle_address: Some(oracle.clone()),
        protocol_fee_bp: 0,
        treasury_address: None,
        swap_router: None,
        tikka_token: None,
        metadata_hash: BytesN::from_array(&env, &[1; 32]),
        claim_lockup_seconds: 0,
        swap_deadline_seconds: 0,
        early_bird_ticket_percentage: 0,
        early_bird_discount_bp: 0,
    };

    client.init(&factory, &admin, &creator, &config);

    // Verify that defaults were resolved (0 values replaced with defaults)
    let raffle = client.get_raffle();
    assert_eq!(raffle.claim_lockup_seconds, DEFAULT_CLAIM_LOCKUP_SECONDS);
    assert_eq!(raffle.swap_deadline_seconds, DEFAULT_SWAP_DEADLINE_SECONDS);

    // Remove factory from storage so buy_tickets skips the factory code path
    env.as_contract(&contract_id, || {
        env.storage().instance().remove(&DataKey::Factory);
    });

    // 3. Deposit prize and buy ticket
    client.deposit_prize();
    client.buy_tickets(&creator, &10);

    // 4. Finalize raffle (requests randomness)
    client.finalize_raffle();

    // 5. Ensure it's in Drawing state and requested randomness
    let raffle = client.get_raffle();
    assert_eq!(raffle.status, RaffleStatus::Drawing);

    // 6. Attempt fallback too early
    let result = client.try_trigger_randomness_fallback(&creator, &false);
    assert_eq!(result.err(), Some(Ok(Error::FallbackTooEarly)));

    // 7. Simulate ledger delays
    env.ledger().with_mut(|l| {
        l.sequence_number += ORACLE_TIMEOUT_LEDGERS + 1;
        l.timestamp += 86400; // 1 day
    });

    // 8. Trigger fallback successfully (no refund — finalize)
    client.trigger_randomness_fallback(&creator, &false);

    // 9. Verify finalized state
    let raffle_after = client.get_raffle();
    assert_eq!(raffle_after.status, RaffleStatus::Finalized);

    // We can also verify the fairness data
    let fairness = client.get_fairness_data();
    assert_eq!(fairness.randomness_source, RandomnessSource::External);
}

#[test]
fn test_admin_updates_oracle_address() {
    let env = Env::default();
    env.mock_all_auths();

    let factory = Address::generate(&env);
    let admin = Address::generate(&env);
    let creator = Address::generate(&env);
    let oracle = Address::generate(&env);
    let new_oracle = Address::generate(&env);

    let contract_id = env.register(RaffleInstance, ());
    let client = RaffleInstanceClient::new(&env, &contract_id);

    let config = RaffleConfig {
        description: String::from_str(&env, "Oracle migration"),
        end_time: 0,
        no_deadline: true,
        max_tickets: 5,
        max_tickets_per_tx: 5,
        min_tickets: 1,
        allow_multiple: true,
        ticket_price: MIN_TICKET_PRICE,
        payment_token: env
            .register_stellar_asset_contract_v2(Address::generate(&env))
            .address(),
        prize_amount: MIN_TICKET_PRICE * 5,
        prizes: soroban_sdk::vec![&env, 10000],
        randomness_source: RandomnessSource::External,
        oracle_address: Some(oracle.clone()),
        protocol_fee_bp: 100,
        treasury_address: None,
        swap_router: None,
        tikka_token: None,
        metadata_hash: BytesN::from_array(&env, &[2; 32]),
        claim_lockup_seconds: 0,
        swap_deadline_seconds: 0,
        early_bird_ticket_percentage: 0,
        early_bird_discount_bp: 0,
    };

    client.init(&factory, &admin, &creator, &config);

    // Verify that defaults were resolved
    let raffle = client.get_raffle();
    assert_eq!(raffle.claim_lockup_seconds, DEFAULT_CLAIM_LOCKUP_SECONDS);
    assert_eq!(raffle.swap_deadline_seconds, DEFAULT_SWAP_DEADLINE_SECONDS);

#[contractimpl]
impl MockFactory {
    pub fn record_volume(_env: Env, _token: Address, _amount: i128) {}
    pub fn track_participant(_env: Env, _participant: Address) {}
}

#[test]
fn non_winner_cannot_claim() {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().set_timestamp(1_000);

    let contract_id = env.register(RaffleInstance, ());
    let client = RaffleInstanceClient::new(&env, &contract_id);

    let config = RaffleConfig {
        description: String::from_str(&env, "Fee update"),
        end_time: 0,
        no_deadline: true,
        max_tickets: 5,
        max_tickets_per_tx: 5,
        min_tickets: 1,
        allow_multiple: true,
        ticket_price: MIN_TICKET_PRICE,
        payment_token: env
            .register_stellar_asset_contract_v2(Address::generate(&env))
            .address(),
        prize_amount: MIN_TICKET_PRICE * 5,
        prizes: soroban_sdk::vec![&env, 10000],
        randomness_source: RandomnessSource::Internal,
        oracle_address: None,
        protocol_fee_bp: 100,
        treasury_address: None,
        swap_router: None,
        tikka_token: None,
        metadata_hash: BytesN::from_array(&env, &[3; 32]),
        claim_lockup_seconds: 0,
        swap_deadline_seconds: 0,
        early_bird_ticket_percentage: 0,
        early_bird_discount_bp: 0,
    };

    client.init(&factory, &admin, &creator, &config);

    // Verify that defaults were resolved
    let raffle = client.get_raffle();
    assert_eq!(raffle.claim_lockup_seconds, DEFAULT_CLAIM_LOCKUP_SECONDS);
    assert_eq!(raffle.swap_deadline_seconds, DEFAULT_SWAP_DEADLINE_SECONDS);

    client.set_protocol_fee_bp(&500);

    let raffle = client.get_raffle();
    assert_eq!(raffle.protocol_fee_bp, 500);
}

#[test]
fn test_admin_withdraws_accumulated_fees() {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().set_timestamp(1_000);

    let factory = Address::generate(&env);
    let admin = Address::generate(&env);
    let creator = Address::generate(&env);
    let buyer = Address::generate(&env);
    let attacker = Address::generate(&env);

    let token_admin = Address::generate(&env);
    let (token_addr, token_mint) = create_token(&env, &token_admin);
    token_mint.mint(&creator, &1_000_000);
    token_mint.mint(&buyer, &1_000_000);

    let config = RaffleConfig {
        description: String::from_str(&env, "test raffle"),
        end_time: 2_000,
        no_deadline: false,
        max_tickets: 2,
        max_tickets_per_tx: 2,
        min_tickets: 1,
        allow_multiple: true,
        ticket_price: MIN_TICKET_PRICE,
        payment_token: token_addr.clone(),
        prize_amount: MIN_TICKET_PRICE * 10,
        prizes: vec![&env, 10000u32],
        randomness_source: RandomnessSource::Internal,
        oracle_address: None,
        protocol_fee_bp: 0,
        treasury_address: None,
        swap_router: None,
        tikka_token: None,
        metadata_hash: BytesN::from_array(&env, &[1u8; 32]),
        claim_lockup_seconds: 0,
        swap_deadline_seconds: 0,
        early_bird_ticket_percentage: 0,
        early_bird_discount_bp: 0,
    };

    client.init(&factory, &admin, &creator, &config);
    client.deposit_prize();
    client.buy_tickets(&buyer, &1);
    env.ledger().set_timestamp(2_000);
    env.ledger().set_timestamp(2_000);
    client.finalize_raffle();

    let raffle = client.get_raffle();
    assert_eq!(raffle.winners.len(), 1);
    assert!(raffle.winners.get(0).unwrap() != attacker);

    env.ledger().set_timestamp(2_000 + DEFAULT_CLAIM_LOCKUP_SECONDS + 1);

    let result = client.try_claim_prize(&attacker, &0u32);
    assert_eq!(result, Err(Ok(Error::NotWinner)));
}

#[test]
fn buy_tickets_rejects_quantity_above_per_tx_cap() {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().set_timestamp(1_000);

    let contract_id = env.register(Contract, ());
    let client = ContractClient::new(&env, &contract_id);

    let factory = env.register(MockFactory, ());
    let admin = Address::generate(&env);
    let creator = Address::generate(&env);
    let buyer = Address::generate(&env);

    let token_admin = Address::generate(&env);
    let (token_addr, token_mint) = create_token(&env, &token_admin);
    token_mint.mint(&creator, &1_000_000);
    token_mint.mint(&buyer, &1_000_000);

    let config = RaffleConfig {
        description: String::from_str(&env, "Per-tx cap"),
        end_time: 0,
        no_deadline: true,
        max_tickets: 100,
        max_tickets_per_tx: 5,
        min_tickets: 1,
        allow_multiple: true,
        ticket_price: MIN_TICKET_PRICE,
        payment_token: token_addr.clone(),
        prize_amount: MIN_TICKET_PRICE * 100,
        prizes: vec![&env, 10000u32],
        randomness_source: RandomnessSource::Internal,
        oracle_address: None,
        protocol_fee_bp: 0,
        treasury_address: None,
        swap_router: None,
        tikka_token: None,
        metadata_hash: BytesN::from_array(&env, &[5u8; 32]),
        claim_lockup_seconds: 0,
        swap_deadline_seconds: 0,
        early_bird_ticket_percentage: 0,
        early_bird_discount_bp: 0,
    };

    client.init(&factory, &admin, &creator, &config);
    client.deposit_prize();

    assert_eq!(
        client.try_buy_tickets(&buyer, &6),
        Err(Ok(Error::ExceedsMaxTicketsPerTx))
    );
    assert_eq!(client.buy_tickets(&buyer, &5), 5);
}

fn setup_active_raffle(
    env: &Env,
) -> (
    ContractClient<'_>,
    Address,
    Address,
    Address,
    Address,
    token::StellarAssetClient<'_>,
) {
    let contract_id = env.register(Contract, ());
    let client = ContractClient::new(env, &contract_id);

    let factory = env.register(MockFactory, ());
    let admin = Address::generate(env);
    let creator = Address::generate(env);
    let buyer = Address::generate(env);

    let token_admin = Address::generate(env);
    let (token_addr, token_mint) = create_token(env, &token_admin);
    token_mint.mint(&creator, &1_000_000);
    token_mint.mint(&buyer, &1_000_000);

    let config = RaffleConfig {
        description: String::from_str(env, "ticket sales pause"),
        end_time: 0,
        no_deadline: true,
        max_tickets: 1,
        max_tickets_per_tx: 1,
        min_tickets: 1,
        allow_multiple: true,
        ticket_price: MIN_TICKET_PRICE,
        payment_token: token_addr,
        prize_amount: MIN_TICKET_PRICE * 100,
        prizes: vec![env, 10000u32],
        randomness_source: RandomnessSource::Internal,
        oracle_address: None,
        protocol_fee_bp: 0,
        treasury_address: None,
        swap_router: None,
        tikka_token: None,
        metadata_hash: BytesN::from_array(env, &[7u8; 32]),
        claim_lockup_seconds: 0,
        swap_deadline_seconds: 0,
        early_bird_ticket_percentage: 0,
        early_bird_discount_bp: 0,
    };

    client.init(&factory, &admin, &creator, &config);
    client.deposit_prize();

    (client, admin, creator, buyer, factory, token_mint)
}

#[test]
fn pause_resume_ticket_sales_controls_buy_tickets() {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().set_timestamp(1_000);

    let (client, _admin, creator, buyer, _factory, _token_mint) = setup_active_raffle(&env);

    assert_eq!(client.get_raffle().status, RaffleStatus::Active);
    assert!(!client.is_ticket_sales_paused());

    let config = RaffleConfig {
        description: String::from_str(&env, "Rollback test"),
        end_time: 0,
        no_deadline: true,
        max_tickets: 1,
        max_tickets_per_tx: 1,
        min_tickets: 1,
        allow_multiple: true,
        ticket_price: MIN_TICKET_PRICE,
        payment_token: payment_token.clone(),
        prize_amount: MIN_TICKET_PRICE,
        prizes: soroban_sdk::vec![&env, 10000],
        randomness_source: RandomnessSource::External,
        oracle_address: Some(Address::generate(&env)),
        protocol_fee_bp: 0,
        treasury_address: None,
        swap_router: None,
        tikka_token: None,
        metadata_hash: BytesN::from_array(&env, &[8; 32]),
        claim_lockup_seconds: 0,
        swap_deadline_seconds: 0,
        early_bird_ticket_percentage: 0,
        early_bird_discount_bp: 0,
    };

    client.init(&factory, &admin, &creator, &config);

    client.resume_ticket_sales(&creator);
    assert!(!client.is_ticket_sales_paused());
    assert_eq!(client.get_raffle().status, RaffleStatus::Active);
    assert_eq!(client.buy_tickets(&buyer, &1), 1);
}

#[test]
fn admin_can_pause_and_resume_ticket_sales() {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().set_timestamp(1_000);

    let (client, admin, _creator, buyer, _factory, _token_mint) = setup_active_raffle(&env);

    client.pause_ticket_sales(&admin);
    assert!(client.is_ticket_sales_paused());
    assert_eq!(
        client.try_buy_tickets(&buyer, &1),
        Err(Ok(Error::ContractPaused))
    );

    client.resume_ticket_sales(&admin);
    assert!(!client.is_ticket_sales_paused());
    assert_eq!(client.buy_tickets(&buyer, &1), 1);
}

#[test]
fn test_wipe_storage_removes_all_keys() {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().set_timestamp(1_000);

    let contract_id = env.register(Contract, ());
    let client = ContractClient::new(&env, &contract_id);

    let factory = env.register(MockFactory, ());
    let admin = Address::generate(&env);
    let creator = Address::generate(&env);
    let buyer_a = Address::generate(&env);
    let buyer_b = Address::generate(&env);

    let token_admin = Address::generate(&env);
    let (token_addr, token_mint) = create_token(&env, &token_admin);
    token_mint.mint(&creator, &1_000_000);
    token_mint.mint(&buyer_a, &1_000_000);
    token_mint.mint(&buyer_b, &1_000_000);

    let config = RaffleConfig {
        description: String::from_str(&env, "wipe test"),
        end_time: 0,
        no_deadline: true,
        max_tickets: 10,
        max_tickets_per_tx: 10,
        min_tickets: 1,
        allow_multiple: true,
        ticket_price: MIN_TICKET_PRICE,
        payment_token: token_addr,
        prize_amount: MIN_TICKET_PRICE * 10,
        prizes: vec![&env, 10000u32],
        randomness_source: RandomnessSource::Internal,
        oracle_address: None,
        protocol_fee_bp: 0,
        treasury_address: None,
        swap_router: None,
        tikka_token: None,
        metadata_hash: BytesN::from_array(&env, &[9u8; 32]),
        claim_lockup_seconds: 0,
        swap_deadline_seconds: 0,
        early_bird_ticket_percentage: 0,
        early_bird_discount_bp: 0,
    };

    client.init(&factory, &admin, &creator, &config);
    client.deposit_prize();
    client.buy_tickets(&buyer_a, &3);
    client.buy_tickets(&buyer_b, &2);

    client.cancel_raffle(&CancelReason::AdminCancelled);

    assert_eq!(client.get_raffle().status, RaffleStatus::Cancelled);

    client.wipe_storage();

    env.as_contract(&contract_id, || {
        for i in 1..=5 {
            assert!(!env.storage().persistent().has(&DataKey::Ticket(i)));
            assert!(!env.storage().persistent().has(&DataKey::TicketRefunded(i)));
            assert!(!env.storage().persistent().has(&DataKey::CommitEntry(i)));
        }
        assert!(!env.storage().persistent().has(&DataKey::TicketCount(buyer_a.clone())));
        assert!(!env.storage().persistent().has(&DataKey::TicketCount(buyer_b.clone())));
        assert!(!env.storage().persistent().has(&DataKey::TicketBuyers));
        assert!(!env.storage().instance().has(&DataKey::Raffle));
        assert!(!env.storage().instance().has(&DataKey::Factory));
        assert!(!env.storage().instance().has(&DataKey::Admin));
        assert!(!env.storage().instance().has(&DataKey::Paused));
        assert!(!env.storage().instance().has(&DataKey::ReentrancyGuard));
        assert!(!env.storage().instance().has(&DataKey::AccumulatedFees));
        assert!(!env.storage().instance().has(&DataKey::RandomnessRequested));
        assert!(!env.storage().instance().has(&DataKey::RandomnessRequestLedger));
        assert!(!env.storage().instance().has(&DataKey::RandomnessRequestId));
        assert!(!env.storage().instance().has(&DataKey::DrawingLock));
        assert!(!env.storage().instance().has(&DataKey::FinishTime));
        assert!(!env.storage().persistent().has(&DataKey::RandomnessSeed));
        assert!(!env.storage().persistent().has(&DataKey::Admin));
    });
}

#[test]
fn emergency_withdraw_fails_before_delay_in_finalized_state() {
fn test_refund_ticket_after_cancel() {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().set_timestamp(1_000);

    let contract_id = env.register(Contract, ());
    let client = ContractClient::new(&env, &contract_id);

    let factory = env.register(MockFactory, ());
    let admin = Address::generate(&env);
    let creator = Address::generate(&env);
    let token_admin = Address::generate(&env);
    let payment_token = env
        .register_stellar_asset_contract_v2(token_admin.clone())
        .address();
    let token_client = StellarAssetClient::new(&env, &payment_token);
    token_client.mint(&creator, &1_000_000);

    let contract_id = env.register(RaffleInstance, ());
    let client = RaffleInstanceClient::new(&env, &contract_id);

    let config = RaffleConfig {
        description: String::from_str(&env, "Test"),
        end_time: 2_000,
        no_deadline: false,
        max_tickets: 1,
        max_tickets_per_tx: 1,
        min_tickets: 1,
        allow_multiple: true,
        ticket_price: MIN_TICKET_PRICE,
        payment_token: payment_token.clone(),
        prize_amount: MIN_TICKET_PRICE * 10,
        prizes: soroban_sdk::vec![&env, 10000],
        randomness_source: RandomnessSource::Internal,
        oracle_address: None,
        protocol_fee_bp: 0,
        treasury_address: None,
        swap_router: None,
        tikka_token: None,
        metadata_hash: BytesN::from_array(&env, &[9; 32]),
        claim_lockup_seconds: 0,
    };

    client.init(&factory, &admin, &creator, &config);
    client.deposit_prize();
    client.buy_tickets(&creator, &1);
    client.finalize_raffle();

    let result = client.try_emergency_withdraw(&creator);
    assert_eq!(result.err(), Some(Ok(Error::EmergencyTooEarly)));
}

#[test]
fn emergency_withdraw_succeeds_after_delay_in_finalized_state() {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().set_timestamp(1_000);

    let factory = Address::generate(&env);
    let admin = Address::generate(&env);
    let creator = Address::generate(&env);
    let buyer = Address::generate(&env);

    let token_admin = Address::generate(&env);
    let (token_addr, token_mint) = create_token(&env, &token_admin);
    token_mint.mint(&creator, &10_000_000);

    let config = RaffleConfig {
        description: String::from_str(&env, "Test"),
        end_time: 2_000,
        no_deadline: false,
        max_tickets: 1,
        max_tickets_per_tx: 1,
        min_tickets: 1,
        allow_multiple: true,
        ticket_price: MIN_TICKET_PRICE,
        payment_token: payment_token.clone(),
        prize_amount: MIN_TICKET_PRICE * 10,
        prizes: soroban_sdk::vec![&env, 10000],
        randomness_source: RandomnessSource::Internal,
        oracle_address: None,
        protocol_fee_bp: 0,
        treasury_address: None,
        swap_router: None,
        tikka_token: None,
        metadata_hash: BytesN::from_array(&env, &[10; 32]),
        claim_lockup_seconds: 0,
    };

    client.init(&factory, &admin, &creator, &config);
    client.deposit_prize();
    client.buy_tickets(&creator, &1);
    client.finalize_raffle();

    env.ledger().set_timestamp(1_000 + EMERGENCY_WITHDRAW_DELAY_SECONDS + 1);

    client.emergency_withdraw(&creator);
    let raffle = client.get_raffle();
    assert_eq!(raffle.status, RaffleStatus::Cancelled);
    assert!(!raffle.prize_deposited);
}

#[test]
fn emergency_withdraw_fails_for_no_deadline_raffle_before_timeout() {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().set_timestamp(1_000);

    let contract_id = env.register(Contract, ());
    let client = ContractClient::new(&env, &contract_id);

    let factory = env.register(MockFactory, ());
    let admin = Address::generate(&env);
    let creator = Address::generate(&env);
    let token_admin = Address::generate(&env);
    let payment_token = env
        .register_stellar_asset_contract_v2(token_admin.clone())
        .address();
    let token_client = StellarAssetClient::new(&env, &payment_token);
    token_client.mint(&creator, &1_000_000);

    let contract_id = env.register(RaffleInstance, ());
    let client = RaffleInstanceClient::new(&env, &contract_id);

    let end_time = 5_000u64;
    let config = RaffleConfig {
        description: String::from_str(&env, "Test"),
        end_time: 0,
        no_deadline: true,
        max_tickets: 1,
        max_tickets_per_tx: 1,
        description: String::from_str(&env, "Refund test"),
        end_time: 0,
        no_deadline: true,
        max_tickets: 5,
        max_tickets_per_tx: 5,
        min_tickets: 1,
        allow_multiple: true,
        ticket_price: MIN_TICKET_PRICE,
        payment_token: payment_token.clone(),
        prize_amount: MIN_TICKET_PRICE * 10,
        prize_amount: MIN_TICKET_PRICE * 5,
        prizes: vec![&env, 10000u32],
        randomness_source: RandomnessSource::External,
        oracle_address: Some(oracle.clone()),
        protocol_fee_bp: 0,
        treasury_address: None,
        swap_router: None,
        tikka_token: None,
        metadata_hash: BytesN::from_array(&env, &[11; 32]),
        claim_lockup_seconds: 0,
    };

    client.init(&factory, &admin, &creator, &config);
    client.deposit_prize();
    client.buy_tickets(&creator, &1);
    client.finalize_raffle();

    let result = client.try_emergency_withdraw(&creator);
    assert_eq!(result.err(), Some(Ok(Error::EmergencyTooEarly)));
}

#[test]
fn emergency_withdraw_succeeds_for_no_deadline_drawing_raffle_after_timeout() {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().set_timestamp(1_000);

    let factory = Address::generate(&env);
    let admin = Address::generate(&env);
    let creator = Address::generate(&env);
    let oracle = Address::generate(&env);
    let token_admin = Address::generate(&env);
    let payment_token = env
        .register_stellar_asset_contract_v2(token_admin.clone())
        .address();
    let token_client = StellarAssetClient::new(&env, &payment_token);
    token_client.mint(&creator, &1_000_000);

    let contract_id = env.register(Contract, ());
    let client = ContractClient::new(&env, &contract_id);

    let config = RaffleConfig {
        description: String::from_str(&env, "Test"),
        end_time: 2_000,
        no_deadline: false,
        max_tickets: 1,
        max_tickets_per_tx: 1,
        min_tickets: 1,
        allow_multiple: true,
        ticket_price: MIN_TICKET_PRICE,
        payment_token: payment_token.clone(),
        prize_amount: MIN_TICKET_PRICE * 10,
        prizes: soroban_sdk::vec![&env, 10000],
        randomness_source: RandomnessSource::External,
        oracle_address: Some(oracle),
        protocol_fee_bp: 0,
        treasury_address: None,
        swap_router: None,
        tikka_token: None,
        metadata_hash: BytesN::from_array(&env, &[12; 32]),
        claim_lockup_seconds: 0,
    };

    client.init(&factory, &admin, &creator, &config);
    client.deposit_prize();
    client.buy_tickets(&creator, &1);
    client.finalize_raffle();

    env.ledger().set_timestamp(2_000 + EMERGENCY_WITHDRAW_DELAY_SECONDS + 1);

    client.emergency_withdraw(&creator);
    let raffle = client.get_raffle();
    assert_eq!(raffle.status, RaffleStatus::Cancelled);
    assert!(!raffle.prize_deposited);
}

#[test]
fn emergency_withdraw_fails_in_active_state() {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().set_timestamp(1_000);

    let factory = Address::generate(&env);
    let admin = Address::generate(&env);
    let creator = Address::generate(&env);
    let token_admin = Address::generate(&env);
    let payment_token = env
        .register_stellar_asset_contract_v2(token_admin.clone())
        .address();
    let token_client = StellarAssetClient::new(&env, &payment_token);
    token_client.mint(&creator, &1_000_000);

    let contract_id = env.register(Contract, ());
    let client = ContractClient::new(&env, &contract_id);

    let config = RaffleConfig {
        description: String::from_str(&env, "Test"),
        end_time: 10_000,
        no_deadline: false,
        max_tickets: 1,
        max_tickets_per_tx: 1,
        min_tickets: 1,
        allow_multiple: true,
        ticket_price: MIN_TICKET_PRICE,
        payment_token: payment_token.clone(),
        prize_amount: MIN_TICKET_PRICE * 10,
        prizes: soroban_sdk::vec![&env, 10000],
        randomness_source: RandomnessSource::Internal,
        oracle_address: None,
        protocol_fee_bp: 0,
        treasury_address: None,
        swap_router: None,
        tikka_token: None,
        metadata_hash: BytesN::from_array(&env, &[13; 32]),
        claim_lockup_seconds: 0,
    };

    client.init(&factory, &admin, &creator, &config);
    client.deposit_prize();

    let result = client.try_emergency_withdraw(&creator);
    assert_eq!(result.err(), Some(Ok(Error::InvalidStatus)));
}

#[test]
fn emergency_withdraw_fails_in_cancelled_state() {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().set_timestamp(1_000);

    let factory = Address::generate(&env);
    let admin = Address::generate(&env);
    let creator = Address::generate(&env);
    let token_admin = Address::generate(&env);
    let payment_token = env
        .register_stellar_asset_contract_v2(token_admin.clone())
        .address();
    let token_client = StellarAssetClient::new(&env, &payment_token);
    token_client.mint(&creator, &1_000_000);

fn setup_external_drawing_raffle(
    env: &Env,
) -> (Address, ContractClient<'_>, Address, Address, Address, u64) {
    let contract_id = env.register(Contract, ());
    let client = ContractClient::new(env, &contract_id);

    let factory = env.register(MockFactory, ());
    let admin = Address::generate(env);
    let creator = Address::generate(env);
    let oracle = Address::generate(env);

    let token_admin = Address::generate(env);
    let (token_addr, token_mint) = create_token(env, &token_admin);
    token_mint.mint(&creator, &10_000_000);

    let config = RaffleConfig {
        description: String::from_str(&env, "Test"),
        end_time: 10_000,
        no_deadline: false,
        max_tickets: 1,
        max_tickets_per_tx: 1,
        min_tickets: 1,
        allow_multiple: true,
        ticket_price: MIN_TICKET_PRICE,
        payment_token: payment_token.clone(),
        prize_amount: MIN_TICKET_PRICE * 10,
        prizes: soroban_sdk::vec![&env, 10000],
        randomness_source: RandomnessSource::Internal,
        oracle_address: None,
        protocol_fee_bp: 0,
        treasury_address: None,
        swap_router: None,
        tikka_token: None,
        metadata_hash: BytesN::from_array(&env, &[14; 32]),
        claim_lockup_seconds: 0,
    };

    client.init(&factory, &admin, &creator, &config);
    client.deposit_prize();
    client.cancel(&creator, &CancelReason::Other);

    let result = client.try_emergency_withdraw(&creator);
    assert_eq!(result.err(), Some(Ok(Error::InvalidStatus)));
}

#[test]
fn emergency_withdraw_fails_if_prize_not_deposited() {
        metadata_hash: BytesN::from_array(&env, &[5; 32]),
        claim_lockup_seconds: 0,
        swap_deadline_seconds: 0,
        prize_token: None,
        nft_contract: None,
    };

    client.init(&factory, &admin, &creator, &config);
    env.as_contract(&contract_id, || {
        env.storage().instance().remove(&DataKey::Factory);
    });

    client.deposit_prize();
    client.buy_tickets(&buyer, &1);

    let balance_before = soroban_sdk::token::Client::new(&env, &payment_token).balance(&buyer);
    client.cancel_raffle(&CancelReason::CreatorCancelled);

    let refunded = client.refund_ticket(&1);
    assert_eq!(refunded, MIN_TICKET_PRICE);

    let balance_after = soroban_sdk::token::Client::new(&env, &payment_token).balance(&buyer);
    assert_eq!(balance_after, balance_before + MIN_TICKET_PRICE);

    let second_refund = client.try_refund_ticket(&1);
    assert_eq!(second_refund.err(), Some(Ok(Error::PrizeAlreadyClaimed)));
}

#[test]
fn test_refund_guard_released_after_success() {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().set_timestamp(1_000);

    let factory = Address::generate(&env);
    let admin = Address::generate(&env);
    let creator = Address::generate(&env);
    let token_admin = Address::generate(&env);
    let payment_token = env
        .register_stellar_asset_contract_v2(token_admin.clone())
        .address();

    let contract_id = env.register(Contract, ());
    let client = ContractClient::new(&env, &contract_id);

    let config = RaffleConfig {
        description: String::from_str(&env, "Test"),
        end_time: 10_000,
        no_deadline: false,
        max_tickets: 1,
        max_tickets_per_tx: 1,
        min_tickets: 1,
        allow_multiple: true,
        ticket_price: MIN_TICKET_PRICE,
        payment_token: payment_token.clone(),
        prize_amount: MIN_TICKET_PRICE * 10,
        prizes: soroban_sdk::vec![&env, 10000],
        randomness_source: RandomnessSource::Internal,
        oracle_address: None,
        protocol_fee_bp: 0,
        treasury_address: None,
        swap_router: None,
        tikka_token: None,
        metadata_hash: BytesN::from_array(&env, &[15; 32]),
        claim_lockup_seconds: 0,
    };

    client.init(&factory, &admin, &creator, &config);

    let result = client.try_emergency_withdraw(&creator);
    assert_eq!(result.err(), Some(Ok(Error::PrizeNotDeposited)));
}

#[test]
fn emergency_withdraw_only_callable_by_creator_or_admin() {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().set_timestamp(1_000);

    let factory = Address::generate(&env);
    let admin = Address::generate(&env);
    let creator = Address::generate(&env);
    let stranger = Address::generate(&env);
    let buyer = Address::generate(&env);

    let token_admin = Address::generate(&env);
    let payment_token = env
        .register_stellar_asset_contract_v2(token_admin.clone())
        .address();
    let token_client = StellarAssetClient::new(&env, &payment_token);
    token_client.mint(&creator, &1_000_000);
    token_client.mint(&buyer, &1_000_000);

    let contract_id = env.register(Contract, ());
    let client = ContractClient::new(&env, &contract_id);

    let config = RaffleConfig {
        description: String::from_str(&env, "Test"),
        end_time: 2_000,
        no_deadline: false,
        max_tickets: 1,
        max_tickets_per_tx: 1,
        description: String::from_str(&env, "Guard release"),
        end_time: 0,
        no_deadline: true,
        max_tickets: 5,
        max_tickets_per_tx: 5,
        min_tickets: 1,
        allow_multiple: true,
        ticket_price: MIN_TICKET_PRICE,
        payment_token: payment_token.clone(),
        prize_amount: MIN_TICKET_PRICE * 10,
        prize_amount: MIN_TICKET_PRICE * 5,
        prizes: soroban_sdk::vec![&env, 10000],
        randomness_source: RandomnessSource::Internal,
        oracle_address: None,
        protocol_fee_bp: 0,
        treasury_address: None,
        swap_router: None,
        tikka_token: None,
        metadata_hash: BytesN::from_array(&env, &[16; 32]),
        claim_lockup_seconds: 0,
    };

    client.init(&factory, &admin, &creator, &config);
    client.deposit_prize();
    client.buy_tickets(&creator, &1);
    client.finalize_raffle();

    env.ledger().set_timestamp(1_000 + EMERGENCY_WITHDRAW_DELAY_SECONDS + 1);

    let stranger_result = client.try_emergency_withdraw(&stranger);
    assert_eq!(stranger_result.err(), Some(Ok(Error::NotAuthorized)));

    client.emergency_withdraw(&admin);
}

#[test]
fn emergency_withdraw_sets_status_to_cancelled_and_clears_prize_deposited() {
        metadata_hash: BytesN::from_array(&env, &[6; 32]),
        claim_lockup_seconds: 0,
        swap_deadline_seconds: 0,
    };

    client.init(&factory, &admin, &creator, &config);
    env.as_contract(&contract_id, || {
        env.storage().instance().remove(&DataKey::Factory);
    });

    client.deposit_prize();
    client.buy_tickets(&buyer, &2);
    client.cancel_raffle(&CancelReason::CreatorCancelled);

    client.refund_ticket(&1);
    let second = client.refund_ticket(&2);
    assert_eq!(second, MIN_TICKET_PRICE);

    let guard_set: bool = env.as_contract(&contract_id, || {
        env.storage()
            .instance()
            .get(&DataKey::ReentrancyGuard)
            .unwrap_or(false)
    });
    assert!(!guard_set);
}

#[test]
fn test_claim_prize_pays_full_gross_with_protocol_fee() {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().set_timestamp(1_000);

    let factory = Address::generate(&env);
    let admin = Address::generate(&env);
    let creator = Address::generate(&env);
    let buyer = Address::generate(&env);

    let token_admin = Address::generate(&env);
    let payment_token = env
        .register_stellar_asset_contract_v2(token_admin.clone())
        .address();
    let token_client = StellarAssetClient::new(&env, &payment_token);
    token_client.mint(&creator, &1_000_000);
    token_client.mint(&buyer, &1_000_000);

    let contract_id = env.register(Contract, ());
    let client = ContractClient::new(&env, &contract_id);

    let config = RaffleConfig {
        description: String::from_str(&env, "Test"),
        end_time: 2_000,
        no_deadline: false,
        description: String::from_str(&env, "Claim gross"),
        end_time: 0,
        no_deadline: true,
        max_tickets: 1,
        max_tickets_per_tx: 1,
        min_tickets: 1,
        allow_multiple: true,
        ticket_price: MIN_TICKET_PRICE,
        payment_token: payment_token.clone(),
        prize_amount: MIN_TICKET_PRICE * 10,
        prizes: soroban_sdk::vec![&env, 10000],
        randomness_source: RandomnessSource::Internal,
        oracle_address: None,
        protocol_fee_bp: 0,
        treasury_address: None,
        swap_router: None,
        tikka_token: None,
        metadata_hash: BytesN::from_array(&env, &[17; 32]),
        claim_lockup_seconds: 0,
        protocol_fee_bp: 1_000,
        treasury_address: None,
        swap_router: None,
        tikka_token: None,
        metadata_hash: BytesN::from_array(&env, &[7; 32]),
        claim_lockup_seconds: 0,
        swap_deadline_seconds: 0,
    };

    client.init(&factory, &admin, &creator, &config);
    client.deposit_prize();
    client.buy_tickets(&creator, &1);
    client.finalize_raffle();

    let before = client.get_raffle();
    assert_eq!(before.status, RaffleStatus::Finalized);
    assert!(before.prize_deposited);

    env.ledger().set_timestamp(1_000 + EMERGENCY_WITHDRAW_DELAY_SECONDS + 1);

    client.emergency_withdraw(&creator);

    let after = client.get_raffle();
    assert_eq!(after.status, RaffleStatus::Cancelled);
    assert!(!after.prize_deposited);
    client.buy_tickets(&buyer, &1);
    client.finalize_raffle();

    env.ledger().set_timestamp(1_000 + DEFAULT_CLAIM_LOCKUP_SECONDS + 1);
    let winner = client.get_raffle().winners.get(0).unwrap();
    let balance_before = soroban_sdk::token::Client::new(&env, &payment_token).balance(&winner);

    let claimed = client.claim_prize(&winner, &0);
    let gross = MIN_TICKET_PRICE * 10;
    assert_eq!(claimed, gross);

    let balance_after = soroban_sdk::token::Client::new(&env, &payment_token).balance(&winner);
    assert_eq!(balance_after, balance_before + gross);

    let ticket_fee = MIN_TICKET_PRICE * 1_000 / 10_000;
    assert_eq!(client.get_accumulated_fees(), ticket_fee);
}

#[test]
fn prize_distribution_invariant_holds_for_multiple_tiers() {
    let tier_configs: [[u32; 3]; 3] = [[10000, 0, 0], [5000, 5000, 0], [6000, 3000, 1000]];
    let fee_bps = [0u32, 100, 250, 1000, 2000];

    for tiers_raw in tier_configs {
        let tiers_count = if tiers_raw[2] > 0 {
            3
        } else if tiers_raw[1] > 0 {
            2
        } else {
            1
        };

        for fee_bp in fee_bps {
            let env = Env::default();
            env.mock_all_auths();
            env.ledger().set_timestamp(1_000);

            let factory = Address::generate(&env);
            let admin = Address::generate(&env);
            let creator = Address::generate(&env);
            let treasury = Address::generate(&env);
            let buyer_a = Address::generate(&env);
            let buyer_b = Address::generate(&env);
            let buyer_c = Address::generate(&env);

            let token_admin = Address::generate(&env);
            let payment_token = env
                .register_stellar_asset_contract_v2(token_admin.clone())
                .address();
            let token_client = StellarAssetClient::new(&env, &payment_token);
            token_client.mint(&creator, &10_000_000);
            token_client.mint(&buyer_a, &10_000_000);
            token_client.mint(&buyer_b, &10_000_000);
            token_client.mint(&buyer_c, &10_000_000);

            let contract_id = env.register(Contract, ());
            let client = ContractClient::new(&env, &contract_id);

            let prize_amount: i128 = 1_000_000;
            let ticket_price: i128 = MIN_TICKET_PRICE;
            let tickets_to_sell: u32 = tiers_count;
            let total_ticket_sales = ticket_price * tickets_to_sell as i128;
            let expected_ticket_fees = total_ticket_sales * fee_bp as i128 / 10_000;

            let prizes = match tiers_count {
                1 => soroban_sdk::vec![&env, tiers_raw[0]],
                2 => soroban_sdk::vec![&env, tiers_raw[0], tiers_raw[1]],
                _ => soroban_sdk::vec![&env, tiers_raw[0], tiers_raw[1], tiers_raw[2]],
            };

            let config = RaffleConfig {
                description: String::from_str(&env, "Prize invariant"),
                end_time: 0,
                no_deadline: true,
                max_tickets: tickets_to_sell,
                max_tickets_per_tx: tickets_to_sell,
                min_tickets: 1,
                allow_multiple: true,
                ticket_price,
                payment_token: payment_token.clone(),
                prize_amount,
                prizes,
                randomness_source: RandomnessSource::Internal,
                oracle_address: None,
                protocol_fee_bp: fee_bp,
                treasury_address: Some(treasury.clone()),
                swap_router: None,
                tikka_token: None,
                metadata_hash: BytesN::from_array(&env, &[33; 32]),
                claim_lockup_seconds: 0,
                swap_deadline_seconds: 0,
            };

            client.init(&factory, &admin, &creator, &config);
            client.deposit_prize();

            client.buy_tickets(&buyer_a, &1);
            if tickets_to_sell > 1 {
                client.buy_tickets(&buyer_b, &1);
            }
            if tickets_to_sell > 2 {
                client.buy_tickets(&buyer_c, &1);
            }

            client.finalize_raffle();

            let token = soroban_sdk::token::Client::new(&env, &payment_token);
            let contract_balance_before_claims = token.balance(&contract_id);

            env.ledger()
                .set_timestamp(1_000 + DEFAULT_CLAIM_LOCKUP_SECONDS + 1);

            let raffle = client.get_raffle();
            let mut total_claimed = 0i128;
            for i in 0..raffle.winners.len() {
                let winner = raffle.winners.get(i).unwrap();
                let claimed = client.claim_prize(&winner, &i);
                total_claimed += claimed;
            }

            // Core invariant: all prize value is either claimed or explicitly
            // accounted as protocol fee. Current implementation has zero
            // protocol fee on prize claims, so this catches accidental
            // underpayment/double-fee regressions.
            let fee_from_prize = prize_amount - total_claimed;
            assert_eq!(total_claimed + fee_from_prize, prize_amount);
            assert_eq!(fee_from_prize, 0);

            assert_eq!(token.balance(&treasury), expected_ticket_fees);

            let contract_balance_after_claims = token.balance(&contract_id);
            assert_eq!(
                contract_balance_after_claims,
                contract_balance_before_claims - prize_amount
            );
        }
    }
}

#[test]
fn commit_reveal_entropy_is_mixed_from_all_tickets() {
    fn run_seed(commit_b: [u8; 32], metadata_byte: u8) -> u64 {
        let env = Env::default();
        env.mock_all_auths();

        let factory = Address::generate(&env);
        let admin = Address::generate(&env);
        let creator = Address::generate(&env);
        let buyer_a = Address::generate(&env);
        let buyer_b = Address::generate(&env);
        let buyer_c = Address::generate(&env);

        let token_admin = Address::generate(&env);
        let payment_token = env
            .register_stellar_asset_contract_v2(token_admin.clone())
            .address();
        let token_client = StellarAssetClient::new(&env, &payment_token);
        token_client.mint(&creator, &1_000_000);
        token_client.mint(&buyer_a, &1_000_000);
        token_client.mint(&buyer_b, &1_000_000);
        token_client.mint(&buyer_c, &1_000_000);

        let contract_id = env.register(Contract, ());
        let client = ContractClient::new(&env, &contract_id);

        let config = RaffleConfig {
            description: String::from_str(&env, "Commit reveal entropy"),
            end_time: 0,
            no_deadline: true,
            max_tickets: 3,
            max_tickets_per_tx: 3,
            min_tickets: 1,
            allow_multiple: true,
            ticket_price: MIN_TICKET_PRICE,
            payment_token: payment_token.clone(),
            prize_amount: MIN_TICKET_PRICE * 10,
            prizes: soroban_sdk::vec![&env, 6000, 3000, 1000],
            randomness_source: RandomnessSource::CommitReveal,
            oracle_address: None,
            protocol_fee_bp: 0,
            treasury_address: None,
            swap_router: None,
            tikka_token: None,
            metadata_hash: BytesN::from_array(&env, &[metadata_byte; 32]),
            claim_lockup_seconds: 0,
            swap_deadline_seconds: 0,
        };

        client.init(&factory, &admin, &creator, &config);
        client.deposit_prize();
        client.buy_tickets(&buyer_a, &1);
        client.buy_tickets(&buyer_b, &1);
        client.buy_tickets(&buyer_c, &1);

        let commit_a = [1u8; 32];
        let commit_c = [3u8; 32];
        client.submit_commit(&1, &BytesN::from_array(&env, &commit_a));
        client.submit_commit(&2, &BytesN::from_array(&env, &commit_b));
        client.submit_commit(&3, &BytesN::from_array(&env, &commit_c));

        client.finalize_raffle();

        let fairness = client.get_fairness_data();

        let mut combined = Bytes::new(&env);
        combined.extend_from_array(&commit_a);
        combined.extend_from_array(&commit_b);
        combined.extend_from_array(&commit_c);
        let hash: BytesN<32> = env.crypto().sha256(&combined).into();
        let arr = hash.to_array();
        let expected_seed = u64::from_be_bytes([
            arr[0], arr[1], arr[2], arr[3], arr[4], arr[5], arr[6], arr[7],
        ]);

        assert_eq!(fairness.seed, expected_seed);
        fairness.seed
    }

    let seed_original = run_seed([2u8; 32], 44);
    let seed_changed = run_seed([9u8; 32], 45);
    assert_ne!(seed_original, seed_changed);
}

#[test]
fn commit_reveal_preserves_entropy_after_ticket_transfer() {
    let env = Env::default();
    env.mock_all_auths();

    let factory = Address::generate(&env);
    let admin = Address::generate(&env);
    let creator = Address::generate(&env);
    let buyer_a = Address::generate(&env);
    let buyer_b = Address::generate(&env);

    let token_admin = Address::generate(&env);
    let payment_token = env
        .register_stellar_asset_contract_v2(token_admin.clone())
        .address();
    let token_client = StellarAssetClient::new(&env, &payment_token);
    token_client.mint(&creator, &1_000_000);
    token_client.mint(&buyer_a, &1_000_000);
    token_client.mint(&buyer_b, &1_000_000);

    let contract_id = env.register(Contract, ());
    let client = ContractClient::new(&env, &contract_id);

    let config = RaffleConfig {
        description: String::from_str(&env, "Commit survives transfer"),
        end_time: 0,
        no_deadline: true,
        max_tickets: 1,
        max_tickets_per_tx: 1,
        min_tickets: 1,
        allow_multiple: true,
        ticket_price: MIN_TICKET_PRICE,
        payment_token: payment_token.clone(),
        prize_amount: MIN_TICKET_PRICE * 5,
        prizes: soroban_sdk::vec![&env, 10000],
        randomness_source: RandomnessSource::CommitReveal,
        oracle_address: None,
        protocol_fee_bp: 0,
        treasury_address: None,
        swap_router: None,
        tikka_token: None,
        metadata_hash: BytesN::from_array(&env, &[46; 32]),
        claim_lockup_seconds: 0,
        swap_deadline_seconds: 0,
    };

    client.init(&factory, &admin, &creator, &config);
    client.deposit_prize();
    client.buy_tickets(&buyer_a, &1);

    let commit = [7u8; 32];
    client.submit_commit(&1, &BytesN::from_array(&env, &commit));

    // Simulate ownership transfer to validate commit persistence by ticket_id.
    env.as_contract(&contract_id, || {
        let mut ticket: Ticket = env
            .storage()
            .persistent()
            .get(&DataKey::Ticket(1))
            .unwrap();
        ticket.owner = buyer_b.clone();
        env.storage().persistent().set(&DataKey::Ticket(1), &ticket);
    });

    client.finalize_raffle();
    let fairness = client.get_fairness_data();

    let mut combined = Bytes::new(&env);
    combined.extend_from_array(&commit);
    let hash: BytesN<32> = env.crypto().sha256(&combined).into();
    let arr = hash.to_array();
    let expected_seed = u64::from_be_bytes([
        arr[0], arr[1], arr[2], arr[3], arr[4], arr[5], arr[6], arr[7],
    ]);

    assert_eq!(fairness.seed, expected_seed);
}

#[test]
fn commit_reveal_with_zero_commits_falls_back_to_prng() {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().set_timestamp(7_777);
    env.ledger().with_mut(|l| {
        l.sequence_number = 999;
    });

    let factory = Address::generate(&env);
    let admin = Address::generate(&env);
    let creator = Address::generate(&env);
    let buyer_a = Address::generate(&env);
    let buyer_b = Address::generate(&env);

    let token_admin = Address::generate(&env);
    let payment_token = env
        .register_stellar_asset_contract_v2(token_admin.clone())
        .address();
    let token_client = StellarAssetClient::new(&env, &payment_token);
    token_client.mint(&creator, &1_000_000);
    token_client.mint(&buyer_a, &1_000_000);
    token_client.mint(&buyer_b, &1_000_000);

    let contract_id = env.register(Contract, ());
    let client = ContractClient::new(&env, &contract_id);

    let config = RaffleConfig {
        description: String::from_str(&env, "Commit reveal no commits"),
        end_time: 0,
        no_deadline: true,
        max_tickets: 2,
        max_tickets_per_tx: 2,
        min_tickets: 1,
        allow_multiple: true,
        ticket_price: MIN_TICKET_PRICE,
        payment_token: payment_token.clone(),
        prize_amount: MIN_TICKET_PRICE * 8,
        prizes: soroban_sdk::vec![&env, 7000, 3000],
        randomness_source: RandomnessSource::CommitReveal,
        oracle_address: None,
        protocol_fee_bp: 0,
        treasury_address: None,
        swap_router: None,
        tikka_token: None,
        metadata_hash: BytesN::from_array(&env, &[47; 32]),
        claim_lockup_seconds: 0,
        swap_deadline_seconds: 0,
    };

    client.init(&factory, &admin, &creator, &config);
    client.deposit_prize();
    client.buy_tickets(&buyer_a, &1);
    client.buy_tickets(&buyer_b, &1);
    client.finalize_raffle();

    let raffle = client.get_raffle();
    assert_eq!(raffle.status, RaffleStatus::Finalized);

    let fairness = client.get_fairness_data();
    let expected_seed = env.as_contract(&contract_id, || {
        let payload = (
            env.ledger().timestamp(),
            env.ledger().sequence(),
            env.current_contract_address().to_xdr(&env),
        )
            .to_xdr(&env);
        let hash: BytesN<32> = env.crypto().sha256(&payload).into();
        let arr = hash.to_array();
        u64::from_be_bytes([
            arr[0], arr[1], arr[2], arr[3], arr[4], arr[5], arr[6], arr[7],
        ])
    });
    assert_eq!(fairness.seed, expected_seed);
    assert_eq!(raffle.winners.len(), 2);
}

#[test]
fn drawing_lock_cleared_after_internal_finalize() {
    let env = Env::default();
    env.mock_all_auths();

    let factory = Address::generate(&env);
    let admin = Address::generate(&env);
    let creator = Address::generate(&env);

    let token_admin = Address::generate(&env);
    let payment_token = env
        .register_stellar_asset_contract_v2(token_admin.clone())
        .address();
    let token_client = StellarAssetClient::new(&env, &payment_token);
    token_client.mint(&creator, &1_000_000);

    let contract_id = env.register(Contract, ());
    let client = ContractClient::new(&env, &contract_id);

    let config = RaffleConfig {
        description: String::from_str(&env, "Lock internal finalize"),
        end_time: 0,
        no_deadline: true,
        max_tickets: 1,
        max_tickets_per_tx: 1,
        min_tickets: 1,
        allow_multiple: true,
        ticket_price: MIN_TICKET_PRICE,
        payment_token: payment_token.clone(),
        prize_amount: MIN_TICKET_PRICE * 2,
        prizes: soroban_sdk::vec![&env, 10000],
        randomness_source: RandomnessSource::Internal,
        oracle_address: None,
        protocol_fee_bp: 0,
        treasury_address: None,
        swap_router: None,
        tikka_token: None,
        metadata_hash: BytesN::from_array(&env, &[48; 32]),
        claim_lockup_seconds: 0,
        swap_deadline_seconds: 0,
    };

    client.init(&factory, &admin, &creator, &config);
    client.deposit_prize();
    client.buy_tickets(&creator, &1);
    client.finalize_raffle();

    assert_drawing_lock_cleared(&env, &contract_id);
}

#[test]
fn drawing_lock_cleared_after_oracle_randomness() {
    let env = Env::default();
    env.mock_all_auths();

    let factory = Address::generate(&env);
    let admin = Address::generate(&env);
    let creator = Address::generate(&env);
    let oracle = Address::generate(&env);

    let token_admin = Address::generate(&env);
    let payment_token = env
        .register_stellar_asset_contract_v2(token_admin.clone())
        .address();
    let token_client = StellarAssetClient::new(&env, &payment_token);
    token_client.mint(&creator, &1_000_000);

    let contract_id = env.register(Contract, ());
    let client = ContractClient::new(&env, &contract_id);

    let config = RaffleConfig {
        description: String::from_str(&env, "Lock oracle finalize"),
        end_time: 0,
        no_deadline: true,
        max_tickets: 1,
        max_tickets_per_tx: 1,
        min_tickets: 1,
        allow_multiple: true,
        ticket_price: MIN_TICKET_PRICE,
        payment_token: payment_token.clone(),
        prize_amount: MIN_TICKET_PRICE * 2,
        prizes: soroban_sdk::vec![&env, 10000],
        randomness_source: RandomnessSource::External,
        oracle_address: Some(oracle),
        protocol_fee_bp: 0,
        treasury_address: None,
        swap_router: None,
        tikka_token: None,
        metadata_hash: BytesN::from_array(&env, &[49; 32]),
        claim_lockup_seconds: 0,
        swap_deadline_seconds: 0,
    };

    client.init(&factory, &admin, &creator, &config);
    client.deposit_prize();
    client.buy_tickets(&creator, &1);

    let request_id: u64 = env.as_contract(&contract_id, || {
        env.storage()
            .instance()
            .get(&DataKey::RandomnessRequestId)
            .unwrap()
    });

    let signing_key = SigningKey::from_bytes(&[5u8; 32]);
    let verifying = signing_key.verifying_key();
    let message = env.as_contract(&contract_id, || {
        build_vrf_proof_message(&env, request_id, 424242)
    });
    let signature = signing_key.sign(message.as_slice());

    client.provide_randomness(
        &424242,
        &BytesN::from_array(&env, &verifying.to_bytes()),
        &BytesN::from_array(&env, &signature.to_bytes()),
        &request_id,
    );

    assert_drawing_lock_cleared(&env, &contract_id);
}

#[test]
fn drawing_lock_cleared_after_fallback_refund() {
    let env = Env::default();
    env.mock_all_auths();

    let factory = Address::generate(&env);
    let admin = Address::generate(&env);
    let creator = Address::generate(&env);
    let oracle = Address::generate(&env);

    let token_admin = Address::generate(&env);
    let payment_token = env
        .register_stellar_asset_contract_v2(token_admin.clone())
        .address();
    let token_client = StellarAssetClient::new(&env, &payment_token);
    token_client.mint(&creator, &1_000_000);

    let contract_id = env.register(Contract, ());
    let client = ContractClient::new(&env, &contract_id);

    let config = RaffleConfig {
        description: String::from_str(&env, "Lock fallback refund"),
        end_time: 0,
        no_deadline: true,
        max_tickets: 1,
        max_tickets_per_tx: 1,
        min_tickets: 1,
        allow_multiple: true,
        ticket_price: MIN_TICKET_PRICE,
        payment_token: payment_token.clone(),
        prize_amount: MIN_TICKET_PRICE * 2,
        prizes: soroban_sdk::vec![&env, 10000],
        randomness_source: RandomnessSource::External,
        oracle_address: Some(oracle),
        protocol_fee_bp: 0,
        treasury_address: None,
        swap_router: None,
        tikka_token: None,
        metadata_hash: BytesN::from_array(&env, &[50; 32]),
        claim_lockup_seconds: 0,
        swap_deadline_seconds: 0,
    };

    client.init(&factory, &admin, &creator, &config);
    client.deposit_prize();
    client.buy_tickets(&creator, &1);

    env.ledger().with_mut(|l| {
        l.sequence_number += ORACLE_TIMEOUT_LEDGERS + 1;
    });
    client.trigger_randomness_fallback(&creator, &true);

    assert_drawing_lock_cleared(&env, &contract_id);
}

#[test]
fn drawing_lock_cleared_after_fallback_no_refund() {
    let env = Env::default();
    env.mock_all_auths();

    let factory = Address::generate(&env);
    let admin = Address::generate(&env);
    let creator = Address::generate(&env);
    let oracle = Address::generate(&env);

    let token_admin = Address::generate(&env);
    let payment_token = env
        .register_stellar_asset_contract_v2(token_admin.clone())
        .address();
    let token_client = StellarAssetClient::new(&env, &payment_token);
    token_client.mint(&creator, &1_000_000);

    let contract_id = env.register(Contract, ());
    let client = ContractClient::new(&env, &contract_id);

    let config = RaffleConfig {
        description: String::from_str(&env, "Lock fallback finalize"),
        end_time: 0,
        no_deadline: true,
        max_tickets: 1,
        max_tickets_per_tx: 1,
        min_tickets: 1,
        allow_multiple: true,
        ticket_price: MIN_TICKET_PRICE,
        payment_token: payment_token.clone(),
        prize_amount: MIN_TICKET_PRICE * 2,
        prizes: soroban_sdk::vec![&env, 10000],
        randomness_source: RandomnessSource::External,
        oracle_address: Some(oracle),
        protocol_fee_bp: 0,
        treasury_address: None,
        swap_router: None,
        tikka_token: None,
        metadata_hash: BytesN::from_array(&env, &[51; 32]),
        claim_lockup_seconds: 0,
        swap_deadline_seconds: 0,
    };

    client.init(&factory, &admin, &creator, &config);
    client.deposit_prize();
    client.buy_tickets(&creator, &1);

    env.ledger().with_mut(|l| {
        l.sequence_number += ORACLE_TIMEOUT_LEDGERS + 1;
    });
    client.trigger_randomness_fallback(&creator, &false);

    assert_drawing_lock_cleared(&env, &contract_id);
}

#[test]
fn drawing_lock_cleared_after_cancel_in_drawing_state() {
    let env = Env::default();
    env.mock_all_auths();

    let factory = Address::generate(&env);
    let admin = Address::generate(&env);
    let creator = Address::generate(&env);
    let oracle = Address::generate(&env);

    let token_admin = Address::generate(&env);
    let payment_token = env
        .register_stellar_asset_contract_v2(token_admin.clone())
        .address();
    let token_client = StellarAssetClient::new(&env, &payment_token);
    token_client.mint(&creator, &1_000_000);

    let contract_id = env.register(Contract, ());
    let client = ContractClient::new(&env, &contract_id);

    let config = RaffleConfig {
        description: String::from_str(&env, "Lock cancel drawing"),
        end_time: 0,
        no_deadline: true,
        max_tickets: 1,
        max_tickets_per_tx: 1,
        min_tickets: 1,
        allow_multiple: true,
        ticket_price: MIN_TICKET_PRICE,
        payment_token: payment_token.clone(),
        prize_amount: MIN_TICKET_PRICE * 2,
        prizes: soroban_sdk::vec![&env, 10000],
        randomness_source: RandomnessSource::External,
        oracle_address: Some(oracle),
        protocol_fee_bp: 0,
        treasury_address: None,
        swap_router: None,
        tikka_token: None,
        metadata_hash: BytesN::from_array(&env, &[52; 32]),
        claim_lockup_seconds: 0,
        swap_deadline_seconds: 0,
    };

    client.init(&factory, &admin, &creator, &config);
    client.deposit_prize();
    client.buy_tickets(&creator, &1);
    client.cancel_raffle(&CancelReason::CreatorCancelled);

    assert_drawing_lock_cleared(&env, &contract_id);
}

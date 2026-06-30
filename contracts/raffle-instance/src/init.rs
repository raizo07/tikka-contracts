use soroban_sdk::{token, Address, BytesN, Env};

use raffle_shared::{RaffleConfig, RandomnessSource};

use crate::events::{PrizeDeposited, RaffleCreated, RaffleStatusChanged};
use crate::{
    read_raffle, require_not_paused, validate_token_address, write_raffle, DataKey, Error, Raffle,
    MAX_CLAIM_LOCKUP_SECONDS, MAX_DESCRIPTION_LENGTH, MAX_PRIZES, MAX_PRIZE_AMOUNT,
    MAX_SWAP_DEADLINE_SECONDS, MAX_TICKETS_LIMIT, MIN_TICKET_PRICE, RaffleStatus,
};

pub(crate) fn init(
    env: Env,
    factory: Address,
    admin: Address,
    creator: Address,
    config: RaffleConfig,
) -> Result<(), Error> {
    if env.storage().instance().has(&DataKey::Raffle) {
        return Err(Error::AlreadyInitialized);
    }

    if config.description.len() > MAX_DESCRIPTION_LENGTH {
        return Err(Error::InvalidParameters);
    }

    let now = env.ledger().timestamp();
    if config.no_deadline && config.end_time != 0 {
        return Err(Error::InvalidParameters);
    }
    if !config.no_deadline && config.end_time <= now {
        return Err(Error::InvalidParameters);
    }
    if config.end_time != 0 && config.end_time <= now {
        return Err(Error::InvalidEndTime);
    }
    if config.max_tickets == 0 || config.max_tickets > MAX_TICKETS_LIMIT {
        return Err(Error::InvalidParameters);
    }
    if config.max_tickets < config.min_tickets {
        return Err(Error::InvalidTicketRange);
    }
    if config.max_tickets_per_tx == 0 || config.max_tickets_per_tx > config.max_tickets {
        return Err(Error::InvalidParameters);
    }
    if config.ticket_price < MIN_TICKET_PRICE {
        return Err(Error::InvalidParameters);
    }
    if config.prize_amount < config.ticket_price {
        return Err(Error::InvalidParameters);
    }
    if config.prize_amount > MAX_PRIZE_AMOUNT {
        return Err(Error::InvalidParameters);
    }
    if config.prizes.is_empty() {
        return Err(Error::InvalidParameters);
    }
    if config.prizes.len() > MAX_PRIZES {
        return Err(Error::TooManyPrizes);
    }
    let mut total = 0u32;
    for bp in config.prizes.iter() {
        total += bp;
    }
    if total != 10000 {
        return Err(Error::InvalidParameters);
    }
    if config.protocol_fee_bp > 10000 {
        return Err(Error::InvalidParameters);
    }
    if config.randomness_source == RandomnessSource::External {
        match &config.oracle_address {
            None => return Err(Error::InvalidParameters),
            Some(addr) if *addr == env.current_contract_address() => return Err(Error::InvalidParameters),
            Some(_) => {}
        }
    }
    if config.randomness_source != RandomnessSource::External && config.oracle_address.is_some() {
        return Err(Error::InvalidParameters);
    }
    if config.metadata_hash == BytesN::from_array(&env, &[0u8; 32]) {
        return Err(Error::InvalidParameters);
    }

    validate_token_address(&env, &config.payment_token)?;
    let config = config.resolve_defaults();

    if config.claim_lockup_seconds > MAX_CLAIM_LOCKUP_SECONDS {
        return Err(Error::InvalidParameters);
    }
    if config.swap_deadline_seconds > MAX_SWAP_DEADLINE_SECONDS {
        return Err(Error::InvalidParameters);
    }

    let raffle = Raffle {
        creator: creator.clone(),
        description: config.description.clone(),
        end_time: config.end_time,
        no_deadline: config.no_deadline,
        max_tickets: config.max_tickets,
        max_tickets_per_tx: config.max_tickets_per_tx,
        min_tickets: config.min_tickets,
        allow_multiple: config.allow_multiple,
        ticket_price: config.ticket_price,
        payment_token: config.payment_token.clone(),
        prize_amount: config.prize_amount,
        prizes: config.prizes.clone(),
        tickets_sold: 0,
        status: RaffleStatus::PendingPrize,
        prize_deposited: false,
        winners: soroban_sdk::Vec::new(&env),
        claimed_winners: soroban_sdk::Vec::new(&env),
        randomness_source: config.randomness_source.clone(),
        oracle_address: config.oracle_address,
        protocol_fee_bp: config.protocol_fee_bp,
        treasury_address: config.treasury_address,
        swap_router: config.swap_router,
        tikka_token: config.tikka_token,
        finalized_at: None,
        claim_lockup_seconds: config.claim_lockup_seconds,
        swap_deadline_seconds: config.swap_deadline_seconds,
        ticket_sales_paused: false,
    };
    write_raffle(&env, &raffle);
    env.storage().instance().set(&DataKey::Factory, &factory);
    env.storage().instance().set(&DataKey::Admin, &admin);

    RaffleCreated {
        raffle_id: env.current_contract_address(),
        creator,
        end_time: config.end_time,
        max_tickets: config.max_tickets,
        ticket_price: config.ticket_price,
        payment_token: config.payment_token,
        prize_amount: config.prize_amount,
        prizes: config.prizes,
        description: config.description,
        randomness_source: config.randomness_source,
        metadata_hash: config.metadata_hash,
    }.publish(&env);

    Ok(())
}

pub(crate) fn deposit_prize(env: Env) -> Result<(), Error> {
    require_not_paused(&env)?;
    let mut raffle = read_raffle(&env)?;
    raffle.creator.require_auth();

    if raffle.prize_deposited {
        return Err(Error::PrizeAlreadyDeposited);
    }

    let old_status = raffle.status.clone();

    let token_client = token::Client::new(&env, &raffle.payment_token);
    let _ = token_client
        .try_transfer(&raffle.creator, env.current_contract_address(), &raffle.prize_amount)
        .map_err(|_| Error::TokenTransferFailed)?;

    raffle.prize_deposited = true;
    raffle.status = RaffleStatus::Active;
    write_raffle(&env, &raffle);

    let ts = env.ledger().timestamp();
    PrizeDeposited { creator: raffle.creator.clone(), amount: raffle.prize_amount, token: raffle.payment_token.clone(), timestamp: ts }.publish(&env);
    RaffleStatusChanged { old_status, new_status: RaffleStatus::Active, timestamp: ts }.publish(&env);

    Ok(())
}

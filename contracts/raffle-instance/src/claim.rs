use soroban_sdk::{token, Address, Env};

use crate::events::{PrizeClaimed, PrizeRefunded, RaffleStatusChanged, TicketRefunded};
use crate::{
    calculate_tier_prize, read_raffle, write_raffle, DataKey, Error, Guard, RaffleStatus,
};

pub(crate) fn claim_prize(env: Env, winner: Address, tier_index: u32) -> Result<i128, Error> {
    winner.require_auth();
    let _guard = Guard::new(&env)?;
    let mut raffle = read_raffle(&env)?;

    if raffle.status != RaffleStatus::Finalized { return Err(Error::InvalidStatus); }
    if let Some(fa) = raffle.finalized_at {
        if env.ledger().timestamp() < fa + raffle.claim_lockup_seconds { return Err(Error::ClaimTooEarly); }
    }
    if tier_index >= raffle.winners.len() { return Err(Error::InvalidParameters); }
    if raffle.winners.get(tier_index).ok_or(Error::InvalidIndex)? != winner { return Err(Error::NotWinner); }
    if raffle.claimed_winners.get(tier_index).ok_or(Error::InvalidIndex)? { return Err(Error::PrizeAlreadyClaimed); }

    let amount = calculate_tier_prize(&raffle, tier_index)?;
    if amount <= 0 { return Err(Error::ZeroPrize); }

    raffle.claimed_winners.set(tier_index, true);

    let mut all_claimed = true;
    for c in raffle.claimed_winners.iter() { if !c { all_claimed = false; break; } }
    if all_claimed {
        raffle.status = RaffleStatus::Claimed;
        RaffleStatusChanged { old_status: RaffleStatus::Finalized, new_status: RaffleStatus::Claimed, timestamp: env.ledger().timestamp() }.publish(&env);
    }
    write_raffle(&env, &raffle);

    let tc = token::Client::new(&env, &raffle.payment_token);
    let _ = tc.try_transfer(&env.current_contract_address(), &winner, &amount).map_err(|_| Error::TokenTransferFailed)?;

    PrizeClaimed { winner, tier_index, payment_token: raffle.payment_token.clone(), gross_amount: amount, net_amount: amount, platform_fee: 0, claimed_at: env.ledger().timestamp() }.publish(&env);
    Ok(amount)
}

pub(crate) fn refund_prize(env: Env) -> Result<(), Error> {
    let mut raffle = read_raffle(&env)?;
    raffle.creator.require_auth();

    if raffle.status != RaffleStatus::Cancelled && raffle.status != RaffleStatus::Failed { return Err(Error::InvalidStatus); }
    if !raffle.prize_deposited { return Err(Error::PrizeNotDeposited); }

    raffle.prize_deposited = false;
    write_raffle(&env, &raffle);

    let tc = token::Client::new(&env, &raffle.payment_token);
    let _ = tc.try_transfer(&env.current_contract_address(), &raffle.creator, &raffle.prize_amount).map_err(|_| Error::TokenTransferFailed)?;

    PrizeRefunded { creator: raffle.creator.clone(), amount: raffle.prize_amount, token: raffle.payment_token.clone(), timestamp: env.ledger().timestamp() }.publish(&env);
    Ok(())
}

pub(crate) fn refund_ticket(env: Env, ticket_id: u32) -> Result<i128, Error> {
    let raffle = read_raffle(&env)?;
    if raffle.status != RaffleStatus::Cancelled && raffle.status != RaffleStatus::Failed { return Err(Error::InvalidStatus); }

    let _guard = Guard::new(&env)?;
    let ticket: crate::Ticket = env.storage().persistent().get(&DataKey::Ticket(ticket_id)).ok_or(Error::TicketNotFound)?;
    ticket.owner.require_auth();

    if env.storage().persistent().has(&DataKey::TicketRefunded(ticket_id)) { return Err(Error::PrizeAlreadyClaimed); }
    env.storage().persistent().set(&DataKey::TicketRefunded(ticket_id), &true);

    let tc = token::Client::new(&env, &raffle.payment_token);
    let _ = tc.try_transfer(&env.current_contract_address(), &ticket.owner, &raffle.ticket_price).map_err(|_| Error::TokenTransferFailed)?;

    TicketRefunded { buyer: ticket.owner, ticket_number: ticket.ticket_number, amount: raffle.ticket_price, timestamp: env.ledger().timestamp() }.publish(&env);
    Ok(raffle.ticket_price)
}

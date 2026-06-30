# Tikka Protocol Fee Model

## Fee Collection Points

### 1. At Ticket Purchase
- **Formula:** `ticket_price × protocol_fee_bp / 10000` per ticket
- **Recipient:** Treasury address
- **Payer:** Ticket buyer
- **Example:** 2.5% fee on 100 XLM ticket = 2.5 XLM to treasury, 97.5 XLM to contract

### 2. At Prize Claim
- **Formula:** `prize_tier_amount × protocol_fee_bp / 10000`
- **Recipient:** Treasury address
- **Payer:** Prize winner (deducted from payout)
- **Example:** 2.5% fee on 1000 XLM prize tier = 25 XLM to treasury, 975 XLM to winner

## Effective Total Fee

For a raffle with protocol_fee_bp = 250 (2.5%), ticket_price = 100 XLM, 10 tickets, prize = 800 XLM:
- Ticket fees: 10 × 2.5 XLM = 25 XLM
- Prize claim fee: 800 × 2.5% = 20 XLM  
- **Total protocol revenue: 45 XLM**

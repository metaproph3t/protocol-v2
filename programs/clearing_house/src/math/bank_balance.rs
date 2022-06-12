use crate::error::{ClearingHouseResult, ErrorCode};
use crate::math::casting::cast_to_u64;
use crate::math::constants::{BANK_UTILIZATION_PRECISION, ONE_YEAR};
use crate::math_error;
use crate::state::bank::Bank;
use crate::state::user::BankBalanceType;
use solana_program::msg;

pub fn get_bank_balance(
    token_amount: u128,
    bank: &Bank,
    balance_type: &BankBalanceType,
) -> ClearingHouseResult<u128> {
    let precision_increase = 10_u128.pow(
        12_u8
            .checked_sub(bank.decimals)
            .ok_or_else(math_error!())?
            .into(),
    );

    let cumulative_interest = match balance_type {
        BankBalanceType::Deposit => bank.cumulative_deposit_interest,
        BankBalanceType::Borrow => bank.cumulative_borrow_interest,
    };

    let mut balance = token_amount
        .checked_mul(precision_increase)
        .ok_or_else(math_error!())?
        .checked_div(cumulative_interest)
        .ok_or_else(math_error!())?;

    if balance != 0 && balance_type == &BankBalanceType::Borrow {
        balance = balance.checked_add(1).ok_or_else(math_error!())?;
    }

    Ok(balance)
}

pub fn get_token_amount(
    balance: u128,
    bank: &Bank,
    balance_type: &BankBalanceType,
) -> ClearingHouseResult<u128> {
    let precision_decrease = 10_u128.pow(
        12_u8
            .checked_sub(bank.decimals)
            .ok_or_else(math_error!())?
            .into(),
    );

    let cumulative_interest = match balance_type {
        BankBalanceType::Deposit => bank.cumulative_deposit_interest,
        BankBalanceType::Borrow => bank.cumulative_borrow_interest,
    };

    let mut token_amount = balance
        .checked_mul(cumulative_interest)
        .ok_or_else(math_error!())?
        .checked_div(precision_decrease)
        .ok_or_else(math_error!())?;

    if token_amount != 0 && balance_type == &BankBalanceType::Borrow {
        token_amount = token_amount.checked_add(1).ok_or_else(math_error!())?;
    }

    Ok(token_amount)
}

pub fn get_bank_deposit_token_amount(bank: &Bank) -> ClearingHouseResult<u128> {
    get_token_amount(bank.deposit_balance, bank, &BankBalanceType::Deposit)
}

pub fn get_bank_borrow_token_amount(bank: &Bank) -> ClearingHouseResult<u128> {
    get_token_amount(bank.borrow_balance, bank, &BankBalanceType::Borrow)
}

pub struct CumulativeInterestDelta {
    pub borrow_delta: u128,
    pub deposit_delta: u128,
}

pub fn get_cumulative_interest_delta(
    bank: &Bank,
    now: i64,
) -> ClearingHouseResult<CumulativeInterestDelta> {
    let deposit_token_amount = get_bank_deposit_token_amount(bank)?;
    let borrow_token_amount = get_bank_borrow_token_amount(bank)?;

    let utilization = borrow_token_amount
        .checked_mul(BANK_UTILIZATION_PRECISION)
        .ok_or_else(math_error!())?
        .checked_div(deposit_token_amount)
        .or_else(|| {
            if deposit_token_amount == 0 && borrow_token_amount == 0 {
                Some(0_u128)
            } else {
                // if there are borrows without deposits, default to maximum utilization rate
                Some(BANK_UTILIZATION_PRECISION)
            }
        })
        .unwrap();

    let interest_rate = if utilization > bank.optimal_utilization {
        let surplus_utilization = utilization
            .checked_sub(bank.optimal_utilization)
            .ok_or_else(math_error!())?;

        let borrow_rate_slope = bank
            .max_borrow_rate
            .checked_sub(bank.optimal_borrow_rate)
            .ok_or_else(math_error!())?
            .checked_mul(BANK_UTILIZATION_PRECISION)
            .ok_or_else(math_error!())?
            .checked_div(
                BANK_UTILIZATION_PRECISION
                    .checked_sub(bank.optimal_utilization)
                    .ok_or_else(math_error!())?,
            )
            .ok_or_else(math_error!())?;

        bank.optimal_borrow_rate
            .checked_add(
                surplus_utilization
                    .checked_mul(borrow_rate_slope)
                    .ok_or_else(math_error!())?
                    .checked_div(BANK_UTILIZATION_PRECISION)
                    .ok_or_else(math_error!())?,
            )
            .ok_or_else(math_error!())?
    } else {
        let borrow_rate_slope = bank
            .optimal_borrow_rate
            .checked_mul(BANK_UTILIZATION_PRECISION)
            .ok_or_else(math_error!())?
            .checked_div(bank.optimal_utilization)
            .ok_or_else(math_error!())?;

        utilization
            .checked_mul(borrow_rate_slope)
            .ok_or_else(math_error!())?
            .checked_div(BANK_UTILIZATION_PRECISION)
            .ok_or_else(math_error!())?
    };

    let time_since_last_update = cast_to_u64(now)
        .or(Err(ErrorCode::UnableToCastUnixTime))?
        .checked_sub(bank.last_updated)
        .ok_or_else(math_error!())?;

    let borrow_interest = interest_rate
        .checked_mul(time_since_last_update as u128)
        .ok_or_else(math_error!())?;

    let deposit_interest = borrow_interest
        .checked_mul(utilization)
        .ok_or_else(math_error!())?
        .checked_div(BANK_UTILIZATION_PRECISION)
        .ok_or_else(math_error!())?;

    let borrow_delta = bank
        .cumulative_borrow_interest
        .checked_mul(borrow_interest)
        .ok_or_else(math_error!())?
        .checked_div(ONE_YEAR)
        .ok_or_else(math_error!())?;

    let deposit_delta = bank
        .cumulative_deposit_interest
        .checked_mul(deposit_interest)
        .ok_or_else(math_error!())?
        .checked_div(ONE_YEAR)
        .ok_or_else(math_error!())?;

    Ok(CumulativeInterestDelta {
        borrow_delta,
        deposit_delta,
    })
}

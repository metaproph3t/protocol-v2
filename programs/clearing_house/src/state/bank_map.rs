use crate::error::{ClearingHouseResult, ErrorCode};
use crate::state::bank::Bank;
use anchor_lang::prelude::{AccountInfo, AccountLoader, Pubkey};
use std::cell::{Ref, RefMut};
use std::collections::{BTreeMap, BTreeSet};

use std::iter::Peekable;
use std::slice::Iter;

use crate::state::oracle_map::OracleMap;
use anchor_lang::Discriminator;
use arrayref::array_ref;

pub struct BankMap<'a>(pub BTreeMap<u64, AccountLoader<'a, Bank>>);

impl<'a> BankMap<'a> {
    pub fn get_ref(&self, bank_index: &u64) -> ClearingHouseResult<Ref<Bank>> {
        self.0
            .get(bank_index)
            .ok_or(ErrorCode::BankNotFound)?
            .load()
            .or(Err(ErrorCode::UnableToLoadBankAccount))
    }

    pub fn get_ref_mut(&self, bank_index: &u64) -> ClearingHouseResult<RefMut<Bank>> {
        self.0
            .get(bank_index)
            .ok_or(ErrorCode::BankNotFound)?
            .load_mut()
            .or(Err(ErrorCode::UnableToLoadBankAccount))
    }

    pub fn load<'b, 'c, 'd>(
        writable_banks: &'b WritableBanks,
        oracle_map: &'d OracleMap,
        account_info_iter: &'c mut Peekable<Iter<AccountInfo<'a>>>,
    ) -> ClearingHouseResult<BankMap<'a>> {
        let mut market_map: BankMap = BankMap(BTreeMap::new());

        let market_discriminator: [u8; 8] = Bank::discriminator();
        while let Some(account_info) = account_info_iter.peek() {
            let data = account_info
                .try_borrow_data()
                .or(Err(ErrorCode::CouldNotLoadBankData))?;

            if data.len() < std::mem::size_of::<Bank>() + 8 {
                break;
            }

            let account_discriminator = array_ref![data, 0, 8];
            if account_discriminator != &market_discriminator {
                break;
            }

            let bank_index = u64::from_le_bytes(*array_ref![data, 8, 8]);
            let oracle = Pubkey::new(array_ref![data, 49, 32]);

            let account_info = account_info_iter.next().unwrap();
            let is_writable = account_info.is_writable;
            let account_loader: AccountLoader<Bank> =
                AccountLoader::try_from(account_info).or(Err(ErrorCode::InvalidBankAccount))?;

            if writable_banks.contains(&bank_index) && !is_writable {
                return Err(ErrorCode::BankWrongMutability);
            }

            if !oracle_map.contains(&oracle) {}

            market_map.0.insert(bank_index, account_loader);
        }

        Ok(market_map)
    }
}

pub type WritableBanks = BTreeSet<u64>;

pub fn get_writable_banks(bank_index: u64) -> WritableBanks {
    let mut writable_markets = WritableBanks::new();
    writable_markets.insert(bank_index);
    writable_markets
}

use crate::error::{DriftResult, ErrorCode};
use crate::math::casting::Cast;
use crate::math::margin::MarginRequirementType;
use crate::math::safe_math::SafeMath;
use crate::{validate, MarketType, MARGIN_PRECISION_U128, PRICE_PRECISION};
use anchor_lang::solana_program::msg;

#[derive(Clone, Copy, Debug)]
pub enum MarginCalculationMode {
    Standard,
    Liquidation {
        margin_buffer: u128,
        track_margin_ratio: bool,
        market_to_track: Option<(MarketType, u16)>,
    },
}

#[derive(Clone, Copy, Debug)]
pub struct MarginContext {
    pub margin_type: MarginRequirementType,
    pub mode: MarginCalculationMode,
    pub strict: bool,
}

impl MarginContext {
    pub fn standard(margin_type: MarginRequirementType) -> Self {
        Self {
            margin_type,
            mode: MarginCalculationMode::Standard,
            strict: false,
        }
    }

    pub fn strict(mut self, strict: bool) -> Self {
        self.strict = strict;
        self
    }

    pub fn liquidation(margin_buffer: u32) -> Self {
        Self {
            margin_type: MarginRequirementType::Maintenance,
            mode: MarginCalculationMode::Liquidation {
                margin_buffer: margin_buffer as u128,
                track_margin_ratio: false,
                market_to_track: None,
            },
            strict: false,
        }
    }

    pub fn track_margin_ratio(mut self) -> DriftResult<Self> {
        match self.mode {
            MarginCalculationMode::Liquidation {
                ref mut track_margin_ratio,
                ..
            } => {
                *track_margin_ratio = true;
            }
            _ => {
                msg!("Cant track margin ratio outside of liquidation mode");
                return Err(ErrorCode::InvalidMarginCalculation);
            }
        }
        Ok(self)
    }

    pub fn track_market(mut self, market: (MarketType, u16)) -> DriftResult<Self> {
        match self.mode {
            MarginCalculationMode::Liquidation {
                ref mut market_to_track,
                ..
            } => {
                *market_to_track = Some(market);
            }
            _ => {
                msg!("Cant track market outside of liquidation mode");
                return Err(ErrorCode::InvalidMarginCalculation);
            }
        }
        Ok(self)
    }
}

#[derive(Clone, Copy, Debug)]
pub struct MarginCalculation {
    pub context: MarginContext,
    pub total_collateral: i128,
    pub margin_requirement: u128,
    #[cfg(not(test))]
    margin_requirement_plus_buffer: u128,
    #[cfg(test)]
    pub margin_requirement_plus_buffer: u128,
    pub num_spot_liabilities: u8,
    pub num_perp_liabilities: u8,
    pub all_oracles_valid: bool,
    /// TODO need to implement this
    pub with_isolated_liability: bool,
    pub total_spot_asset_value: i128,
    pub total_spot_liability_value: u128,
    pub total_perp_liability_value: u128,
    tracked_market_margin_requirement: u128,
}

impl MarginCalculation {
    pub fn new(context: MarginContext) -> Self {
        Self {
            context,
            total_collateral: 0,
            margin_requirement: 0,
            margin_requirement_plus_buffer: 0,
            num_spot_liabilities: 0,
            num_perp_liabilities: 0,
            all_oracles_valid: true,
            with_isolated_liability: false,
            total_spot_asset_value: 0,
            total_spot_liability_value: 0,
            total_perp_liability_value: 0,
            tracked_market_margin_requirement: 0,
        }
    }

    pub fn add_total_collateral(&mut self, total_collateral: i128) -> DriftResult {
        self.total_collateral = self.total_collateral.safe_add(total_collateral)?;
        Ok(())
    }

    pub fn add_margin_requirement(
        &mut self,
        margin_requirement: u128,
        liability_value: u128,
        market: (MarketType, u16),
    ) -> DriftResult {
        self.margin_requirement = self.margin_requirement.safe_add(margin_requirement)?;
        if let MarginCalculationMode::Liquidation { margin_buffer, .. } = self.context.mode {
            self.margin_requirement_plus_buffer = self
                .margin_requirement_plus_buffer
                .safe_add(margin_requirement.safe_add(
                    liability_value.safe_mul(margin_buffer)? / MARGIN_PRECISION_U128,
                )?)?;
        }

        if let Some(market_to_track) = self.market_to_track() {
            if market_to_track == market {
                self.tracked_market_margin_requirement = self
                    .tracked_market_margin_requirement
                    .safe_add(margin_requirement)?;
            }
        }

        Ok(())
    }

    pub fn add_spot_asset_value(
        &mut self,
        asset_value_fn: &dyn Fn() -> DriftResult<i128>,
    ) -> DriftResult {
        if self.track_margin_ratio_enabled() {
            self.total_spot_asset_value = self.total_spot_asset_value.safe_add(asset_value_fn()?)?;
        }

        Ok(())
    }

    pub fn add_spot_liability_value(
        &mut self,
        liability_value_fn: &dyn Fn() -> DriftResult<u128>,
    ) -> DriftResult {
        if self.track_margin_ratio_enabled() {
            self.total_spot_liability_value = self
                .total_spot_liability_value
                .safe_add(liability_value_fn()?)?;
        }

        Ok(())
    }

    pub fn add_perp_liability_value(
        &mut self,
        perp_value_fn: &dyn Fn() -> DriftResult<u128>,
    ) -> DriftResult {
        if self.track_margin_ratio_enabled() {
            self.total_perp_liability_value =
                self.total_perp_liability_value.safe_add(perp_value_fn()?)?;
        }

        Ok(())
    }

    pub fn add_spot_liability(&mut self) -> DriftResult {
        self.num_spot_liabilities = self.num_spot_liabilities.safe_add(1)?;
        Ok(())
    }

    pub fn add_perp_liability(&mut self) -> DriftResult {
        self.num_perp_liabilities = self.num_perp_liabilities.safe_add(1)?;
        Ok(())
    }

    pub fn update_all_oracles_valid(&mut self, valid: bool) {
        self.all_oracles_valid &= valid;
    }

    pub fn validate_num_spot_liabilities(&self) -> DriftResult {
        if self.num_spot_liabilities > 0 {
            validate!(
                self.margin_requirement > 0,
                ErrorCode::InvalidMarginRatio,
                "num_spot_liabilities={} but margin_requirement=0",
                self.num_spot_liabilities
            )?;
        }
        Ok(())
    }

    pub fn get_num_of_liabilities(&self) -> DriftResult<u8> {
        self.num_spot_liabilities
            .safe_add(self.num_perp_liabilities)
    }

    pub fn meets_margin_requirement(&self) -> bool {
        self.total_collateral >= self.margin_requirement as i128
    }

    pub fn can_exit_liquidation(&self) -> DriftResult<bool> {
        if !self.is_liquidation_mode() {
            msg!("liquidation mode not enabled");
            return Err(ErrorCode::InvalidMarginCalculation);
        }

        Ok(self.total_collateral >= self.margin_requirement_plus_buffer as i128)
    }

    pub fn margin_shortage(&self) -> DriftResult<u128> {
        Ok(self
            .margin_requirement_plus_buffer
            .cast::<i128>()?
            .safe_sub(self.total_collateral)?
            .unsigned_abs())
    }

    pub fn tracked_market_margin_shortage(&self, margin_shortage: u128) -> DriftResult<u128> {
        if self.market_to_track().is_none() {
            msg!("cant call tracked_market_margin_shortage");
            return Err(ErrorCode::InvalidMarginCalculation);
        }

        if self.margin_requirement == 0 {
            return Ok(0);
        }

        margin_shortage
            .safe_mul(self.tracked_market_margin_requirement)?
            .safe_div(self.margin_requirement)
    }

    pub fn get_free_collateral(&self) -> DriftResult<u128> {
        self.total_collateral
            .safe_sub(self.margin_requirement.cast::<i128>()?)?
            .max(0)
            .cast()
    }

    fn track_margin_ratio_enabled(&self) -> bool {
        if let MarginCalculationMode::Liquidation {
            track_margin_ratio, ..
        } = self.context.mode
        {
            track_margin_ratio
        } else {
            false
        }
    }

    fn market_to_track(&self) -> Option<(MarketType, u16)> {
        if let MarginCalculationMode::Liquidation {
            market_to_track: track_margin_requirement,
            ..
        } = self.context.mode
        {
            track_margin_requirement
        } else {
            None
        }
    }

    fn is_liquidation_mode(&self) -> bool {
        matches!(self.context.mode, MarginCalculationMode::Liquidation { .. })
    }

    pub fn get_margin_ratio(&self) -> DriftResult<u128> {
        if !self.track_margin_ratio_enabled() {
            msg!("track margin ratio is not enabled");
            return Err(ErrorCode::InvalidMarginCalculation);
        }

        if self.total_spot_asset_value < 0 {
            return Ok(0);
        }

        let net_asset_value = self
            .total_spot_asset_value
            .unsigned_abs()
            .saturating_sub(self.total_spot_liability_value);

        if net_asset_value == 0 {
            return Ok(0);
        }

        // spot liability value + perp liability value / (spot asset value - spot liability value)
        net_asset_value.safe_mul(PRICE_PRECISION)?.safe_div(
            self.total_perp_liability_value
                .safe_add(self.total_spot_liability_value)?,
        )
    }
}

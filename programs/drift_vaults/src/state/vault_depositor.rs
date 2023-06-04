use crate::error::ErrorCode;
use crate::Size;
use anchor_lang::prelude::*;

use crate::math_error;
use crate::safe_decrement;
use crate::safe_increment;
use crate::state::vault::Vault;
use crate::validate;
use static_assertions::const_assert_eq;

use crate::events::{VaultDepositorAction, VaultDepositorRecord};

use drift::math::insurance::{if_shares_to_vault_amount, vault_amount_to_if_shares};

use drift::math::casting::Cast;
use drift::math::safe_math::SafeMath;

#[account(zero_copy)]
#[derive(Eq, PartialEq, Debug)]
#[repr(C)]
pub struct VaultDepositor {
    /// The vault deposited into
    pub vault: Pubkey,
    /// The vault depositor account's pubkey. It is a pda of vault and authority
    pub pubkey: Pubkey,
    /// The authority is the address w permission to deposit/withdraw
    pub authority: Pubkey,
    /// share of vault owned by this depoistor. vault_shares / vault.total_shares is depositor's ownership of vault_equity
    vault_shares: u128,
    /// exponent for vault_shares decimal places
    pub vault_shares_base: u32,
    /// requested vault shares for withdraw
    pub last_withdraw_request_shares: u128,
    /// requested value (in vault spot_market_index) of shares for withdraw
    pub last_withdraw_request_value: u64,
    /// request ts of vault withdraw
    pub last_withdraw_request_ts: i64,
    /// creation ts of vault depositor
    pub last_valid_ts: i64,
    /// lifetime net deposits for the vault
    pub cost_basis: i64,
}

impl Size for VaultDepositor {
    const SIZE: usize = 168 + 8;
}

const_assert_eq!(
    VaultDepositor::SIZE,
    std::mem::size_of::<VaultDepositor>() + 8
);

impl VaultDepositor {
    fn validate_base(&self, vault: &Vault) -> Result<()> {
        validate!(
            self.vault_shares_base == vault.shares_base,
            ErrorCode::InvalidVaultRebase,
            "vault depositor bases mismatch. user base: {} vault base {}",
            self.vault_shares_base,
            vault.shares_base
        )?;

        Ok(())
    }

    pub fn checked_vault_shares(&self, vault: &Vault) -> Result<u128> {
        self.validate_base(vault)?;
        Ok(self.vault_shares)
    }

    pub fn unchecked_vault_shares(&self) -> u128 {
        self.vault_shares
    }

    pub fn increase_vault_shares(&mut self, delta: u128, vault: &Vault) -> Result<()> {
        self.validate_base(vault)?;
        safe_increment!(self.vault_shares, delta);
        Ok(())
    }

    pub fn decrease_vault_shares(&mut self, delta: u128, vault: &Vault) -> Result<()> {
        self.validate_base(vault)?;
        safe_decrement!(self.vault_shares, delta);
        Ok(())
    }

    pub fn update_vault_shares(&mut self, new_shares: u128, vault: &Vault) -> Result<()> {
        self.validate_base(vault)?;
        self.vault_shares = new_shares;

        Ok(())
    }

    pub fn apply_rebase(
        self: &mut VaultDepositor,
        vault: &mut Vault,
        vault_equity: u64,
    ) -> Result<()> {
        vault.apply_rebase(vault_equity)?;

        if vault.shares_base != self.vault_shares_base {
            validate!(
                vault.shares_base > self.vault_shares_base,
                ErrorCode::InvalidVaultRebase,
                "Rebase expo out of bounds"
            )?;

            let expo_diff = (vault.shares_base - self.vault_shares_base).cast::<u32>()?;

            let rebase_divisor = 10_u128.pow(expo_diff);

            msg!(
                "rebasing vault depositor: base: {} -> {} ",
                self.vault_shares_base,
                vault.shares_base,
            );

            self.vault_shares_base = vault.shares_base;

            let old_vault_shares = self.unchecked_vault_shares();
            let new_vault_shares = old_vault_shares.safe_div(rebase_divisor)?;

            msg!("rebasing vault depositor: shares -> {} ", new_vault_shares);

            self.update_vault_shares(new_vault_shares, vault)?;

            self.last_withdraw_request_shares =
                self.last_withdraw_request_shares.safe_div(rebase_divisor)?;
        }

        Ok(())
    }

    pub fn calculate_vault_shares_lost(
        self: &VaultDepositor,
        vault: &Vault,
        vault_balance: u64,
    ) -> Result<u128> {
        let n_shares = self.last_withdraw_request_shares;

        let amount = if_shares_to_vault_amount(n_shares, vault.total_shares, vault_balance)?;

        let vault_shares_lost = if amount > self.last_withdraw_request_value {
            let new_n_shares = vault_amount_to_if_shares(
                self.last_withdraw_request_value,
                vault.total_shares - n_shares,
                vault_balance - self.last_withdraw_request_value,
            )?;

            validate!(
                new_n_shares <= n_shares,
                ErrorCode::InvalidVaultSharesDetected,
                "Issue calculating delta if_shares after canceling request {} < {}",
                new_n_shares,
                n_shares
            )?;

            n_shares.safe_sub(new_n_shares)?
        } else {
            0
        };

        Ok(vault_shares_lost)
    }

    pub fn deposit(
        self: &mut VaultDepositor,
        amount: u64,
        vault_equity: u64,
        vault: &mut Vault,
        now: i64,
    ) -> Result<()> {
        validate!(
            !(vault_equity == 0 && vault.total_shares != 0),
            ErrorCode::InvalidVaultForNewDepositors,
            "Vault balance should be non-zero for new depositors to enter"
        )?;

        self.apply_rebase(vault, vault_equity)?;

        let vault_shares_before = self.checked_vault_shares(vault)?;
        let total_vault_shares_before = vault.total_shares;
        let user_vault_shares_before = vault.user_shares;

        let n_shares = vault_amount_to_if_shares(amount, vault.total_shares, vault_equity)?;

        // reset cost basis if no shares
        self.cost_basis = if vault_shares_before == 0 {
            amount.cast()?
        } else {
            self.cost_basis.safe_add(amount.cast()?)?
        };

        self.increase_vault_shares(n_shares, vault)?;

        vault.total_shares = vault.total_shares.safe_add(n_shares)?;

        vault.user_shares = vault.user_shares.safe_add(n_shares)?;

        let vault_shares_after = self.checked_vault_shares(vault)?;
        emit!(VaultDepositorRecord {
            ts: now,
            vault: vault.pubkey,
            user_authority: self.authority,
            action: VaultDepositorAction::Deposit,
            amount,
            spot_market_index: vault.spot_market_index,
            vault_amount_before: vault_equity,
            vault_shares_before,
            user_vault_shares_before,
            total_vault_shares_before,
            vault_shares_after,
            total_vault_shares_after: vault.total_shares,
            user_vault_shares_after: vault.user_shares,
        });

        Ok(())
    }

    pub fn request_withdraw(
        self: &mut VaultDepositor,
        n_shares: u128,
        vault_equity: u64,
        vault: &mut Vault,
        now: i64,
    ) -> Result<u64> {
        validate!(
            n_shares > 0,
            ErrorCode::InvalidVaultWithdrawSize,
            "Requested n_shares = 0"
        )?;
        validate!(
            self.last_withdraw_request_shares == 0,
            ErrorCode::VaultWithdrawRequestInProgress,
            "Vault withdraw request is already in progress"
        )?;

        self.last_withdraw_request_shares = n_shares;
        self.apply_rebase(vault, vault_equity)?;

        let vault_shares_before: u128 = self.checked_vault_shares(vault)?;
        let total_vault_shares_before = vault.total_shares;
        let user_vault_shares_before = vault.user_shares;

        validate!(
            self.last_withdraw_request_shares <= self.checked_vault_shares(vault)?,
            ErrorCode::InvalidVaultWithdrawSize,
            "last_withdraw_request_shares exceeds vault_shares {} > {}",
            self.last_withdraw_request_shares,
            self.checked_vault_shares(vault)?
        )?;

        validate!(
            self.vault_shares_base == vault.shares_base,
            ErrorCode::InvalidVaultRebase,
            "vault depositor shares_base != vault shares_base"
        )?;

        self.last_withdraw_request_value = if_shares_to_vault_amount(
            self.last_withdraw_request_shares,
            vault.total_shares,
            vault_equity,
        )?
        .min(vault_equity.saturating_sub(1));

        validate!(
            self.last_withdraw_request_value == 0
                || self.last_withdraw_request_value < vault_equity,
            ErrorCode::InvalidVaultWithdrawSize,
            "Requested withdraw value is not below Insurance Fund balance"
        )?;

        let vault_shares_after = self.checked_vault_shares(vault)?;

        emit!(VaultDepositorRecord {
            ts: now,
            vault: vault.pubkey,
            user_authority: self.authority,
            action: VaultDepositorAction::WithdrawRequest,
            amount: self.last_withdraw_request_value,
            spot_market_index: vault.spot_market_index,
            vault_amount_before: vault_equity,
            vault_shares_before,
            user_vault_shares_before,
            total_vault_shares_before,
            vault_shares_after,
            total_vault_shares_after: vault.total_shares,
            user_vault_shares_after: vault.user_shares,
        });

        self.last_withdraw_request_ts = now;

        Ok(self.last_withdraw_request_value)
    }

    pub fn cancel_withdraw_request(
        self: &mut VaultDepositor,
        vault_equity: u64,
        vault: &mut Vault,
        now: i64,
    ) -> Result<()> {
        self.apply_rebase(vault, vault_equity)?;

        let vault_shares_before: u128 = self.checked_vault_shares(vault)?;
        let total_vault_shares_before = vault.total_shares;
        let user_vault_shares_before = vault.user_shares;

        validate!(
            self.vault_shares_base == vault.shares_base,
            ErrorCode::InvalidVaultRebase,
            "vault depositor shares_base != vault shares_base"
        )?;

        let vault_shares_lost = self.calculate_vault_shares_lost(vault, vault_equity)?;
        self.decrease_vault_shares(vault_shares_lost, vault)?;

        vault.total_shares = vault.total_shares.safe_sub(vault_shares_lost)?;

        vault.user_shares = vault.user_shares.safe_sub(vault_shares_lost)?;

        let vault_shares_after = self.checked_vault_shares(vault)?;

        emit!(VaultDepositorRecord {
            ts: now,
            vault: vault.pubkey,
            user_authority: self.authority,
            action: VaultDepositorAction::CancelWithdrawRequest,
            amount: 0,
            spot_market_index: vault.spot_market_index,
            vault_amount_before: vault_equity,
            vault_shares_before,
            user_vault_shares_before,
            total_vault_shares_before,
            vault_shares_after,
            total_vault_shares_after: vault.total_shares,
            user_vault_shares_after: vault.user_shares,
        });

        Ok(())
    }

    pub fn withdraw(
        self: &mut VaultDepositor,
        vault_equity: u64,
        user_authority: Pubkey,
        vault: &mut Vault,
        now: i64,
    ) -> Result<u64> {
        let time_since_withdraw_request = now.safe_sub(self.last_withdraw_request_ts)?;

        validate!(
            time_since_withdraw_request >= vault.redeem_period,
            ErrorCode::CannotWithdrawBeforeRedeemPeriodEnd
        )?;

        self.apply_rebase(vault, vault_equity)?;

        let vault_shares_before: u128 = self.checked_vault_shares(vault)?;
        let total_vault_shares_before = vault.total_shares;
        let user_vault_shares_before = vault.user_shares;

        let n_shares = self.last_withdraw_request_shares;

        validate!(
            n_shares > 0,
            ErrorCode::InvalidVaultWithdraw,
            "Must submit withdraw request and wait the redeem_period ({} seconds)",
            vault.redeem_period
        )?;

        validate!(
            vault_shares_before >= n_shares,
            ErrorCode::InsufficientVaultShares
        )?;

        let amount = if_shares_to_vault_amount(n_shares, vault.total_shares, vault_equity)?;

        let _vault_shares_lost = self.calculate_vault_shares_lost(vault, vault_equity)?;

        let withdraw_amount = amount.min(self.last_withdraw_request_value);

        self.decrease_vault_shares(n_shares, vault)?;

        self.cost_basis = self.cost_basis.safe_sub(withdraw_amount.cast()?)?;

        vault.total_shares = vault.total_shares.safe_sub(n_shares)?;

        vault.user_shares = vault.user_shares.safe_sub(n_shares)?;

        // reset vault_depositor withdraw request info
        self.last_withdraw_request_shares = 0;
        self.last_withdraw_request_value = 0;
        self.last_withdraw_request_ts = now;

        let vault_shares_after = self.checked_vault_shares(vault)?;

        emit!(VaultDepositorRecord {
            ts: now,
            vault: vault.pubkey,
            user_authority,
            action: VaultDepositorAction::Withdraw,
            amount: withdraw_amount,
            spot_market_index: vault.spot_market_index,
            vault_amount_before: vault_equity,
            vault_shares_before,
            user_vault_shares_before,
            total_vault_shares_before,
            vault_shares_after,
            total_vault_shares_after: vault.total_shares,
            user_vault_shares_after: vault.user_shares,
        });

        Ok(withdraw_amount)
    }
}

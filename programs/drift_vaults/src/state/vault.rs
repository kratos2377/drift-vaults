use crate::Size;
use anchor_lang::prelude::*;
use drift::math::casting::Cast;
use drift::math::insurance::calculate_rebase_info;
use drift::math::safe_math::SafeMath;
use static_assertions::const_assert_eq;

#[account(zero_copy)]
#[derive(Eq, PartialEq, Debug)]
#[repr(C)]
pub struct Vault {
    /// The name of the vault. Vault pubkey is derived from this name.
    pub name: [u8; 32],
    /// The vault's pubkey. It is a pda of name and also used as the authority for drift user
    pub pubkey: Pubkey,
    /// The authority of the vault who has ability to update vault params
    pub authority: Pubkey,
    /// The vaults token account. Used to receive tokens between deposits and withdrawals
    pub token_account: Pubkey,
    /// The drift user stats account for the vault
    pub user_stats: Pubkey,
    /// The drift user account for the vault
    pub user: Pubkey,
    /// The spot market index the vault deposits into/withdraws from
    pub spot_market_index: u16,
    /// The bump for the vault pda
    pub bump: u8,
    pub padding: [u8; 1],

    pub redeem_period: i64,
    pub shares_base: u128,
    pub user_shares: u128,
    pub total_shares: u128,
}

impl Vault {
    pub fn get_vault_signer_seeds<'a>(name: &'a [u8], bump: &'a u8) -> [&'a [u8]; 3] {
        [b"vault".as_ref(), name, bytemuck::bytes_of(bump)]
    }
}

impl Size for Vault {
    const SIZE: usize = 228;
}

// const_assert_eq!(Vault::SIZE, std::mem::size_of::<Vault>() + 8);

impl Vault {
    pub fn apply_rebase(&mut self, vault_balance: u64) -> Result<()> {
        if vault_balance != 0 && vault_balance.cast::<u128>()? < self.total_shares {
            let (expo_diff, rebase_divisor) =
                calculate_rebase_info(self.total_shares, vault_balance)?;

            self.total_shares = self.total_shares.safe_div(rebase_divisor)?;
            self.user_shares = self.user_shares.safe_div(rebase_divisor)?;
            self.shares_base = self.shares_base.safe_add(expo_diff.cast::<u128>()?)?;

            msg!("rebasing vault: expo_diff={}", expo_diff);
        }

        if vault_balance != 0 && self.total_shares == 0 {
            self.total_shares = vault_balance.cast::<u128>()?;
        }

        Ok(())
    }
}

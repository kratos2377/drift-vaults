use crate::constraints::is_user_for_vault;
use crate::cpi::UpdateUserCPI;
use crate::error::ErrorCode;
use crate::{declare_vault_seeds, implement_update_user_delegate_cpi};
use crate::{validate, Vault};
use anchor_lang::prelude::*;
use drift::cpi::accounts::UpdateUser;
use drift::program::Drift;
use drift::state::user::User;

pub fn reset_delegate<'info>(ctx: Context<'_, '_, '_, 'info, ResetDelegate<'info>>) -> Result<()> {
    let mut vault = ctx.accounts.vault.load_mut()?;

    validate!(
        vault.in_liquidation(),
        ErrorCode::Default,
        "vault not in liquidation"
    )?;

    let now = Clock::get()?.unix_timestamp;
    vault.check_can_exit_liquidation(now)?;
    vault.reset_liquidation_delegate();

    let delegate = vault.delegate;

    drop(vault);

    ctx.drift_update_user_delegate(delegate)?;

    Ok(())
}

#[derive(Accounts)]
pub struct ResetDelegate<'info> {
    #[account(mut)]
    pub vault: AccountLoader<'info, Vault>,
    pub authority: Signer<'info>,
    #[account(
        mut,
        constraint = is_user_for_vault(&vault, &drift_user.key())?
    )]
    /// CHECK: checked in drift cpi
    pub drift_user: AccountLoader<'info, User>,
    pub drift_program: Program<'info, Drift>,
}

impl<'info> UpdateUserCPI for Context<'_, '_, '_, 'info, ResetDelegate<'info>> {
    fn drift_update_user_delegate(&self, delegate: Pubkey) -> Result<()> {
        implement_update_user_delegate_cpi!(self, delegate);
        Ok(())
    }
}
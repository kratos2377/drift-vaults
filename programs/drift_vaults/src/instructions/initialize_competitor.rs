use crate::constraints::{is_manager_for_vault, is_user_stats_for_vault};
use crate::cpi::InitializeCompetitorCPI;
use crate::declare_vault_seeds;
use crate::Vault;
use anchor_lang::prelude::*;
use drift::state::user::UserStats;
use drift_competitions::cpi::accounts::InitializeCompetitor as DriftCompetitionInitializeCompetitor;
use drift_competitions::program::DriftCompetitions;
// use drift_competitions::state::{Competition, Competitor};
use drift_competitions::state::{Competition, Competitor};

pub fn initialize_competitor<'info>(
    ctx: Context<'_, '_, '_, 'info, InitializeCompetitor<'info>>,
) -> Result<()> {
    ctx.drift_competition_initialize_competitor()?;
    Ok(())
}

#[derive(Accounts)]
#[instruction(market_index: u16)]
pub struct InitializeCompetitor<'info> {
    #[account(
        mut,
        constraint = is_manager_for_vault(&vault, &manager)?,
    )]
    pub vault: AccountLoader<'info, Vault>,
    pub manager: Signer<'info>,
    #[account(mut)]
    pub payer: Signer<'info>,
    pub rent: Sysvar<'info, Rent>,
    pub system_program: Program<'info, System>,

    #[account(
        mut,
        seeds = [b"competitor",  competition.key().as_ref(), vault.key().as_ref()],
        bump
    )]
    pub competitor: AccountLoader<'info, Competitor>,
    #[account(mut)]
    pub competition: AccountLoader<'info, Competition>,
    #[account(
        mut,
        constraint = is_user_stats_for_vault(&vault, &drift_user_stats.to_account_info())?
    )]
    /// CHECK: checked in drift cpi
    pub drift_user_stats: AccountLoader<'info, UserStats>,
    pub drift_competitions_program: Program<'info, DriftCompetitions>,
}

impl<'info> InitializeCompetitorCPI for Context<'_, '_, '_, 'info, InitializeCompetitor<'info>> {
    fn drift_competition_initialize_competitor(&self) -> Result<()> {
        declare_vault_seeds!(self.accounts.vault, seeds);

        let cpi_accounts = DriftCompetitionInitializeCompetitor {
            competitor: self.accounts.competitor.to_account_info().clone(),
            competition: self.accounts.competition.to_account_info().clone(),
            drift_user_stats: self.accounts.drift_user_stats.to_account_info().clone(),
            authority: self.accounts.vault.to_account_info().clone(),
            payer: self.accounts.payer.to_account_info().clone(),
            rent: self.accounts.rent.to_account_info().clone(),
            system_program: self.accounts.system_program.to_account_info().clone(),
        };

        let drift_competitions_program = self
            .accounts
            .drift_competitions_program
            .to_account_info()
            .clone();
        let cpi_context =
            CpiContext::new_with_signer(drift_competitions_program, cpi_accounts, seeds)
                .with_remaining_accounts(self.remaining_accounts.into());
        drift_competitions::cpi::initialize_competitor(cpi_context)?;

        Ok(())
    }
}

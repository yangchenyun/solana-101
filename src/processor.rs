use solana_program::account_info::next_account_info;
use solana_program::program::invoke_signed;
use solana_program::program_error::ProgramError;
use solana_program::{
    account_info::AccountInfo,
    entrypoint::ProgramResult,
    msg,
    program::invoke,
    program_pack::{IsInitialized, Pack},
    pubkey::Pubkey,
    sysvar::{rent::Rent, Sysvar},
};
use spl_token::state::Account;

use crate::{error::EscrowError, instruction::EscrowInstruction, state::Escrow};

pub struct Processor;

impl Processor {
    pub fn process(
        program_id: &Pubkey,
        accounts: &[AccountInfo],
        instruction_data: &[u8],
    ) -> ProgramResult {
        let instruction = EscrowInstruction::unpack(instruction_data)?;

        // use instruction to dispatch procedure
        match instruction {
            EscrowInstruction::InitEscrow { amount } => {
                msg!("Instruction: InitEscrow");
                Self::process_init_escrow(accounts, amount, program_id)
            }
            EscrowInstruction::Exchange { amount } => {
                msg!("Instruction: Exchange");
                Self::process_exchange(accounts, amount, program_id)
            }
            EscrowInstruction::CancelEscrow { amount: _ } => {
                msg!("Instruction: Cancel");
                Self::process_cancel(accounts, program_id)
            }
        }
    }

    fn process_cancel(accounts: &[AccountInfo], program_id: &Pubkey) -> ProgramResult {
        let acc_iter = &mut accounts.iter();

        let owner = next_account_info(acc_iter)?;

        if !owner.is_signer {
            return Err(ProgramError::MissingRequiredSignature);
        }

        let owner_token_to_receive_acc = next_account_info(acc_iter)?;
        let owner_token_to_receive_acc_info =
            Account::unpack(&owner_token_to_receive_acc.try_borrow_data()?)?;

        let escrow_temp_token_acc = next_account_info(acc_iter)?;
        let escrow_temp_token_acc_info =
            Account::unpack(&escrow_temp_token_acc.try_borrow_data()?)?;
        let (pda, bump_seed) = Pubkey::find_program_address(&[b"escrow"], program_id);

        let escrow_acc = next_account_info(acc_iter)?;
        let escrow_acc_info = Escrow::unpack(&escrow_acc.try_borrow_data()?)?;

        let token_program = next_account_info(acc_iter)?;
        let pda_acc = next_account_info(acc_iter)?;

        if owner_token_to_receive_acc_info.mint != escrow_temp_token_acc_info.mint {
            return Err(EscrowError::ExpectedMintMismatch.into());
        }

        let tx_to_owner_ix = spl_token::instruction::transfer(
            token_program.key,
            escrow_temp_token_acc.key,
            owner_token_to_receive_acc.key,
            &pda,
            &[&pda],
            escrow_temp_token_acc_info.amount,
        )?;

        msg!("Calling the token program to return tokens to the escrow's owner.");
        invoke_signed(
            &tx_to_owner_ix,
            &[
                escrow_temp_token_acc.clone(),
                owner_token_to_receive_acc.clone(),
                pda_acc.clone(),
                token_program.clone(),
            ],
            &[&[&b"escrow"[..], &[bump_seed]]],
        )?;

        let close_temp_ix = spl_token::instruction::close_account(
            token_program.key,
            escrow_temp_token_acc.key,
            owner.key,
            &pda,
            &[&pda],
        )?;

        msg!("Calling the token program close temp.");
        invoke_signed(
            &close_temp_ix,
            &[
                escrow_temp_token_acc.clone(),
                owner.clone(),
                pda_acc.clone(),
                token_program.clone(),
            ],
            &[&[&b"escrow"[..], &[bump_seed]]],
        )?;

        msg!("Closing the escrow account...");
        **owner.lamports.borrow_mut() = owner
            .lamports()
            .checked_add(escrow_acc.lamports())
            .ok_or(EscrowError::AmountOverflow)?;

        **escrow_acc.lamports.borrow_mut() = 0;
        // Setting it to empty fields
        *escrow_acc.try_borrow_mut_data()? = &mut [];

        Ok(())
    }

    fn process_exchange(
        accounts: &[AccountInfo],
        amount_expected: u64,
        program_id: &Pubkey,
    ) -> ProgramResult {
        let acc_iter = &mut accounts.iter();

        let taker = next_account_info(acc_iter)?;

        if !taker.is_signer {
            return Err(ProgramError::MissingRequiredSignature);
        }

        let taker_token_sent_acc = next_account_info(acc_iter)?;
        let taker_token_sent_acc_info = Account::unpack(&taker_token_sent_acc.try_borrow_data()?)?;

        let taker_token_to_receive_acc = next_account_info(acc_iter)?;
        let taker_token_to_receive_acc_info =
            Account::unpack(&taker_token_to_receive_acc.try_borrow_data()?)?;

        let escrow_temp_token_acc = next_account_info(acc_iter)?;
        let escrow_temp_token_acc_info =
            Account::unpack(&escrow_temp_token_acc.try_borrow_data()?)?;
        let (pda, bump_seed) = Pubkey::find_program_address(&[b"escrow"], program_id);

        let escrow_maker_acc = next_account_info(acc_iter)?;

        let escrow_maker_to_receive_acc = next_account_info(acc_iter)?;
        let escrow_maker_to_receive_acc_info =
            Account::unpack(&escrow_maker_to_receive_acc.try_borrow_data()?)?;

        let escrow_acc = next_account_info(acc_iter)?;
        let escrow_acc_info = Escrow::unpack(&escrow_acc.try_borrow_data()?)?;

        let token_program = next_account_info(acc_iter)?;
        let pda_acc = next_account_info(acc_iter)?;

        if taker_token_sent_acc_info.mint != escrow_maker_to_receive_acc_info.mint {
            return Err(EscrowError::ExpectedMintMismatch.into());
        }
        if taker_token_to_receive_acc_info.mint != escrow_temp_token_acc_info.mint {
            return Err(EscrowError::ExpectedMintMismatch.into());
        }

        // Now the exchange tokens are matched

        if amount_expected != escrow_temp_token_acc_info.amount {
            return Err(EscrowError::ExpectedAmountMismatch.into());
        }

        if taker_token_sent_acc_info.amount <= escrow_acc_info.expected_amount {
            return Err(EscrowError::NotEnoughBalanceToSent.into());
        }

        // Why check the data here because you couldn't trust the data sent by client?
        // Then why not
        // - Read onchain data here
        // - Use a hash

        if escrow_acc_info.temp_token_account_pubkey != *escrow_temp_token_acc.key {
            return Err(EscrowError::InvalidAccountData.into());
        }

        if escrow_acc_info.initializer_pubkey != *escrow_maker_acc.key {
            return Err(EscrowError::InvalidAccountData.into());
        }

        if escrow_acc_info.initializer_token_to_receive_account_pubkey
            != *escrow_maker_to_receive_acc.key
        {
            return Err(EscrowError::InvalidAccountData.into());
        }

        let tx_to_taker_ix = spl_token::instruction::transfer(
            token_program.key,
            escrow_temp_token_acc.key,
            taker_token_to_receive_acc.key,
            &pda,
            &[&pda],
            amount_expected,
        )?;

        msg!("Calling the token program to transfer tokens to the escrow's taker.");
        invoke_signed(
            &tx_to_taker_ix,
            &[
                escrow_temp_token_acc.clone(),
                taker_token_to_receive_acc.clone(),
                pda_acc.clone(),
                token_program.clone(),
            ],
            &[&[&b"escrow"[..], &[bump_seed]]],
        )?;

        let tx_to_maker_ix = spl_token::instruction::transfer(
            token_program.key,
            taker_token_sent_acc.key,
            escrow_maker_to_receive_acc.key,
            taker.key,
            &[&taker.key],
            escrow_acc_info.expected_amount,
        )?;

        msg!("Calling the token program to transfer tokens to the escrow's maker.");
        invoke(
            &tx_to_maker_ix,
            &[
                taker_token_sent_acc.clone(),
                escrow_maker_to_receive_acc.clone(),
                taker.clone(),
                token_program.clone(),
            ],
        )?;

        let close_temp_ix = spl_token::instruction::close_account(
            token_program.key,
            escrow_temp_token_acc.key,
            escrow_maker_acc.key,
            &pda,
            &[&pda],
        )?;

        msg!("Calling the token program close temp.");
        invoke_signed(
            &close_temp_ix,
            &[
                escrow_temp_token_acc.clone(),
                escrow_maker_acc.clone(),
                pda_acc.clone(),
                token_program.clone(),
            ],
            &[&[&b"escrow"[..], &[bump_seed]]],
        )?;

        msg!("Closing the escrow account...");
        **escrow_maker_acc.lamports.borrow_mut() = escrow_maker_acc
            .lamports()
            .checked_add(escrow_acc.lamports())
            .ok_or(EscrowError::AmountOverflow)?;

        **escrow_acc.lamports.borrow_mut() = 0;
        // Setting it to empty fields
        *escrow_acc.try_borrow_mut_data()? = &mut [];

        Ok(())
    }

    fn process_init_escrow(
        accounts: &[AccountInfo],
        amount: u64,
        program_id: &Pubkey,
    ) -> ProgramResult {
        let account_info_iter = &mut accounts.iter();
        let initializer = next_account_info(account_info_iter)?;

        if !initializer.is_signer {
            return Err(ProgramError::MissingRequiredSignature);
        }

        // TODO: how and should I check this is writable
        let temp_token_account = next_account_info(account_info_iter)?;

        let token_to_receive_account = next_account_info(account_info_iter)?;
        if *token_to_receive_account.owner != spl_token::id() {
            return Err(ProgramError::IncorrectProgramId);
        }

        let escrow_account = next_account_info(account_info_iter)?;

        let sysvar_rent = next_account_info(account_info_iter)?;
        let rent = &Rent::from_account_info(sysvar_rent)?;

        if !rent.is_exempt(escrow_account.lamports(), escrow_account.data_len()) {
            return Err(ProgramError::AccountNotRentExempt);
        }

        let mut escrow_info = Escrow::unpack_unchecked(&escrow_account.try_borrow_data()?)?;
        if escrow_info.is_initialized() {
            return Err(ProgramError::AccountAlreadyInitialized);
        }

        escrow_info.is_initialized = true;
        escrow_info.initializer_pubkey = *initializer.key;
        escrow_info.temp_token_account_pubkey = *temp_token_account.key;
        escrow_info.initializer_token_to_receive_account_pubkey = *token_to_receive_account.key;
        escrow_info.expected_amount = amount;

        Escrow::pack(escrow_info, &mut escrow_account.try_borrow_mut_data()?)?;

        let (pda, _bump_seed) = Pubkey::find_program_address(&[b"escrow"], program_id);

        let token_program = next_account_info(account_info_iter)?;
        // spl instruction to change authority
        let owner_change_ix = spl_token::instruction::set_authority(
            token_program.key,
            temp_token_account.key,
            Some(&pda),
            spl_token::instruction::AuthorityType::AccountOwner,
            initializer.key,
            &[initializer.key], // signer, support multi-sig
        )?;

        msg!("Calling the token program to transfer token account ownership...");
        // CPI interface
        // i.e. the signature is extended to the CPIs.
        invoke(
            &owner_change_ix,
            &[
                temp_token_account.clone(),
                initializer.clone(),
                token_program.clone(),
            ],
        )
    }
}

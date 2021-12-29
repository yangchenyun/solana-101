use thiserror::Error;

use solana_program::program_error::ProgramError;

#[derive(Error, Debug, Copy, Clone)]
pub enum EscrowError {
    #[error("Invalid Instruction")]
    InvalidInstruction,

    #[error("mint mismatch")]
    ExpectedMintMismatch,

    #[error("amount mismatch")]
    ExpectedAmountMismatch,

    #[error("not enough balance")]
    NotEnoughBalanceToSent,

    #[error("Invalid Account Data")]
    InvalidAccountData,

    #[error("Amount Overflow")]
    AmountOverflow,
}

impl From<EscrowError> for ProgramError {
    fn from(e: EscrowError) -> Self {
        ProgramError::Custom(e as u32)
    }
}

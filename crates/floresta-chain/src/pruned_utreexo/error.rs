// SPDX-License-Identifier: MIT OR Apache-2.0

//! This module defines error types specific to the blockchain validation and database operations, along with conversion between types.
//!
//! The main error types are:
//! - [BlockchainError]: High-level error type that encapsulates all the error kinds from our node chain backend operation.
//! - [TransactionError]: Represents errors in transaction validation
//! - [BlockValidationErrors]: Errors encountered during block validation that are not tied to any specific transaction
//!
//! Each error type implements `Display` and `Debug` for error reporting.

extern crate alloc;

use core::error::Error;
use core::fmt;
use core::fmt::Debug;
use core::fmt::Display;
use core::fmt::Formatter;

use bitcoin::OutPoint;
use bitcoin::Txid;
use floresta_common::impl_error_from;
use floresta_common::prelude::*;
use rustreexo::stump::StumpError;

use crate::extensions::ChainWorkOverflow;
use crate::proof_util::UtreexoLeafError;
use crate::pruned_utreexo::chain_state_builder::BlockchainBuilderError;

pub trait DatabaseError: Debug + Display + Send + Sync + 'static {}

#[derive(Debug)]
/// Errors that can happen whilst interacting with the local blockchain.
///
/// It's the highest level error type in [`floresta_chain`](crate),
/// and is returned by [`ChainState`](crate::ChainState) methods.
pub enum BlockchainError {
    /// The block is not present in the [`ChainState`](crate::ChainState).
    BlockNotPresent,

    /// The block is an orphan or is invalid.
    OrphanOrInvalidBlock,

    /// The block failed validation.
    BlockValidation(BlockValidationErrors),

    /// The block contains invalid transaction(s).
    TransactionError(TransactionError),

    /// The Utreexo proof for this block is invalid.
    InvalidUtreexoProof,

    /// Error whilst interacting with the [accumulator](rustreexo::stump::Stump).
    AccumulatorError(StumpError),

    /// Failed to reconstruct a scriptpubkey from a [leaf](crate::pruned_utreexo::udata::CompactLeafData).
    UtreexoLeaf(UtreexoLeafError),

    /// Error whilst interacting with the the [`ChainStore`](crate::ChainStore).
    Database(Box<dyn DatabaseError>),

    /// The [`ChainState`](crate::ChainState) is not initialized.
    ChainNotInitialized,

    /// The [`ChainState`](crate::ChainState)'s tip is invalid.
    InvalidTip(String),

    /// The [`ChainState`](crate::ChainState)'s validation index is invalid.
    BadValidationIndex,

    /// A chainwork calculation overflowed the 256-bit `Work` type.
    OperationOverflow(ChainWorkOverflow),

    /// The requested operation is not supported by this backend
    ///
    /// some [`ChainState`](crate::ChainState) implementations are pruned and
    /// do not hold full blocks or transactions; callers should handle this
    /// variant gracefully
    Unsupported(&'static str),
}

impl_error_from!(BlockchainError, ChainWorkOverflow, OperationOverflow);
impl Display for BlockchainError {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::BlockNotPresent => write!(f, "The block is not present in the ChainState"),
            Self::OrphanOrInvalidBlock => write!(f, "The block was orphaned or is invalid"),
            Self::BlockValidation(e) => write!(f, "Failed to validate the block: {e}"),
            Self::TransactionError(e) => {
                write!(f, "The block contains invalid transaction(s): {e}")
            }
            Self::InvalidUtreexoProof => write!(f, "The Utreexo proof for this block is invalid"),
            Self::AccumulatorError(e) => {
                write!(f, "Error whilst interacting with the accumulator: {e:?}")
            }
            Self::UtreexoLeaf(e) => write!(
                f,
                "Failed to reconstruct a scriptpubkey from Compact Leaf Data: {e}"
            ),
            Self::Database(e) => {
                write!(f, "Error whilst interacting with the the ChainState: {e}")
            }
            Self::ChainNotInitialized => write!(f, "The ChainState is not initialized"),
            Self::InvalidTip(e) => write!(f, "The ChainState's tip is invalid: {e}"),
            Self::BadValidationIndex => write!(f, "The ChainState's validation index is invalid"),
            Self::OperationOverflow(_) => write!(f, "A ChainState operation overflowed"),
            Self::Unsupported(op) => write!(f, "Operation not supported: {op}"),
        }
    }
}

impl Error for BlockchainError {
    /// Exposes the wrapped failure so a caller can walk back to the original cause.
    ///
    /// `Database` holds a [`DatabaseError`], which is deliberately only `Debug + Display`
    /// rather than [`Error`], so it cannot be surfaced here; its message is already carried
    /// by [`Display`]. `AccumulatorError` wraps rustreexo's `StumpError`, which likewise
    /// does not implement [`Error`]. The remaining variants carry no inner error.
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::BlockValidation(e) => Some(e),
            Self::TransactionError(e) => Some(e),
            Self::UtreexoLeaf(e) => Some(e),
            Self::OperationOverflow(e) => Some(e),
            Self::BlockNotPresent
            | Self::OrphanOrInvalidBlock
            | Self::InvalidUtreexoProof
            | Self::AccumulatorError(_)
            | Self::Database(_)
            | Self::ChainNotInitialized
            | Self::InvalidTip(_)
            | Self::BadValidationIndex
            | Self::Unsupported(_) => None,
        }
    }
}

impl<T: DatabaseError> From<T> for BlockchainError {
    fn from(value: T) -> Self {
        Self::Database(Box::new(value))
    }
}

#[derive(Clone, Debug, PartialEq)]
/// Represents errors encountered during transaction validation.
pub struct TransactionError {
    /// The id of the transaction that caused this error
    pub txid: Txid,

    /// The error we've encountered
    pub error: BlockValidationErrors,
}

#[derive(Clone, Debug, PartialEq)]
/// Represents errors encountered during block validation.
pub enum BlockValidationErrors {
    BlockDoesntExtendTip,
    InvalidCoinbase(String),
    UtxoNotFound(OutPoint),
    ScriptValidationError(String),
    NullPrevOut,
    EmptyInputs,
    EmptyOutputs,
    ScriptError,
    BlockTooBig,
    TooManyCoins,
    NotEnoughPow,
    BadMerkleRoot,
    BadWitnessCommitment,
    NotEnoughMoney,
    FirstTxIsNotCoinbase,
    BadCoinbaseOutValue,
    EmptyBlock,
    BlockExtendsAnOrphanChain,
    BadBip34,
    InvalidUtreexoProof,
    CoinbaseNotMatured,
    NonFinalTransaction,
    UnspendableUTXO,
    BIP94TimeWarp,
    DuplicateInput,
}

// Helpful macro for generating a TransactionError
macro_rules! tx_err {
    ($txid_fn:expr, $variant:ident, $msg:expr) => {
        TransactionError {
            txid: ($txid_fn)(),
            error: BlockValidationErrors::$variant($msg.into()),
        }
    };
    ($txid_fn:expr, $variant:ident) => {
        TransactionError {
            txid: ($txid_fn)(),
            error: BlockValidationErrors::$variant,
        }
    };
}

impl Display for TransactionError {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        write!(f, "Transaction {} is invalid: {}", self.txid, self.error)
    }
}

impl Error for TransactionError {
    /// The validation failure that made this transaction invalid.
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        Some(&self.error)
    }
}

impl Display for BlockValidationErrors {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::BlockDoesntExtendTip => {
                write!(f, "This block doesn't build directly on the tip")
            }
            Self::ScriptValidationError(e) => {
                write!(f, "{e}")
            }
            Self::UtxoNotFound(outpoint) => {
                write!(f, "Utxo referenced by {outpoint:?} not found")
            }
            Self::NullPrevOut => {
                write!(
                    f,
                    "This transaction has a null PrevOut but it's not coinbase"
                )
            }
            Self::EmptyInputs => {
                write!(f, "This transaction has no inputs")
            }
            Self::EmptyOutputs => {
                write!(f, "This transaction has no outputs")
            }
            Self::BlockTooBig => write!(f, "Block too big"),
            Self::InvalidCoinbase(e) => {
                write!(f, "Invalid coinbase: {e:?}")
            }
            Self::TooManyCoins => write!(f, "Moving more coins that exists"),
            Self::ScriptError => {
                write!(
                    f,
                    "Script does not follow size requirements of 2>= and <=520"
                )
            }
            Self::NotEnoughPow => {
                write!(f, "This block doesn't have enough proof-of-work")
            }
            Self::BadMerkleRoot => write!(f, "Wrong merkle root"),
            Self::BadWitnessCommitment => write!(f, "Wrong witness commitment"),
            Self::NotEnoughMoney => {
                write!(f, "A transaction spends more than it should")
            }
            Self::FirstTxIsNotCoinbase => {
                write!(f, "The first transaction in a block isn't a coinbase")
            }
            Self::BadCoinbaseOutValue => {
                write!(f, "Coinbase claims more bitcoins than it should")
            }
            Self::EmptyBlock => {
                write!(f, "This block is empty (doesn't have a coinbase tx)")
            }
            Self::BlockExtendsAnOrphanChain => {
                write!(f, "This block extends a chain we don't have the ancestors")
            }
            Self::BadBip34 => write!(f, "BIP34 commitment mismatch"),
            Self::InvalidUtreexoProof => write!(f, "Invalid proof"),
            Self::CoinbaseNotMatured => {
                write!(f, "Coinbase not matured yet")
            }
            Self::NonFinalTransaction => {
                write!(f, "Block contains a non-final transaction")
            }
            Self::UnspendableUTXO => {
                write!(
                    f,
                    "Attempts to spend unspendable UTXO that was overwritten by the historical BIP30 violation"
                )
            }
            Self::BIP94TimeWarp => {
                write!(f, "BIP94 time warp detected")
            }
            Self::DuplicateInput => {
                write!(f, "This transaction has duplicate inputs")
            }
        }
    }
}

impl Error for BlockValidationErrors {}

impl<T: DatabaseError> From<T> for BlockchainBuilderError {
    fn from(value: T) -> Self {
        Self::Database(Box::new(value))
    }
}

impl_error_from!(BlockchainError, TransactionError, TransactionError);
impl_error_from!(BlockchainError, BlockValidationErrors, BlockValidation);
impl_error_from!(BlockchainError, StumpError, AccumulatorError);

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::unreachable,
    clippy::unimplemented,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    clippy::wildcard_enum_match_arm,
    reason = "test code: a panic is the assertion failing, which is the intent"
)]
mod tests {
    use bitcoin::Txid;
    use bitcoin::hashes::Hash;

    use super::*;

    /// A validation failure reaches the caller wrapped, and the chain can be walked back to
    /// the specific rule that rejected the block.
    #[test]
    fn preserves_source_chain_through_transaction_error() {
        let inner = TransactionError {
            txid: Txid::all_zeros(),
            error: BlockValidationErrors::NotEnoughMoney,
        };
        let err = BlockchainError::TransactionError(inner);

        let source = err.source().expect("TransactionError must expose a source");
        assert_eq!(
            source.to_string(),
            TransactionError {
                txid: Txid::all_zeros(),
                error: BlockValidationErrors::NotEnoughMoney,
            }
            .to_string()
        );

        // and one level deeper, to the validation rule itself
        let root = source.source().expect("the validation error is the root");
        assert_eq!(
            root.to_string(),
            BlockValidationErrors::NotEnoughMoney.to_string()
        );
    }

    /// Block validation failures are wrapped rather than flattened into a message.
    #[test]
    fn preserves_source_chain_through_block_validation() {
        let err = BlockchainError::BlockValidation(BlockValidationErrors::BadMerkleRoot);

        let source = err.source().expect("BlockValidation must expose a source");
        assert_eq!(
            source.to_string(),
            BlockValidationErrors::BadMerkleRoot.to_string()
        );
    }

    /// Variants that describe a condition themselves have no source to expose.
    #[test]
    fn domain_variants_have_no_source() {
        assert!(BlockchainError::BlockNotPresent.source().is_none());
        assert!(BlockchainError::ChainNotInitialized.source().is_none());
        assert!(BlockchainError::Unsupported("get_tx").source().is_none());
    }
}

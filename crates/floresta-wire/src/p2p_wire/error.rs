// SPDX-License-Identifier: MIT OR Apache-2.0

use core::fmt;
use core::fmt::Display;
use core::fmt::Formatter;
use std::io;

use floresta_chain::BlockchainError;
use floresta_common::impl_error_from;
use floresta_compact_filters::IterableFilterStoreError;
use tokio::sync::mpsc::error::SendError;

use super::peer::PeerError;
use super::transport::TransportError;
use crate::address_man::AddressManError;
use crate::bitcoin_socket_addr::InvalidAddressError;
use crate::node::NodeRequest;
use crate::node::chain_selector_ctx::ChainSelectorError;
use crate::node::peer_man::PeerManError;
use crate::node::running_ctx::RunningCtxError;
use crate::node::user_req::UserReqError;

#[derive(Debug)]
pub enum WireError {
    /// A failure in the address book.
    ///
    /// Kept distinct from [`Io`](WireError::Io) so a caller can tell a `peers.json`
    /// problem from any other I/O failure in this crate.
    AddressMan(AddressManError),

    /// A failure while managing the set of connected peers.
    PeerMan(PeerManError),

    /// A failure while choosing which chain to follow.
    ChainSelector(ChainSelectorError),

    /// A failure while serving a user request, such as the caller giving up.
    UserReq(UserReqError),

    /// A failure while driving a running node, such as unusable backfill state.
    RunningCtx(RunningCtxError),

    /// Blockchain-related error.
    ///
    /// This error kind is returned by our `ChainState`.
    Blockchain(BlockchainError),

    /// Error while writing into a channel
    ChannelSend(SendError<NodeRequest>),

    /// Attempted to connect with a network we can' reach
    UnreachableNetwork,

    /// Peer error
    PeerError(PeerError),

    /// Coinbase isn't mature
    CoinbaseNotMatured,

    /// Peer not found in our current connections
    PeerNotFound,

    /// Our peer is misbehaving
    PeerMisbehaving,

    /// Failed to init Utreexo peers: anchors.json does not exist yet
    AnchorFileNotFound,

    /// Generic io error
    Io(std::io::Error),

    /// JSON (de)serialization error
    Serde(serde_json::Error),

    /// We couldn't find a peer to send a request
    NoPeerToSendRequest,

    /// Peer timed out some request
    PeerTimeout,

    /// Compact block filters storage error
    CompactBlockFiltersError(IterableFilterStoreError),

    /// Poisoned lock
    PoisonedLock,

    /// We couldn't parse the provided address
    InvalidAddress(InvalidAddressError),

    /// Transport error
    Transport(TransportError),

    /// No addresses available to connect to
    NoAddressesAvailable,

    /// We tried to work on a block we don't have. This is a bug!
    BlockNotFound,

    /// We tried to work on a block that we don't have a proof for yet. This is a bug!
    BlockProofNotFound,

    /// Couldn't find the leaf data for a block
    LeafDataNotFound,

    /// We assumed a chain with invalid blocks, something went really wrong
    AssumedChainInvalid,
}

impl Display for WireError {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnreachableNetwork => {
                write!(f, "The provided network is invalid or unreachable")
            }
            Self::AddressMan(err) => write!(f, "Address book error: {err}"),
            Self::RunningCtx(err) => write!(f, "Running node error: {err}"),
            Self::UserReq(err) => write!(f, "User request error: {err}"),
            Self::ChainSelector(err) => write!(f, "Chain selection error: {err}"),
            Self::PeerMan(err) => write!(f, "Peer management error: {err}"),
            Self::Blockchain(err) => write!(f, "Blockchain error: {err:?}"),
            Self::ChannelSend(err) => write!(f, "Error while writing into channel: {err:?}"),
            Self::PeerError(err) => write!(f, "Peer error: {err:?}"),
            Self::CoinbaseNotMatured => write!(f, "Coinbase isn't mature yet"),
            Self::PeerNotFound => write!(f, "Peer not found in our current connections list"),
            Self::PeerMisbehaving => write!(f, "Our peer is misbehaving"),
            Self::AnchorFileNotFound => write!(
                f,
                "Failed to init Utreexo peers: anchors.json does not exist yet"
            ),
            Self::Io(err) => write!(f, "Generic IO error: {err:?}"),
            Self::Serde(err) => write!(f, "Serde error: {err:?}"),
            Self::NoPeerToSendRequest => {
                write!(f, "We couldn't find a peer to send the request")
            }
            Self::PeerTimeout => write!(f, "Peer timed out"),
            Self::CompactBlockFiltersError(err) => {
                write!(f, "Compact block filters error: {err:?}")
            }
            Self::PoisonedLock => write!(f, "Poisoned lock"),
            Self::InvalidAddress(err) => {
                write!(f, "We couldn't parse the provided address due to: {err:?}")
            }
            Self::Transport(err) => write!(f, "Transport error: {err:?}"),
            Self::NoAddressesAvailable => write!(f, "No addresses available to connect to"),
            Self::BlockNotFound => write!(f, "We tried to work on a block we don't have"),
            Self::BlockProofNotFound => write!(
                f,
                "We tried to work on a block that we don't have a proof for yet"
            ),
            Self::LeafDataNotFound => write!(f, "Couldn't find the leaf data for a block"),
            Self::AssumedChainInvalid => write!(
                f,
                "We assumed a chain with invalid blocks, something went really wrong"
            ),
        }
    }
}

impl std::error::Error for WireError {
    /// Exposes the wrapped failure so a caller can walk back to the original cause. The
    /// variants that describe a condition rather than wrap a failure return `None`.
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::AddressMan(e) => Some(e),
            Self::RunningCtx(e) => Some(e),
            Self::UserReq(e) => Some(e),
            Self::ChainSelector(e) => Some(e),
            Self::PeerMan(e) => Some(e),
            Self::Blockchain(e) => Some(e),
            Self::ChannelSend(e) => Some(e),
            Self::PeerError(e) => Some(e),
            Self::Io(e) => Some(e),
            Self::Serde(e) => Some(e),
            Self::CompactBlockFiltersError(e) => Some(e),
            Self::InvalidAddress(e) => Some(e),
            Self::Transport(e) => Some(e),
            Self::UnreachableNetwork
            | Self::CoinbaseNotMatured
            | Self::PeerNotFound
            | Self::PeerMisbehaving
            | Self::AnchorFileNotFound
            | Self::NoPeerToSendRequest
            | Self::PeerTimeout
            | Self::PoisonedLock
            | Self::NoAddressesAvailable
            | Self::BlockNotFound
            | Self::BlockProofNotFound
            | Self::LeafDataNotFound
            | Self::AssumedChainInvalid => None,
        }
    }
}

impl_error_from!(WireError, AddressManError, AddressMan);
impl_error_from!(WireError, RunningCtxError, RunningCtx);
impl_error_from!(WireError, UserReqError, UserReq);
impl_error_from!(WireError, ChainSelectorError, ChainSelector);
impl_error_from!(WireError, PeerManError, PeerMan);
impl_error_from!(WireError, PeerError, PeerError);
impl_error_from!(WireError, BlockchainError, Blockchain);
impl_error_from!(
    WireError,
    IterableFilterStoreError,
    CompactBlockFiltersError
);
impl_error_from!(WireError, InvalidAddressError, InvalidAddress);
impl_error_from!(WireError, SendError<NodeRequest>, ChannelSend);
impl_error_from!(WireError, serde_json::Error, Serde);
impl_error_from!(WireError, io::Error, Io);

impl From<tokio::sync::oneshot::error::RecvError> for WireError {
    /// A closed response channel means the other half went away, which is the same
    /// condition the user-request module reports when a caller stops waiting.
    fn from(_: tokio::sync::oneshot::error::RecvError) -> Self {
        Self::UserReq(UserReqError::CallerGone)
    }
}

impl From<TransportError> for WireError {
    fn from(e: TransportError) -> Self {
        match e {
            TransportError::Io(io) => Self::Io(io),
            other @ (TransportError::Protocol(_)
            | TransportError::SerdeV2(_)
            | TransportError::SerdeV1(_)
            | TransportError::Proxy(_)
            | TransportError::OversizedMessage { .. }
            | TransportError::BadChecksum { .. }
            | TransportError::BadMagicBits { .. }
            | TransportError::InvalidAddress) => Self::Transport(other),
        }
    }
}

#[derive(Debug, Clone)]
pub enum AddrParseError {
    InvalidIpv6,
    InvalidIpv4,
    InvalidHostname,
    InvalidPort,
    Inconclusive,
}

impl Display for AddrParseError {
    fn fmt(&self, f: &mut Formatter) -> fmt::Result {
        match self {
            Self::InvalidIpv6 => write!(f, "Invalid ipv6"),
            Self::InvalidIpv4 => write!(f, "Invalid ipv4"),
            Self::InvalidHostname => write!(f, "Invalid hostname"),
            Self::InvalidPort => write!(f, "Invalid port"),
            Self::Inconclusive => write!(f, "Inconclusive"),
        }
    }
}

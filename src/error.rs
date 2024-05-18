//! Error Types

use std::num::TryFromIntError;

use nix::errno::Errno;
use oathgate_net::ProtocolError;

/// Helper type for application errors
pub type AppResult<T> = std::result::Result<T, Error>;

#[derive(thiserror::Error, Clone, Debug)]
pub enum PayloadError {
    #[error("Payload is missing")]
    Missing,

    #[error("not enough data for payload, got = {0}, expected = {1}")]
    NotEnoughData(usize, usize),

    #[error("ancillary / control data missing")]
    MissingControlData,

    #[error("no file descriptors found in ancillary data")]
    NoFileDescriptorsFound,

    #[error("control data mismatch")]
    ControlDataMismatch,
}

#[derive(thiserror::Error, Debug)]
pub enum MemoryError {
    #[error("no memory has been mapped")]
    NoMappedMemory,

    #[error("no mapping from host to guest address found: host address 0x{0:08x}")]
    NoHostToGuestMappingFound(u64),
}

#[derive(thiserror::Error, Debug)]
pub enum QueueError {
    #[error("queue is disabled")]
    Disabled,

    #[error("memory: {0}")]
    Memory(#[from] MemoryError),

    #[error("virtio: {0}")]
    Virtio(#[from] virtio_queue::Error),

    #[error("i/o: {0}")]
    IO(#[from] std::io::Error),
}

#[derive(thiserror::Error, Debug)]
pub enum UpstreamError {
    #[error("unable to create upstream")]
    CreateFailed(String),
}

#[derive(thiserror::Error, Debug)]
pub enum Error {
    #[error("error: {0}")]
    Errno(#[from] Errno),

    #[error("payload: {0}")]
    Payload(#[from] PayloadError),

    #[error("i/o: {0}")]
    IO(#[from] std::io::Error),

    #[error("failed to cast int: {0}")]
    TryFromInt(#[from] TryFromIntError),

    #[error("vhost header is missing")]
    HeaderMissing,

    #[error("mmap: {0}")]
    Mmap(#[from] vm_memory::mmap::Error),

    #[error("mmap region: {0}")]
    MmapRegion(#[from] vm_memory::mmap::MmapRegionError),

    #[error("memory: {0}")]
    Memory(#[from] MemoryError),

    #[error("invalid message: {0}")]
    InvalidMessage(&'static str),

    #[error("queue not found, index = {0}")]
    QueueNotFound(usize),

    #[error("queue is disabled")]
    QueueDisabled,

    #[error("no receivers available")]
    ChannelClosed,

    #[error("virtio: {0}")]
    Virtio(#[from] virtio_queue::Error),

    #[error("protocol failed: {0}")]
    Protocol(#[from] ProtocolError),

    #[error("upstream: {0}")]
    Upstream(#[from] UpstreamError),
}

impl<T> From<flume::SendError<T>> for Error {
    fn from(_value: flume::SendError<T>) -> Self {
        Self::ChannelClosed
    }
}

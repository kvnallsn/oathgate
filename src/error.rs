//! Error Types

use std::num::TryFromIntError;

use nix::errno::Errno;

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
pub enum MessageError {
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
}

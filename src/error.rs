/// Crate-level error. Collector and publisher stubs use [`Error::NotImplemented`].
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// The command or module exists, but this phase does not implement it yet.
    #[error("not implemented in F0-1: {0}")]
    NotImplemented(&'static str),
    /// Clap already formatted this message. Callers print it without a prefix.
    #[error("{0}")]
    Usage(String),
    /// One or more graph or alias invariants failed.
    #[error("{0}")]
    Validation(String),
    /// Config file or environment mapping is unusable.
    #[error("{0}")]
    Config(String),
    /// Snapshot or meta store failed.
    #[error("{0}")]
    Store(String),
    /// Filesystem failure.
    #[error(transparent)]
    Io(#[from] std::io::Error),
    /// GitLab HTTP or auth failure. The message is redacted.
    #[error("{0}")]
    GitLab(String),
}

impl Error {
    /// Process status for this error. `2` means usage or a stub command.
    #[must_use]
    pub fn exit_code(&self) -> u8 {
        match self {
            Self::NotImplemented(_) | Self::Usage(_) => 2,
            Self::Validation(_)
            | Self::Config(_)
            | Self::Store(_)
            | Self::Io(_)
            | Self::GitLab(_) => 1,
        }
    }
}

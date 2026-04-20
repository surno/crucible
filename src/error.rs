use thiserror::Error;

#[derive(Debug, Error)]
pub enum SandboxError {
    #[error("sandbox configuration is invalid: {0}")]
    InvalidConfig(String),

    #[error("failed to initialize wasm engine")]
    EngineInit(#[source] wasmtime::Error),

    #[error("failed to compile module")]
    Compile(#[source] wasmtime::Error),

    #[error("guest exhausted its fuel budget")]
    OutOfFuel,

    #[error("guest exceeded its {limit_bytes}-byte memory limit")]
    MemoryExceeded { limit_bytes: usize },

    #[error("guest exceeded its execution timeout")]
    Timeout,

    #[error("guest trapped: {0}")]
    Trap(String),

    #[error(transparent)]
    Io(#[from] std::io::Error),
}

pub type Result<T> = std::result::Result<T, SandboxError>;

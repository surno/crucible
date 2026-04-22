use std::time::Duration;

use wasmtime::{Config, Engine};

use super::Sandbox;
use crate::error::{Result, SandboxError};
use crate::units::ByteSize;

pub struct SandboxBuilder {
    memory_limit: ByteSize,
    fuel: Option<u64>,
    timeout: Option<Duration>,
}

impl SandboxBuilder {
    pub fn new(memory_limit: ByteSize) -> Self {
        Self {
            memory_limit,
            fuel: None,
            timeout: None,
        }
    }

    pub fn fuel(mut self, amount: u64) -> Self {
        self.fuel = Some(amount);
        self
    }

    pub fn timeout(mut self, duration: Duration) -> Self {
        self.timeout = Some(duration);
        self
    }

    pub fn build(self) -> Result<Sandbox> {
        let mut config = Config::default();
        if self.fuel.is_some() {
            config.consume_fuel(true);
        }
        if self.timeout.is_some() {
            config.epoch_interruption(true);
        }
        let engine = Engine::new(&config).map_err(SandboxError::EngineInit)?;
        Ok(Sandbox::new(
            engine,
            self.fuel,
            self.memory_limit,
            self.timeout,
        ))
    }
}

impl Default for SandboxBuilder {
    fn default() -> Self {
        Self::new(Sandbox::DEFAULT_MEMORY_LIMIT)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_builder_uses_default_memory_limit() {
        let b = SandboxBuilder::default();
        assert_eq!(b.memory_limit, Sandbox::DEFAULT_MEMORY_LIMIT);
    }

    #[test]
    fn builder_starts_with_no_fuel_or_timeout() {
        let b = Sandbox::builder(ByteSize::mib(1));
        assert_eq!(b.memory_limit, ByteSize::mib(1));
        assert_eq!(b.fuel, None);
        assert_eq!(b.timeout, None);
    }

    #[test]
    fn setters_record_values() {
        let b = Sandbox::builder(Sandbox::DEFAULT_MEMORY_LIMIT)
            .fuel(1_000)
            .timeout(Duration::from_millis(500));
        assert_eq!(b.memory_limit, Sandbox::DEFAULT_MEMORY_LIMIT);
        assert_eq!(b.fuel, Some(1_000));
        assert_eq!(b.timeout, Some(Duration::from_millis(500)));
    }

    #[test]
    fn last_setter_wins() {
        let b = Sandbox::builder(Sandbox::DEFAULT_MEMORY_LIMIT)
            .fuel(100)
            .fuel(200);
        assert_eq!(b.fuel, Some(200));
    }

    #[test]
    fn build_returns_ok_with_no_options() {
        let result = Sandbox::builder(Sandbox::DEFAULT_MEMORY_LIMIT).build();
        assert!(result.is_ok());
    }

    #[test]
    fn build_returns_ok_with_all_limits() {
        let result = Sandbox::builder(ByteSize::mib(2))
            .fuel(10_000)
            .timeout(Duration::from_millis(250))
            .build();
        assert!(result.is_ok());
    }
}

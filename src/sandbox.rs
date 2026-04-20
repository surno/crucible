use std::time::Duration;

use wasmtime::{Config, Engine};

use crate::error::{Result, SandboxError};
use crate::units::ByteSize;

pub struct Sandbox {
    engine: Engine,
    fuel_limit: Option<u64>,
    memory_limit: Option<ByteSize>,
    timeout: Option<Duration>,
}

impl Sandbox {
    pub fn builder() -> SandboxBuilder {
        SandboxBuilder::new()
    }
}

pub struct SandboxBuilder {
    memory_limit: Option<ByteSize>,
    fuel: Option<u64>,
    timeout: Option<Duration>,
}

impl SandboxBuilder {
    pub fn new() -> Self {
        Self {
            memory_limit: None,
            fuel: None,
            timeout: None,
        }
    }

    pub fn memory_limit(mut self, size: ByteSize) -> Self {
        self.memory_limit = Some(size);
        self
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
        Ok(Sandbox {
            engine,
            fuel_limit: self.fuel,
            memory_limit: self.memory_limit,
            timeout: self.timeout,
        })
    }
}

impl Default for SandboxBuilder {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builder_starts_with_no_limits() {
        let b = Sandbox::builder();
        assert_eq!(b.memory_limit, None);
        assert_eq!(b.fuel, None);
        assert_eq!(b.timeout, None);
    }

    #[test]
    fn setters_record_values() {
        let b = Sandbox::builder()
            .memory_limit(ByteSize::kib(64))
            .fuel(1_000)
            .timeout(Duration::from_millis(500));
        assert_eq!(b.memory_limit, Some(ByteSize::kib(64)));
        assert_eq!(b.fuel, Some(1_000));
        assert_eq!(b.timeout, Some(Duration::from_millis(500)));
    }

    #[test]
    fn last_setter_wins() {
        let b = Sandbox::builder().fuel(100).fuel(200);
        assert_eq!(b.fuel, Some(200));
    }

    #[test]
    fn build_with_no_options_succeeds() {
        let sandbox = Sandbox::builder().build().expect("build should succeed");
        assert_eq!(sandbox.fuel_limit, None);
        assert_eq!(sandbox.memory_limit, None);
        assert_eq!(sandbox.timeout, None);
    }

    #[test]
    fn build_propagates_all_limits_to_sandbox() {
        let sandbox = Sandbox::builder()
            .memory_limit(ByteSize::mib(2))
            .fuel(10_000)
            .timeout(Duration::from_millis(250))
            .build()
            .expect("build should succeed");
        assert_eq!(sandbox.fuel_limit, Some(10_000));
        assert_eq!(sandbox.memory_limit, Some(ByteSize::mib(2)));
        assert_eq!(sandbox.timeout, Some(Duration::from_millis(250)));
    }
}

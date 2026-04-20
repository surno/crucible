use std::num::TryFromIntError;
use std::time::Duration;

use wasmtime::{Config, Engine, Instance, Module, Store, StoreLimits, StoreLimitsBuilder, Val};

use crate::error::{Result, SandboxError};
use crate::units::ByteSize;

struct SandboxData {
    limiter: StoreLimits,
}

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

    pub fn run(&self, wasm: &[u8], func_name: &str, args: &[Val]) -> Result<Vec<Val>> {
        let state = SandboxData {
            limiter: StoreLimitsBuilder::new()
                .memory_size(
                    self.memory_limit
                        .unwrap_or(ByteSize::mib(1))
                        .as_bytes()
                        .try_into()
                        .map_err(|x: TryFromIntError| SandboxError::Trap(x.to_string()))?,
                )
                .instances(1)
                .build(),
        };
        let mut store = Store::new(&self.engine, state);

        if let Some(fuel) = self.fuel_limit {
            store.set_fuel(fuel).map_err(SandboxError::EngineInit)?;
        }

        if let Some(_) = self.memory_limit {
            store.limiter(|state| &mut state.limiter);
        }

        let module = Module::from_binary(&self.engine, wasm).map_err(SandboxError::Compile)?;

        let instance = Instance::new(&mut store, &module, &[]).map_err(SandboxError::Compile)?;

        let func = instance
            .get_func(&mut store, func_name)
            .ok_or_else(|| SandboxError::Trap(format!("Export '{func_name}', not found!")))?;

        let result_count = func.ty(&store).results().len();
        let mut result = vec![Val::I32(0); result_count];
        func.call(&mut store, args, &mut result)
            .map_err(|e| SandboxError::Trap(e.to_string()))?;
        Ok(result)
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

    fn wasm(text: &str) -> Vec<u8> {
        wat::parse_str(text).expect("test wat should compile")
    }

    #[test]
    fn run_executes_function_with_no_args_or_results() {
        let sandbox = Sandbox::builder().build().unwrap();
        let bytes = wasm("(module (func (export \"noop\")))");

        let result = sandbox.run(&bytes, "noop", &[]).expect("call should succeed");
        assert!(result.is_empty());
    }

    #[test]
    fn run_returns_value_from_no_arg_function() {
        let sandbox = Sandbox::builder().build().unwrap();
        let bytes = wasm("(module (func (export \"answer\") (result i32) i32.const 42))");

        let result = sandbox.run(&bytes, "answer", &[]).unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].i32(), Some(42));
    }

    #[test]
    fn run_passes_args_and_returns_result() {
        let sandbox = Sandbox::builder().build().unwrap();
        let bytes = wasm(
            "(module (func (export \"add\") (param i32 i32) (result i32)
                local.get 0 local.get 1 i32.add))",
        );

        let result = sandbox
            .run(&bytes, "add", &[Val::I32(7), Val::I32(35)])
            .unwrap();
        assert_eq!(result[0].i32(), Some(42));
    }

    #[test]
    fn run_returns_trap_when_export_missing() {
        let sandbox = Sandbox::builder().build().unwrap();
        let bytes = wasm("(module (func (export \"only\")))");

        let err = sandbox.run(&bytes, "missing", &[]).unwrap_err();
        match err {
            SandboxError::Trap(msg) => assert!(msg.contains("missing")),
            other => panic!("expected Trap, got {other:?}"),
        }
    }

    #[test]
    fn run_returns_compile_error_for_invalid_bytes() {
        let sandbox = Sandbox::builder().build().unwrap();

        let err = sandbox.run(b"definitely not wasm", "anything", &[]).unwrap_err();
        assert!(matches!(err, SandboxError::Compile(_)));
    }

    #[test]
    fn run_traps_when_fuel_is_exhausted() {
        let sandbox = Sandbox::builder().fuel(10).build().unwrap();
        let bytes = wasm(
            "(module (func (export \"spin\") (loop $l br $l)))",
        );

        let err = sandbox.run(&bytes, "spin", &[]).unwrap_err();
        assert!(matches!(err, SandboxError::Trap(_)));
    }

    #[test]
    fn run_traps_when_arg_types_mismatch_signature() {
        let sandbox = Sandbox::builder().build().unwrap();
        let bytes = wasm(
            "(module (func (export \"need_i32\") (param i32)))",
        );

        // Function wants i32, we pass i64 — wasmtime should reject the call.
        let err = sandbox.run(&bytes, "need_i32", &[Val::I64(0)]).unwrap_err();
        assert!(matches!(err, SandboxError::Trap(_)));
    }
}

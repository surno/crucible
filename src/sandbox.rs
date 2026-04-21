use std::num::TryFromIntError;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::{scope, sleep};
use std::time::{Duration, Instant};

use wasmtime::{Config, Engine, Instance, Module, Store, Trap, Val};

use crate::error::{Result, SandboxError};
use crate::limits::CrucibleResourceLimiter;
use crate::units::ByteSize;

struct SandboxData {
    limiter: CrucibleResourceLimiter,
}

pub struct Sandbox {
    engine: Engine,
    fuel_limit: Option<u64>,
    memory_limit: ByteSize,
    timeout: Option<Duration>,
}

impl Sandbox {
    pub const DEFAULT_MEMORY_LIMIT: ByteSize = ByteSize::kib(64);

    pub fn builder(memory_limit: ByteSize) -> SandboxBuilder {
        SandboxBuilder::new(memory_limit)
    }

    pub fn run(&self, wasm: &[u8], func_name: &str, args: &[Val]) -> Result<Vec<Val>> {
        let mut store = self.configure_store()?;
        let module = Module::from_binary(&self.engine, wasm).map_err(SandboxError::Compile)?;

        let instance = Instance::new(&mut store, &module, &[]).map_err(SandboxError::Compile)?;

        let func = instance
            .get_func(&mut store, func_name)
            .ok_or_else(|| SandboxError::Trap(format!("Export '{func_name}', not found!")))?;

        let result_count = func.ty(&store).results().len();
        let mut result = vec![Val::I32(0); result_count];

        // call the thread which will manage the timeout
        let stop = AtomicBool::new(false);
        scope(|s| {
            if let Some(timeout) = self.timeout {
                let backstop = timeout + Duration::from_millis(100);
                let engine_weak = self.engine.weak();
                let stop_ref = &stop;
                s.spawn(move || {
                    let start = Instant::now();
                    while !stop_ref.load(Ordering::Relaxed) {
                        sleep(Duration::from_millis(1));
                        let Some(e) = engine_weak.upgrade() else {
                            break;
                        };
                        e.increment_epoch();

                        // Backstop: if the trap was supposed to fire long ago, but
                        // hasn't. We must bail out since additional ticks won't help
                        if start.elapsed() > backstop {
                            break;
                        }
                    }
                });
            }

            let r = func
                .call(&mut store, args, &mut result)
                .map_err(|e: wasmtime::Error| self.classify_call_error(e, &store));
            // signal the the spawned thread that the engine has ran the function and completed.
            stop.store(true, Ordering::Relaxed);
            r
        })?;
        Ok(result)
    }

    fn configure_store(&self) -> Result<Store<SandboxData>> {
        let state = SandboxData {
            limiter: CrucibleResourceLimiter::new(
                self.memory_limit
                    .as_bytes()
                    .try_into()
                    .map_err(|e: TryFromIntError| {
                        SandboxError::InvalidConfig(format!(
                            "Failed to convert memory limit to host: {e}"
                        ))
                    })?,
            ),
        };
        let mut store = Store::new(&self.engine, state);

        if let Some(fuel) = self.fuel_limit {
            store.set_fuel(fuel).map_err(SandboxError::EngineInit)?;
        }

        if let Some(duration) = self.timeout {
            let ticks = u64::try_from(duration.as_millis()).map_err(|_| {
                SandboxError::InvalidConfig(format!("timeout {duration:?} exceeds u64 ms"))
            })?;
            store.set_epoch_deadline(ticks); // arm this store's trap
        }

        store.limiter(|state| &mut state.limiter);
        Ok(store)
    }

    fn classify_call_error(&self, e: wasmtime::Error, store: &Store<SandboxData>) -> SandboxError {
        let refused = store.data().limiter.refused_memory_growth();
        match e.downcast_ref::<Trap>() {
            Some(Trap::OutOfFuel) => SandboxError::OutOfFuel,
            Some(Trap::Interrupt) => SandboxError::Timeout,
            Some(Trap::MemoryOutOfBounds) if refused => SandboxError::MemoryExceeded {
                limit_bytes: self.memory_limit.as_bytes(),
            },
            _ => SandboxError::Trap(e.to_string()),
        }
    }
}

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
        Self::new(Sandbox::DEFAULT_MEMORY_LIMIT)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_memory_limit_is_one_wasm_page() {
        assert_eq!(Sandbox::DEFAULT_MEMORY_LIMIT.as_bytes(), 64 * 1024);
    }

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
    fn build_with_no_options_succeeds() {
        let sandbox = Sandbox::builder(Sandbox::DEFAULT_MEMORY_LIMIT)
            .build()
            .expect("build should succeed");
        assert_eq!(sandbox.fuel_limit, None);
        assert_eq!(sandbox.memory_limit, Sandbox::DEFAULT_MEMORY_LIMIT);
        assert_eq!(sandbox.timeout, None);
    }

    #[test]
    fn build_propagates_all_limits_to_sandbox() {
        let sandbox = Sandbox::builder(ByteSize::mib(2))
            .fuel(10_000)
            .timeout(Duration::from_millis(250))
            .build()
            .expect("build should succeed");
        assert_eq!(sandbox.fuel_limit, Some(10_000));
        assert_eq!(sandbox.memory_limit, ByteSize::mib(2));
        assert_eq!(sandbox.timeout, Some(Duration::from_millis(250)));
    }

    fn wasm(text: &str) -> Vec<u8> {
        wat::parse_str(text).expect("test wat should compile")
    }

    #[test]
    fn run_executes_function_with_no_args_or_results() {
        let sandbox = Sandbox::builder(Sandbox::DEFAULT_MEMORY_LIMIT)
            .build()
            .unwrap();
        let bytes = wasm("(module (func (export \"noop\")))");

        let result = sandbox
            .run(&bytes, "noop", &[])
            .expect("call should succeed");
        assert!(result.is_empty());
    }

    #[test]
    fn run_returns_value_from_no_arg_function() {
        let sandbox = Sandbox::builder(Sandbox::DEFAULT_MEMORY_LIMIT)
            .build()
            .unwrap();
        let bytes = wasm("(module (func (export \"answer\") (result i32) i32.const 42))");

        let result = sandbox.run(&bytes, "answer", &[]).unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].i32(), Some(42));
    }

    #[test]
    fn run_passes_args_and_returns_result() {
        let sandbox = Sandbox::builder(Sandbox::DEFAULT_MEMORY_LIMIT)
            .build()
            .unwrap();
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
        let sandbox = Sandbox::builder(Sandbox::DEFAULT_MEMORY_LIMIT)
            .build()
            .unwrap();
        let bytes = wasm("(module (func (export \"only\")))");

        let err = sandbox.run(&bytes, "missing", &[]).unwrap_err();
        match err {
            SandboxError::Trap(msg) => assert!(msg.contains("missing")),
            other => panic!("expected Trap, got {other:?}"),
        }
    }

    #[test]
    fn run_returns_compile_error_for_invalid_bytes() {
        let sandbox = Sandbox::builder(Sandbox::DEFAULT_MEMORY_LIMIT)
            .build()
            .unwrap();

        let err = sandbox
            .run(b"definitely not wasm", "anything", &[])
            .unwrap_err();
        assert!(matches!(err, SandboxError::Compile(_)));
    }

    #[test]
    fn run_returns_out_of_fuel_when_fuel_is_exhausted() {
        let sandbox = Sandbox::builder(Sandbox::DEFAULT_MEMORY_LIMIT)
            .fuel(10)
            .build()
            .unwrap();
        let bytes = wasm("(module (func (export \"spin\") (loop $l br $l)))");

        let err = sandbox.run(&bytes, "spin", &[]).unwrap_err();
        assert!(matches!(err, SandboxError::OutOfFuel));
    }

    #[test]
    fn run_traps_when_arg_types_mismatch_signature() {
        let sandbox = Sandbox::builder(Sandbox::DEFAULT_MEMORY_LIMIT)
            .build()
            .unwrap();
        let bytes = wasm("(module (func (export \"need_i32\") (param i32)))");

        // Function wants i32, we pass i64 — wasmtime should reject the call.
        let err = sandbox.run(&bytes, "need_i32", &[Val::I64(0)]).unwrap_err();
        assert!(matches!(err, SandboxError::Trap(_)));
    }

    #[test]
    fn run_allows_memory_growth_within_limit() {
        // Limit = 2 pages. Module starts at 1 page and grows by 1 — exactly at limit.
        let sandbox = Sandbox::builder(ByteSize::kib(128)).build().unwrap();
        let bytes = wasm(
            "(module
                (memory 1)
                (func (export \"grow_one\") (result i32)
                    i32.const 1
                    memory.grow))",
        );

        let result = sandbox
            .run(&bytes, "grow_one", &[])
            .expect("grow within limit succeeds");
        // memory.grow returns the previous size in pages on success (1), or -1 on failure.
        assert_eq!(result[0].i32(), Some(1));
    }

    #[test]
    fn run_returns_memory_exceeded_when_grow_denied_then_access_traps() {
        // Limit = 1 page. Module starts at 1 page, asks to grow by 100 (denied),
        // then writes way past the current size — that write traps.
        // The limiter's refusal flag should override the generic trap classification.
        let sandbox = Sandbox::builder(Sandbox::DEFAULT_MEMORY_LIMIT)
            .build()
            .unwrap();
        let bytes = wasm(
            "(module
                (memory 1)
                (func (export \"grow_then_oob\")
                    i32.const 100
                    memory.grow
                    drop
                    i32.const 1000000
                    i32.const 42
                    i32.store))",
        );

        let err = sandbox.run(&bytes, "grow_then_oob", &[]).unwrap_err();
        match err {
            SandboxError::MemoryExceeded { limit_bytes } => {
                assert_eq!(limit_bytes, Sandbox::DEFAULT_MEMORY_LIMIT.as_bytes());
            }
            other => panic!("expected MemoryExceeded, got {other:?}"),
        }
    }

    #[test]
    fn run_succeeds_when_grow_denied_but_guest_handles_gracefully() {
        // The guest asks for too much memory, sees -1 from memory.grow, and returns it.
        // No trap → no error path → MemoryExceeded must NOT fire even though the
        // limiter recorded a refusal.
        let sandbox = Sandbox::builder(Sandbox::DEFAULT_MEMORY_LIMIT)
            .build()
            .unwrap();
        let bytes = wasm(
            "(module
                (memory 1)
                (func (export \"try_grow\") (result i32)
                    i32.const 100
                    memory.grow))",
        );

        let result = sandbox
            .run(&bytes, "try_grow", &[])
            .expect("call should succeed");
        assert_eq!(result[0].i32(), Some(-1));
    }

    #[test]
    fn run_times_out_on_infinite_loop() {
        let sandbox = Sandbox::builder(Sandbox::DEFAULT_MEMORY_LIMIT)
            .timeout(Duration::from_millis(50))
            .build()
            .unwrap();
        let bytes = wasm("(module (func (export \"spin\") (loop $l br $l)))");

        let err = sandbox.run(&bytes, "spin", &[]).unwrap_err();
        assert!(matches!(err, SandboxError::Timeout));
    }

    #[test]
    fn run_returns_out_of_fuel_when_fuel_exhausted_after_grow_denied() {
        let s = Sandbox::builder(Sandbox::DEFAULT_MEMORY_LIMIT)
            .fuel(100_000)
            .build()
            .unwrap();
        let bytes = wasm(
            "(module (memory 1) (func (export \"f\")
        i32.const 100 memory.grow drop
        (loop $l br $l)))",
        );
        assert!(matches!(
            s.run(&bytes, "f", &[]).unwrap_err(),
            SandboxError::OutOfFuel
        ));
    }

    #[test]
    fn run_returns_timeout_when_loop_runs_after_grow_denied() {
        let s = Sandbox::builder(Sandbox::DEFAULT_MEMORY_LIMIT)
            .timeout(Duration::from_millis(50))
            .build()
            .unwrap();
        let bytes = wasm(
            "(module (memory 1) (func (export \"f\")
        i32.const 100 memory.grow drop
        (loop $l br $l)))",
        );
        assert!(matches!(
            s.run(&bytes, "f", &[]).unwrap_err(),
            SandboxError::Timeout
        ));
    }
}

use std::{collections::HashMap, path::PathBuf};

use crate::{SandboxError, error::Result};

use wasmtime::Val;
use wasmtime_wasi::{DirPerms, FilePerms, WasiCtxBuilder};

use crate::Sandbox;

#[derive(Debug)]
struct Preopen {
    host_path: PathBuf,
    guest_path: String,
    dir_perms: DirPerms,
    file_perms: FilePerms,
}

pub struct Invocation<'a> {
    sandbox: &'a Sandbox,
    wasm: &'a [u8],
    preopens: Vec<Preopen>,
    env: HashMap<String, String>,
}

impl<'a> Invocation<'a> {
    pub(super) fn new(sandbox: &'a Sandbox, wasm: &'a [u8]) -> Self {
        Self {
            sandbox,
            wasm,
            preopens: Vec::new(),
            env: HashMap::new(),
        }
    }

    pub fn allow_read_dir(
        mut self,
        host_path: impl Into<PathBuf>,
        guest_path: impl Into<String>,
    ) -> Self {
        self.preopens.push(Preopen {
            host_path: host_path.into(),
            guest_path: guest_path.into(),
            dir_perms: DirPerms::READ,
            file_perms: FilePerms::READ,
        });
        self
    }

    pub fn env(mut self, key: impl Into<String>, val: impl Into<String>) -> Self {
        self.env.insert(key.into(), val.into());
        self
    }

    pub fn allow_write_dir(
        mut self,
        host_path: impl Into<PathBuf>,
        guest_path: impl Into<String>,
    ) -> Self {
        self.preopens.push(Preopen {
            host_path: host_path.into(),
            guest_path: guest_path.into(),
            dir_perms: DirPerms::READ | DirPerms::MUTATE,
            file_perms: FilePerms::READ | FilePerms::WRITE,
        });
        self
    }

    pub fn invoke(self, func_name: &str, args: &[Val]) -> Result<Vec<Val>> {
        let mut wasi_builder = WasiCtxBuilder::new();
        for preopen in self.preopens {
            wasi_builder
                .preopened_dir(
                    preopen.host_path,
                    preopen.guest_path,
                    preopen.dir_perms,
                    preopen.file_perms,
                )
                .map_err(|e| SandboxError::InvalidConfig(e.to_string()))?;
        }

        for (key, val) in self.env {
            wasi_builder.env(key, val);
        }

        self.sandbox
            .run(self.wasm, func_name, args, wasi_builder.build_p1())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ByteSize;

    fn wasm(text: &str) -> Vec<u8> {
        wat::parse_str(text).expect("test wat should compile")
    }

    fn sandbox() -> Sandbox {
        Sandbox::builder(Sandbox::DEFAULT_MEMORY_LIMIT)
            .build()
            .unwrap()
    }

    #[test]
    fn invoke_runs_simple_function() {
        let s = sandbox();
        let bytes = wasm("(module (func (export \"noop\")))");

        let result = s
            .module(&bytes)
            .invoke("noop", &[])
            .expect("invoke should succeed");
        assert!(result.is_empty());
    }

    #[test]
    fn invoke_returns_result_value() {
        let s = sandbox();
        let bytes = wasm("(module (func (export \"answer\") (result i32) i32.const 42))");

        let result = s.module(&bytes).invoke("answer", &[]).unwrap();
        assert_eq!(result[0].i32(), Some(42));
    }

    #[test]
    fn invoke_passes_args_and_returns_result() {
        let s = sandbox();
        let bytes = wasm(
            "(module (func (export \"add\") (param i32 i32) (result i32)
                local.get 0 local.get 1 i32.add))",
        );

        let result = s
            .module(&bytes)
            .invoke("add", &[Val::I32(7), Val::I32(35)])
            .unwrap();
        assert_eq!(result[0].i32(), Some(42));
    }

    #[test]
    fn invoke_with_valid_preopen_succeeds() {
        // /tmp (or platform equivalent) exists wherever tests run.
        let s = sandbox();
        let bytes = wasm("(module (func (export \"noop\")))");

        let result = s
            .module(&bytes)
            .allow_read_dir(std::env::temp_dir(), "/data")
            .invoke("noop", &[]);
        assert!(
            result.is_ok(),
            "invoke with valid preopen should succeed: {result:?}"
        );
    }

    #[test]
    fn invoke_with_multiple_preopens_succeeds() {
        let s = sandbox();
        let bytes = wasm("(module (func (export \"noop\")))");
        let temp = std::env::temp_dir();

        let result = s
            .module(&bytes)
            .allow_read_dir(&temp, "/in")
            .allow_write_dir(&temp, "/out")
            .invoke("noop", &[]);
        assert!(
            result.is_ok(),
            "multiple preopens should compose: {result:?}"
        );
    }

    #[test]
    fn invoke_with_invalid_host_path_returns_invalid_config() {
        let s = sandbox();
        let bytes = wasm("(module (func (export \"noop\")))");

        let err = s
            .module(&bytes)
            .allow_read_dir("/this/path/should/never/exist/on/any/host", "/data")
            .invoke("noop", &[])
            .unwrap_err();
        assert!(
            matches!(err, SandboxError::InvalidConfig(_)),
            "expected InvalidConfig, got {err:?}"
        );
    }

    #[test]
    fn invoke_propagates_export_not_found_as_trap() {
        // Confirms the same error classification reaches the user via the Invocation path.
        let s = sandbox();
        let bytes = wasm("(module (func (export \"only\")))");

        let err = s.module(&bytes).invoke("missing", &[]).unwrap_err();
        match err {
            SandboxError::Trap(msg) => assert!(msg.contains("missing")),
            other => panic!("expected Trap, got {other:?}"),
        }
    }

    #[test]
    fn invoke_honors_sandbox_memory_limit() {
        // Sandbox-level memory limit (1 page) reaches the guest through Invocation too.
        // Guest tries to grow by 100 pages, gets denied (-1), and returns it.
        let s = Sandbox::builder(ByteSize::kib(64)).build().unwrap();
        let bytes = wasm(
            "(module
                (memory 1)
                (func (export \"grow\") (result i32)
                    i32.const 100
                    memory.grow))",
        );

        let result = s.module(&bytes).invoke("grow", &[]).unwrap();
        assert_eq!(result[0].i32(), Some(-1));
    }

    #[test]
    fn invocation_starts_with_empty_env() {
        let s = sandbox();
        let bytes = wasm("(module)");
        let inv = s.module(&bytes);
        assert!(inv.env.is_empty());
    }

    #[test]
    fn env_setter_records_value() {
        let s = sandbox();
        let bytes = wasm("(module)");
        let inv = s.module(&bytes).env("LANG", "en_US.UTF-8");
        assert_eq!(inv.env.get("LANG"), Some(&"en_US.UTF-8".to_string()));
        assert_eq!(inv.env.len(), 1);
    }

    #[test]
    fn env_last_setter_wins_for_same_key() {
        let s = sandbox();
        let bytes = wasm("(module)");
        let inv = s.module(&bytes).env("KEY", "first").env("KEY", "second");
        assert_eq!(inv.env.get("KEY"), Some(&"second".to_string()));
        assert_eq!(inv.env.len(), 1);
    }

    #[test]
    fn env_multiple_keys_accumulate() {
        let s = sandbox();
        let bytes = wasm("(module)");
        let inv = s
            .module(&bytes)
            .env("FOO", "1")
            .env("BAR", "2")
            .env("BAZ", "3");
        assert_eq!(inv.env.len(), 3);
        assert_eq!(inv.env.get("FOO"), Some(&"1".to_string()));
        assert_eq!(inv.env.get("BAR"), Some(&"2".to_string()));
        assert_eq!(inv.env.get("BAZ"), Some(&"3".to_string()));
    }

    #[test]
    fn env_reaches_guest_via_environ_sizes_get() {
        // Round-trip: set N env vars on the host, verify the guest sees N via WASI.
        let s = Sandbox::builder(ByteSize::mib(1)).build().unwrap();
        let bytes = wasm(
            "(module
                (import \"wasi_snapshot_preview1\" \"environ_sizes_get\"
                    (func $env_sizes (param i32 i32) (result i32)))
                (memory (export \"memory\") 1)
                (func (export \"env_count\") (result i32)
                    ;; environ_sizes_get(count_ptr=0, buf_size_ptr=4)
                    i32.const 0
                    i32.const 4
                    call $env_sizes
                    drop
                    ;; read back the count from addr 0
                    i32.const 0
                    i32.load))",
        );

        let result = s
            .module(&bytes)
            .env("FOO", "1")
            .env("BAR", "2")
            .invoke("env_count", &[])
            .expect("invoke should succeed");
        assert_eq!(result[0].i32(), Some(2));
    }

    #[test]
    fn env_empty_when_no_env_set() {
        // Without any .env() calls the guest should see zero env vars.
        let s = Sandbox::builder(ByteSize::mib(1)).build().unwrap();
        let bytes = wasm(
            "(module
                (import \"wasi_snapshot_preview1\" \"environ_sizes_get\"
                    (func $env_sizes (param i32 i32) (result i32)))
                (memory (export \"memory\") 1)
                (func (export \"env_count\") (result i32)
                    i32.const 0
                    i32.const 4
                    call $env_sizes
                    drop
                    i32.const 0
                    i32.load))",
        );

        let result = s.module(&bytes).invoke("env_count", &[]).unwrap();
        assert_eq!(result[0].i32(), Some(0));
    }
}

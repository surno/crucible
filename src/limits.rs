use wasmtime::ResourceLimiter;

pub(crate) struct CrucibleResourceLimiter {
    memory_cap_bytes: usize,
    // The signal Sandbox checks after a failed call.
    // Bumped (or set true) every time we deny a growth.
    memory_grow_refusals: u32,
}

impl CrucibleResourceLimiter {
    pub(crate) fn new(memory_cap_bytes: usize) -> Self {
        Self {
            memory_cap_bytes,
            memory_grow_refusals: 0,
        }
    }

    /// Whether any memory.grow request was denied during this call.
    pub fn refused_memory_growth(&self) -> bool {
        self.memory_grow_refusals > 0
    }
}

impl ResourceLimiter for CrucibleResourceLimiter {
    fn memory_growing(
        &mut self,
        _current: usize,
        desired: usize,
        _maximum: Option<usize>,
    ) -> wasmtime::Result<bool> {
        if desired > self.memory_cap_bytes {
            self.memory_grow_refusals += 1;
            Ok(false) // deny → memory.grow returns -1 to guest
        } else {
            Ok(true)
        }
    }

    fn table_growing(
        &mut self,
        _current: usize,
        _desired: usize,
        _maximum: Option<usize>,
    ) -> wasmtime::Result<bool> {
        Ok(true)
    }

    fn instances(&self) -> usize {
        1
    }
    fn tables(&self) -> usize {
        1
    }
    fn memories(&self) -> usize {
        1
    }
}

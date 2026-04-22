use wasmtime_wasi::{WasiView, p1::WasiP1Ctx};

use crate::limits::CrucibleResourceLimiter;

pub(super) struct SandboxData {
    pub(super) limiter: CrucibleResourceLimiter,
    pub(super) wasi_p1_ctx: WasiP1Ctx,
}

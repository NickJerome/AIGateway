#[derive(Debug, Clone, Copy)]
pub struct Limits {
    // Per-Engine QuickJS heap limit, excluding host memory.
    pub memory_bytes: usize,
    pub max_stack_bytes: usize,
    pub slice_ms: u64,
    // Shared admission limit for all requests on one worker thread.
    pub max_in_flight: usize,
    // Fixed waitUntil background window after the response is sent.
    pub background_ms: u64,
}

impl Default for Limits {
    fn default() -> Self {
        Limits {
            memory_bytes: 32 * 1024 * 1024,
            max_stack_bytes: 1024 * 1024,
            slice_ms: 1_000,
            max_in_flight: 32,
            background_ms: 30_000,
        }
    }
}

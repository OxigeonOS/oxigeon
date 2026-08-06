//! What a worker process needs to know to build its VM.
//!
//! A deliberate copy of the fields `[compute]` holds rather than the config type
//! itself: the server's `ComputeConfig` is a serde type with defaults, roots and
//! queue policy, none of which a worker has any business seeing. This is the
//! subset that reaches Lua, and it is what crosses the pipe in the handshake.

/// The VM-shaping half of `[compute]`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ComputeSettings {
    /// Instructions one job may execute. 0 disables the check — and with it the
    /// only thing that can interrupt a runaway job from inside the VM.
    pub instruction_limit: u64,
    /// Memory ceiling for the worker VM, in megabytes. 0 = no ceiling.
    pub memory_mb: usize,
    pub max_arg_depth: usize,
    pub max_arg_nodes: usize,
}

impl Default for ComputeSettings {
    fn default() -> Self {
        Self {
            instruction_limit: 0,
            memory_mb: 256,
            max_arg_depth: 64,
            max_arg_nodes: 100_000,
        }
    }
}

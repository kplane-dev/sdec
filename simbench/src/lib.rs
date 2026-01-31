//! Scenario generation and benchmarking for the sdec codec.
//!
//! This crate provides:
//!
//! - Deterministic scenario generators (movement, bursts, spawns)
//! - Loss/reorder simulation for chaos testing
//! - Benchmark harness with JSON/CSV output
//!
//! # Design Principles
//!
//! - **Reproducible** - All scenarios are deterministic given a seed.
//! - **Realistic** - Scenarios model real FPS game patterns.
//! - **Measurable** - Output format suitable for CI regression tracking.

#[cfg(test)]
mod tests {
    #[test]
    fn placeholder_test() {
        // Keep a smoke test to ensure the crate builds in CI.
        let _ = 1 + 1;
    }
}

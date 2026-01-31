//! Introspection and debugging tools for the sdec codec.
//!
//! This crate provides utilities for inspecting and understanding encoded packets:
//!
//! - Decode and print packet structure
//! - Explain packet size by section/component/field
//! - Diff baseline vs current state
//!
//! # Design Principles
//!
//! - **First-class tooling** - These tools are part of the product, not afterthoughts.
//! - **Human-readable output** - Make it easy to understand what the codec is doing.

#[cfg(test)]
mod tests {
    #[test]
    fn placeholder_test() {
        // Keep a smoke test to ensure the crate builds in CI.
        let _ = 1 + 1;
    }
}

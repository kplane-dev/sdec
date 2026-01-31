//! Benchmarks for encode/decode operations.

use criterion::{criterion_group, criterion_main, Criterion};

fn encode_benchmark(_c: &mut Criterion) {
    // Placeholder - will be implemented when codec is complete
}

fn decode_benchmark(_c: &mut Criterion) {
    // Placeholder - will be implemented when codec is complete
}

criterion_group!(benches, encode_benchmark, decode_benchmark);
criterion_main!(benches);

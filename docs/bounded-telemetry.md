# Bounded telemetry aggregation

`Aggregator::new()` and `Default` limit retained state to 4,096 unique keys,
1,024 UTF-8 bytes per key, and 65,536 total key bytes. Use `with_limits` with
`AggregationLimits` for explicit application budgets; zero limits are supported.
These are retained-data limits, not a total heap-size claim: BTreeMap overhead,
input lines, scrubber allocations and JSON rendering require additional memory.

`observe`, `observe_line`, and `merge` now return `Result<(), AggregationError>`.
Rust callers must handle rejection. Counts are exact `u64`; overflow is an error,
never saturation or wrapping. Existing keys can still increment at capacity.
Every rejected operation leaves prior counts and retained-byte accounting intact.
Line staging uses a second bounded aggregate; merge preflights the destination's
limits and all additions before mutation. Allocation failure is outside this
recoverable-error contract. JSON ordering remains count-descending, key-ascending.
The C ABI and header are unchanged.

The `agg` CLI propagates admission failures as a nonzero exit before emitting
any histogram; it does not silently publish partial counts. Error text omits
token content. Its input reader is not a bounded line reader.

## Provenance

Independently authored by OpenAI GPT-6 Astra (`openai/gpt-6-astra`), informed by
the fallible capacity admission and preflighted append design in Tiemoko Ballo's
MIT-licensed [scapegoat](https://github.com/tnballo/scapegoat), reviewed at
`e7fe533f51919de0a97b466fdf362507f7f2010b` in the standardgalactic research pass
(`review-18`). No upstream source or new dependency is incorporated.

Validation covers UTF-8 byte boundaries, zero limits, existing keys at capacity,
count overflow, late merge failure, destination-specific limits, atomic line
rejection, reference-map counts and deterministic tie ordering.

## Validation receipt (2026-10-28)

- `cargo test --locked -j 2`: 48 tests pass; bin/doc targets pass.
- `cargo test --locked --release -j 2`: the same 48 tests pass optimized.
- CLI fixtures: normal counts, 1,025-byte key rejection, and 4,097-key rejection
  pass; rejected input emits no partial histogram on stdout.
- Changed-file rustfmt, standalone telemetry `clippy-driver -D warnings`, and
  `git diff --check` pass.
- Whole-crate Clippy and formatting remain blocked by unchanged baseline code.
  A pristine `0094ccf` worktree reproduces the three Clippy diagnostics in
  `phoenix.rs`/`wave.rs` (approximate golden ratio and `chunks_exact` lints), plus
  formatting differences in existing modules. Phoenix retains legacy floating
  arithmetic; this change introduces none and does not claim to migrate Phoenix.

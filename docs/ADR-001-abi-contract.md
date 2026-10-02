# ADR-001 — the language/ABI contract

Status: accepted · 2026-10-03 · supersedes: qwave's `core/include/qwave_core.h`
(that header becomes a shim over this one during migration)

## Context

Every block of 8b-kit — Swift glue on Apple hosts, Rust glue on
Linux/Unix, the Chez runtime, the GPU lane, the engine facade — must speak
to the decision core. qwave proved the shape: one zero-dependency Rust
staticlib, one C header, thin Swift bindings. 8b-kit generalizes it.

## Decision

**Rust is the decision core. A C ABI is the only crossing. Chez Scheme is
the dynamic layer. Nothing else crosses.**

The contract lives in one file, `include/kit_abi.h`:

1. **C99, `kit_*` namespace, `KIT_ABI_VERSION` exported.** Additive-only
   within a major version; removals bump the major.
2. **Opaque handles.** `kit_phoenix_new` / `kit_phoenix_free` pairs; the
   Rust side owns all long-lived state; Swift/Scheme only hold pointers.
3. **No exceptions, no errno.** Every call returns a sized status code
   (`kit_status_t`), outputs through out-params.
4. **Caller-owned buffers.** In: UTF-8, NUL-terminated. Out:
   length-prefixed (`size_t` capacity, bytes-written return) — no
   cross-ABI allocation surprises.
5. **Sized types only** (`uint32_t`, `float`, `bool`); no enums wider than
   `uint8_t`; structs never cross — byte arrays do (the 79-byte mem8
   frame is the model).
6. **Bindings are hand-written.** Swift declares the header directly; Rust
   bindings are hand-declared `extern "C"` (no bindgen — the zero-dep
   stance); Chez reaches it through a small FFI shim.
7. **Panic-free surface.** The Rust side never unwinds across the ABI;
   internal panics become `KIT_ERR_INTERNAL`.

## Consequences

- Any language can bind the core in an afternoon — the ABI is the SDK.
- The stable binary stays MIT: the core links nothing; Chez (Apache-2.0)
  and the night-side features (AGPL, vendor GPU) stay behind cargo
  features.
- The first `kit_abi.h` exports the qwave-proven surface: egress permits,
  mem8 frame validate/coord, phoenix verdicts, telemetry scrub, mem16
  gate (nightly feature).

## Alternatives considered

- **SwiftPM/Cargo-only APIs** — rejected: locks every consumer into one
  toolchain; the constellation is polyglot by design.
- **bindgen-generated Rust** — rejected: adds a build dependency to the
  zero-dep core for bindings a human writes once.
- **IPC (XPC/sockets) instead of in-process ABI** — rejected for the core:
  process boundaries are an engine-facade concern, not a decision-core one;
  in-process stays fast and testable.

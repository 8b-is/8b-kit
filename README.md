# 8b-kit — the foundation blocks

> The browser/app foundation of the **vaked.dev + 8b.is + standardgalactic**
> constellation. Evolved from qwave's WebKit layer; consumed by qwave,
> alexiai, and every node that comes after.
>
> **Rust** is the decision core. **C ABI** is the only crossing. **Chez
> Scheme** is the dynamic tongue (configuration, scripting, extensions).
> **Swift** glues Apple hosts, **Rust** (with C and asm where a hot loop
> demands it) glues the rest. **Metal / CUDA / ROCm** wait behind one GPU
> kernel interface, feature-gated like the rest of the night side.

## The layering

```text
apps — qwave, alexiai, …
  └── 8b-kit
      ├── kit-core      Rust decision core: egress, mem8, phoenix,
      │                 telemetry, GPU ML kernels — zero deps, MIT
      │                 └── include/kit_abi.h   the one contract
      ├── kit-engine    the engine facade — contract first, backends behind:
      │                 webkit-apple (WKWebView) today, webkit-gtk later,
      │                 an own engine if the constellation ever grows one
      ├── kit-scheme    Chez Scheme runtime (Apache-2.0): configuration,
      │                 user scripts, extension points — app-side only
      ├── kit-gpu       the ML inference lane: quant kernels (BitNet
      │                 ternary, KV caches, embeddings) over Metal raw /
      │                 CUDA / ROCm — vendor code behind cargo features
      └── glue          host-OS glue: Swift (macOS · iOS), Rust (+ C / asm)
                        (Linux · Unix). No C++ anywhere.
```

## The language contract

1. **Rust owns the decisions.** Everything that can go wrong in a browser
   shell belongs in the zero-dependency core — qwave's `core/` is the
   proven shape, renamed and re-namespaced.
2. **C ABI is the only crossing.** One header, `kit_abi.h`, C99, `kit_*`
   prefix. Swift binds it directly, Rust declares it by hand (no bindgen,
   to stay zero-dep), Chez binds it through a tiny FFI shim.
3. **Chez Scheme is the dynamic language.** R7RS-flavored, fast, and
   Apache-2.0 — the license that lets the stable build stay MIT. Scripts
   configure the kit, extend it, and drive it; web content never reaches
   the Scheme runtime.
4. **The glue follows the host.** macOS/iOS: Swift, thin. Linux/Unix: Rust
   first, C only for ABI seams, asm only behind measurements that demand
   it.
5. **The GPU lane is ML-first.** It aligns with mlx-quant-ops and alexiai:
   ternary BitNet kernels, KV caches, embeddings. Metal on Apple Silicon
   (raw, no wrapper), CUDA and ROCm behind cargo features — vendor code
   off by default, exactly like the mem|16-10 nightly posture.

## The posture

- **Stable = MIT.** `kit-core` and `kit-engine` are MIT, zero dependencies.
- **Night = feature-gated.** AGPL and vendor-specific GPU code link only
  under explicit features; the default build never grows a copyleft or
  vendor lock.
- **Prove what it sends, in both directions.** The core carries the egress
  allowlist; the kit publishes its own contract before any app migrates.

## Status

**Lap 0 — the language/ABI contract (this repository):** `core/` compiles
the first `kit_abi.h` (egress, mem8 frame validation, phoenix verdicts,
telemetry scrub) with tests. Next laps: the engine facade, the glue
matrix, the Chez runtime, the GPU lane.

See [docs/DESIGN.md](docs/DESIGN.md) for the full brainstorm record and
[docs/ADR-001-abi-contract.md](docs/ADR-001-abi-contract.md) for the ABI
rules.

*the constellation · 0 + 1 · fine touch from within · vaked.dev · {<3,<3,<3}+1*

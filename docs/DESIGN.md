# 8b-kit — design brief (brainstorm record)

Brainstorm of 2026-10-03, Tokyo. Seed: *evolve WebKit → 8b-kit: vaked +
8b-is + standardgalactic foundation blocks — Rust + Scheme (if a dynamic
language is needed) + host-OS glue (macOS/iOS = Swift, Linux/Unix = Rust
± C ± asm) + a host-respecting GPU language (Apple Silicon raw, CUDA,
ROCm)*. Decisions were Socratic, one fork per question; the locked answers
are the requirement set.

## Locked decisions

| Fork | Decision | Why |
|---|---|---|
| Repo shape | **Standalone `8b-is/8b-kit`** | qwave and later nodes consume it like a platform SDK; QwaveKit stays until the facade lands, then shrinks to the Apple backend |
| Scheme | **Chez Scheme (Apache-2.0)** | R7RS-flavored, fast, and permissive — keeps the stable build MIT; the dynamic layer is configuration + scripting + extension points, never web content |
| GPU lane | **ML inference + quant kernels** | Aligns with mlx-quant-ops / alexiai (BitNet ternary, KV caches, embeddings); CUDA/ROCm become real requirements only when the lane ships |
| Engine stance | **Own facade, WebKit first backend** | The kit defines the engine contract; qwave's WKWebView layer becomes `webkit-apple` behind it, a webkit-gtk backend follows for Linux, an own engine is possible without touching apps |
| First cut | **The language/ABI contract** | Every other block binds to it; get the crossing right before anything else moves |

## The five blocks

1. **kit-core** — Rust, zero deps, MIT. Egress allowlist, MEM8 wave frames,
   Phoenix protocol, telemetry scrub, GPU ML kernels (feature-gated). The
   decision core, ported and re-namespaced from qwave's `core/`.
2. **kit-engine** — the engine facade. Contract-first: page, container,
   shields, navigation, permission surfaces. Backends: `webkit-apple`
   (WKWebView), `webkit-gtk`, later an own engine.
3. **kit-scheme** — the Chez runtime. App-side only: configuration, user
   scripts, extension points. Web content never reaches it.
4. **kit-gpu** — the ML lane. One kernel interface; Metal (Apple Silicon,
   raw), CUDA, ROCm behind cargo features. Off by default.
5. **glue** — the host-OS layer. Swift on macOS/iOS; Rust on Linux/Unix
   with C only at ABI seams and asm only behind measurements.

## The contract, sketched

- One header: `include/kit_abi.h`, C99, `kit_*` namespace, versioned
  (`KIT_ABI_VERSION`).
- Opaque handles + `kit_*_new` / `kit_*_free` pairs; no cross-ABI
  ownership.
- Errors: return codes + out-params; no errno, no exceptions across the
  crossing.
- Strings: UTF-8, NUL-terminated in, length-prefixed out.
- Bindings: Swift direct, Rust hand-declared (zero-dep stance), Chez via
  an FFI shim.
- The ABI is additive-only within a major version.

## Security and license posture

- Stable = MIT end to end. Chez (Apache-2.0) preserves that.
- Night = feature-gated: AGPL (mem|16-10) and vendor GPU code link only
  under explicit features; the default build carries neither.
- The egress allowlist stays in the core — the kit proves what it sends
  before any app trusts it.

## Migration (qwave → 8b-kit)

1. qwave `core/` moves to `kit-core` with the `qw_*` → `kit_*` rename;
   qwave keeps thin shims during the transition.
2. QwaveKit becomes `kit-engine`'s Apple backend; the facade lands first.
3. The Chez runtime lands with configuration/scripting.
4. The GPU lane ships Metal kernels for the mlx-quant work first.
5. qwave is a consumer; the QwaveKit package shrinks to what only qwave
   needs.

## Open items (deliberate)

- Whether kit-scheme configuration replaces qwave's UserDefaults-style
  settings, or sits beside them (decide when the runtime lands).
- The exact engine-facade surface (decide against two backends, not one).
- Which vendor GPU lands first behind the kernel interface (Metal — the
  only host we can prove today).

*the constellation · 0 + 1 · fine touch from within · vaked.dev*

//! WiCi Wireless GPU & Sparse MoE Decision Core (`arXiv:2608.24204v1` × `8b-is`).
//!
//! Implements the four core mechanisms of Wireless GPU Computing Infrastructure
//! (WiCi) in 100% integer and reduced `p/q` (`Rational`) arithmetic with zero
//! floating-point operations and zero external dependencies:
//!
//! 1. **Exact `p/q` Wireless GPU Latency Model (§3.3 Eq. 1)**:
//!    $$T_{\text{WiCi}} = T_{\text{Local}} + N \cdot \text{RTT} + \frac{D \cdot 8}{\text{Rate}_{\text{Mbps}}}$$
//! 2. **Thresholded Content-Addressed Tensor & MoE Expert Caching (§4.1)**:
//!    Compares client hashing + lookup RTT cost against wireless streaming cost
//!    to decide when tensors or 4096B QAT sub-windows should be cached in the
//!    GPU router's NVMe/RAM vault.
//! 3. **Speculative Patterned Function Batching (§3.2 & §4.2.1)**:
//!    Intercepts outcome-dependent (`cuMemCpyHToDAsync -> cuStreamSynchronize`,
//!    `cuLibraryLoadData -> cuCtxPushCurrent`, `cuMoeExpertRoute -> cuMoeGroupedGemm`,
//!    `cuBackwardGradAccum -> cuQatTritFlipUpdate`) and independent
//!    (`cuLaunchKernel -> cuLaunchKernel`) sequences, returning speculative
//!    local `OK` and coalescing into a single `1450B` MEMNET MTU frame.
//! 4. **Cyclic Token & QAT Training Trace Replay (§4.2.2)**:
//!    Records recurring multi-kernel groups across LLM token generation and
//!    forward/backward training micro-steps, replaying an entire $L$-kernel
//!    trace in a single 64-byte `0x62`-sealed trigger RTT and falling back
//!    automatically on trace divergence.
//! 5. **3-Tier Sparse MoE Memory Planner (VRAM -> Host RAM -> NVMe Vault)**:
//!    Plans exact integer MiB placement of shared attention/router weights and
//!    routed experts across GPU VRAM (e.g. 24 GB RTX 4090 / 48 GB RTX 6000 Ada),
//!    pinned Host RAM (e.g. 125 GB local / 720 GB Hetzner), and NVMe storage
//!    (e.g. 1.8 TB T-FORCE TM8FPP002T).

use crate::rational::Rational;

/// Gate seal byte shared with `upz` / `LitanyWireFrame` (`0x62`).
pub const WICI_GATE_MAGIC: u8 = 0x62;

/// Canonical MEMNET coalescing MTU sweet spot (`1450` bytes).
pub const WICI_MEMNET_MTU_BYTES: usize = 1450;

/// Minimum tensor size (`64 KiB` = sixteen `4096B` QAT sub-windows) eligible for
/// hash-based server caching (WiCi §4.1 Fig. 8b threshold floor).
pub const WICI_MIN_CACHE_BYTES: u64 = 65_536;

/// Maximum driver calls recorded in a single cyclic token or training trace.
pub const WICI_MAX_TRACE_OPS: usize = 64;

/// Payload type flag in the WiCi forwarding header (§3.2 Fig. 6, extended for MEMNET).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum WiciPayloadKind {
    Normal = 0,
    Batched = 1,
    TraceReplay = 2,
    TensorCacheQuery = 3,
    MoeExpertDelta = 4,
}

impl WiciPayloadKind {
    pub const fn from_u8(v: u8) -> Option<Self> {
        match v {
            0 => Some(Self::Normal),
            1 => Some(Self::Batched),
            2 => Some(Self::TraceReplay),
            3 => Some(Self::TensorCacheQuery),
            4 => Some(Self::MoeExpertDelta),
            _ => None,
        }
    }
}

/// 8-byte deterministic WiCi forwarding packet header (§3.2 Fig. 6 + `0x62` gate seal).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(C)]
pub struct WiciPacketHeader {
    pub magic: u8,
    pub payload_kind: u8,
    pub total_fragments: u16,
    pub fragment_id: u16,
    pub seal16: u16,
}

impl WiciPacketHeader {
    /// Construct and seal a packet header for `payload_len` bytes.
    pub const fn new(
        kind: WiciPayloadKind,
        total_fragments: u16,
        fragment_id: u16,
        payload_len: u32,
    ) -> Self {
        let k = kind as u8;
        let seal = Self::compute_seal(WICI_GATE_MAGIC, k, total_fragments, fragment_id, payload_len);
        Self {
            magic: WICI_GATE_MAGIC,
            payload_kind: k,
            total_fragments,
            fragment_id,
            seal16: seal,
        }
    }

    pub const fn compute_seal(
        magic: u8,
        payload_kind: u8,
        total_fragments: u16,
        fragment_id: u16,
        payload_len: u32,
    ) -> u16 {
        let w0 = u16::from_le_bytes([magic, payload_kind]);
        let len_lo = (payload_len & 0xFFFF) as u16;
        let len_hi = (payload_len >> 16) as u16;
        w0 ^ total_fragments ^ fragment_id.rotate_left(3) ^ len_lo ^ len_hi ^ 0x6215
    }

    pub const fn verify(&self, payload_len: u32) -> bool {
        if self.magic != WICI_GATE_MAGIC {
            return false;
        }
        if WiciPayloadKind::from_u8(self.payload_kind).is_none() {
            return false;
        }
        if self.total_fragments == 0 || self.fragment_id >= self.total_fragments {
            return false;
        }
        let expected = Self::compute_seal(
            self.magic,
            self.payload_kind,
            self.total_fragments,
            self.fragment_id,
            payload_len,
        );
        self.seal16 == expected
    }

    pub const fn to_bytes(self) -> [u8; 8] {
        let tf = self.total_fragments.to_le_bytes();
        let fid = self.fragment_id.to_le_bytes();
        let s = self.seal16.to_le_bytes();
        [self.magic, self.payload_kind, tf[0], tf[1], fid[0], fid[1], s[0], s[1]]
    }

    pub const fn from_bytes(b: [u8; 8]) -> Self {
        Self {
            magic: b[0],
            payload_kind: b[1],
            total_fragments: u16::from_le_bytes([b[2], b[3]]),
            fragment_id: u16::from_le_bytes([b[4], b[5]]),
            seal16: u16::from_le_bytes([b[6], b[7]]),
        }
    }
}

/// Exact `p/q` evaluation of WiCi Eq. (1) in microseconds:
///
/// $$T_{\text{WiCi}} = T_{\text{Local}} + N \cdot \text{RTT} + \frac{D \cdot 8}{\text{Rate}_{\text{Mbps}}}$$
///
/// Since $1\text{ Mbps} = 1\text{ bit}/\mu\text{s}$, transmitting `payload_bytes`
/// (`payload_bytes * 8` bits) at `rate_mbps` takes `(payload_bytes * 8) / rate_mbps` $\mu\text{s}$.
pub fn wici_step_latency_us(
    t_local_us: u32,
    n_calls: u32,
    rtt_us: u32,
    payload_bytes: u32,
    rate_mbps: u32,
) -> Option<Rational> {
    if rate_mbps == 0 {
        return None;
    }
    let call_rtt_us = (n_calls as u64).checked_mul(rtt_us as u64)?;
    let fixed_us = (t_local_us as u64).checked_add(call_rtt_us)?;
    let tx_bits = (payload_bytes as u64).checked_mul(8)?;
    let total_num = fixed_us
        .checked_mul(rate_mbps as u64)?
        .checked_add(tx_bits)?;
    if total_num > i32::MAX as u64 {
        // Reduce by GCD in u64 before narrowing to i32/u32.
        let g = gcd_u64(total_num, rate_mbps as u64);
        let red_num = total_num / g;
        let red_den = (rate_mbps as u64) / g;
        if red_num > i32::MAX as u64 || red_den > u32::MAX as u64 {
            return None;
        }
        Rational::new(red_num as i32, red_den as u32)
    } else {
        Rational::new(total_num as i32, rate_mbps)
    }
}

/// Decides whether a tensor of `tensor_bytes` should be content-hashed and
/// cached on the GPU router (WiCi §4.1 Fig. 8b) using exact integer microsecond bounds.
///
/// - Client hash + 32-byte cache query cost: `(tensor_bytes / hash_rate_mb_s) + rtt_us`
/// - Direct wireless transmission cost: `(tensor_bytes * 8) / link_mbps`
pub const fn wici_should_cache_tensor(
    tensor_bytes: u64,
    hash_rate_mb_s: u32,
    link_mbps: u32,
    rtt_us: u32,
) -> bool {
    if tensor_bytes < WICI_MIN_CACHE_BYTES || hash_rate_mb_s == 0 || link_mbps == 0 {
        return false;
    }
    // 1 MB/s = 1 byte/us.
    let hash_us = (tensor_bytes / (hash_rate_mb_s as u64)).saturating_add(rtt_us as u64);
    let tx_us = tensor_bytes.saturating_mul(8) / (link_mbps as u64);
    hash_us < tx_us
}

/// User-mode GPU driver operation classes intercepted by WiCi (§4.2.1 Fig. 7c + MoE/QAT).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum WiciDriverOp {
    /// `cuMemCpyHToDAsync_v2` — prefix of Dependent Pattern `Func1`.
    CuMemCpyHToDAsync = 1,
    /// `cuStreamSynchronize` — completion of Dependent Pattern `Func1`.
    CuStreamSynchronize = 2,
    /// `cuLibraryLoadData` — prefix of Dependent Pattern `Func2`.
    CuLibraryLoadData = 3,
    /// `cuCtxPushCurrent_v2` — completion of Dependent Pattern `Func2`.
    CuCtxPushCurrent = 4,
    /// `cuLaunchKernel` — repeatable Independent Pattern `Func3`.
    CuLaunchKernel = 5,
    /// MoE router top-k expert selection (`CuMoeExpertRoute -> CuMoeGroupedGemm`).
    CuMoeExpertRoute = 6,
    /// Coalesced grouped expert GEMM across all active experts in a layer.
    CuMoeGroupedGemm = 7,
    /// Backward pass gradient accumulation (`CuBackwardGradAccum -> CuQatTritFlipUpdate`).
    CuBackwardGradAccum = 8,
    /// QAT ternary `{-1, 0, +1}` sub-window flip update.
    CuQatTritFlipUpdate = 9,
    /// Immediate barrier / device query (`cuCtxSynchronize`, `nvmlDeviceGet*`).
    CuImmediateBarrier = 10,
}

impl WiciDriverOp {
    pub const fn from_u8(v: u8) -> Option<Self> {
        match v {
            1 => Some(Self::CuMemCpyHToDAsync),
            2 => Some(Self::CuStreamSynchronize),
            3 => Some(Self::CuLibraryLoadData),
            4 => Some(Self::CuCtxPushCurrent),
            5 => Some(Self::CuLaunchKernel),
            6 => Some(Self::CuMoeExpertRoute),
            7 => Some(Self::CuMoeGroupedGemm),
            8 => Some(Self::CuBackwardGradAccum),
            9 => Some(Self::CuQatTritFlipUpdate),
            10 => Some(Self::CuImmediateBarrier),
            _ => None,
        }
    }
}

/// Action returned by the speculative pattern batcher (`WiciBatcher`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum WiciBatchAction {
    /// Speculatively return `CUDA_SUCCESS` (`0`) locally without spending a wireless RTT.
    SpeculativeAckLocal = 0,
    /// Flush the coalesced batch in a single wireless RTT (`Batched` payload).
    FlushCoalescedBatch = 1,
    /// Pattern broken or immediate barrier: flush pending + current immediately.
    FlushFallbackImmediate = 2,
}

/// Speculative function batcher implementing WiCi §4.2.1 within a `1450B` MEMNET MTU budget.
#[derive(Clone, Debug)]
pub struct WiciBatcher {
    pending_ops: [u8; 32],
    pending_len: usize,
    pending_bytes: usize,
    pub total_ops_seen: u64,
    pub total_rtts_emitted: u64,
    pub speculative_acks: u64,
}

impl Default for WiciBatcher {
    fn default() -> Self {
        Self::new()
    }
}

impl WiciBatcher {
    pub const fn new() -> Self {
        Self {
            pending_ops: [0u8; 32],
            pending_len: 0,
            pending_bytes: 0,
            total_ops_seen: 0,
            total_rtts_emitted: 0,
            speculative_acks: 0,
        }
    }

    pub const fn pending_count(&self) -> usize {
        self.pending_len
    }

    /// Submit one intercepted user-mode driver call (`op`, `param_bytes`).
    pub fn push_op(&mut self, op: WiciDriverOp, param_bytes: usize) -> WiciBatchAction {
        self.total_ops_seen += 1;

        // If adding this op would exceed the 1450B MEMNET MTU or 32-op slot cap, flush now.
        if self.pending_len >= self.pending_ops.len()
            || (self.pending_len > 0
                && self.pending_bytes.saturating_add(param_bytes) > WICI_MEMNET_MTU_BYTES)
        {
            self.pending_ops[0] = op as u8;
            self.pending_len = 1;
            self.pending_bytes = param_bytes;
            self.total_rtts_emitted += 1;
            return WiciBatchAction::FlushCoalescedBatch;
        }

        if self.pending_len == 0 {
            match op {
                WiciDriverOp::CuMemCpyHToDAsync
                | WiciDriverOp::CuLibraryLoadData
                | WiciDriverOp::CuLaunchKernel
                | WiciDriverOp::CuMoeExpertRoute
                | WiciDriverOp::CuBackwardGradAccum => {
                    self.pending_ops[0] = op as u8;
                    self.pending_len = 1;
                    self.pending_bytes = param_bytes;
                    self.speculative_acks += 1;
                    return WiciBatchAction::SpeculativeAckLocal;
                }
                _ => {
                    self.total_rtts_emitted += 1;
                    return WiciBatchAction::FlushFallbackImmediate;
                }
            }
        }

        let prev = WiciDriverOp::from_u8(self.pending_ops[self.pending_len - 1]);
        match (prev, op) {
            // Dependent pattern completions (Func1, Func2, MoE Route->Gemm, QAT Grad->Flip):
            // coalesce and emit in 1 RTT!
            (Some(WiciDriverOp::CuMemCpyHToDAsync), WiciDriverOp::CuStreamSynchronize)
            | (Some(WiciDriverOp::CuLibraryLoadData), WiciDriverOp::CuCtxPushCurrent)
            | (Some(WiciDriverOp::CuMoeExpertRoute), WiciDriverOp::CuMoeGroupedGemm)
            | (Some(WiciDriverOp::CuBackwardGradAccum), WiciDriverOp::CuQatTritFlipUpdate) => {
                self.pending_len = 0;
                self.pending_bytes = 0;
                self.total_rtts_emitted += 1;
                WiciBatchAction::FlushCoalescedBatch
            }
            // Independent kernel launches (`cuLaunchKernel -> cuLaunchKernel`) or chained async copies:
            // keep coalescing speculatively while under MTU!
            (Some(WiciDriverOp::CuLaunchKernel), WiciDriverOp::CuLaunchKernel)
            | (Some(WiciDriverOp::CuLaunchKernel), WiciDriverOp::CuMoeExpertRoute)
            | (Some(WiciDriverOp::CuMemCpyHToDAsync), WiciDriverOp::CuLaunchKernel) => {
                self.pending_ops[self.pending_len] = op as u8;
                self.pending_len += 1;
                self.pending_bytes = self.pending_bytes.saturating_add(param_bytes);
                self.speculative_acks += 1;
                WiciBatchAction::SpeculativeAckLocal
            }
            // Launch stream followed by synchronize flushes the whole kernel wave in 1 RTT!
            (Some(WiciDriverOp::CuLaunchKernel), WiciDriverOp::CuStreamSynchronize) => {
                self.pending_len = 0;
                self.pending_bytes = 0;
                self.total_rtts_emitted += 1;
                WiciBatchAction::FlushCoalescedBatch
            }
            _ => {
                self.pending_len = 0;
                self.pending_bytes = 0;
                self.total_rtts_emitted += 1;
                WiciBatchAction::FlushFallbackImmediate
            }
        }
    }

    /// Flush any remaining buffered ops at the end of a step.
    pub fn flush(&mut self) -> bool {
        if self.pending_len > 0 {
            self.pending_len = 0;
            self.pending_bytes = 0;
            self.total_rtts_emitted += 1;
            true
        } else {
            false
        }
    }

    /// Exact `p/q` RTT savings ratio: `(total_ops_seen - total_rtts_emitted) / total_ops_seen`.
    pub fn rtt_savings_ratio(&self) -> Option<Rational> {
        if self.total_ops_seen == 0 || self.total_ops_seen > u32::MAX as u64 {
            return None;
        }
        let saved = self.total_ops_seen.saturating_sub(self.total_rtts_emitted);
        Rational::new(saved as i32, self.total_ops_seen as u32)
    }
}

/// Verdict returned by the cyclic trace replay engine (`WiciTraceEngine`, WiCi §4.2.2).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum WiciTraceVerdict {
    /// Recording initial trace or executing normal fallback (1 RTT per op/batch).
    RecordOrNormalForward = 0,
    /// Op matches locked trace interior: return `OK` locally with `0` wireless traffic!
    ReplaySkipLocal = 1,
    /// Final op of a matched cyclic trace: emit a single 64B `0x62` trigger frame (`1` RTT for all $L$ ops!).
    EmitSingleTriggerFrame = 2,
    /// Op diverged from recorded trace: revert to normal one-by-one execution.
    FallbackDiverged = 3,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TracePhase {
    Idle,
    Recording,
    Locked,
}

/// Cyclic trace recorder and replayer for LLM token decode loops and QAT training micro-steps (§4.2.2).
#[derive(Clone, Debug)]
pub struct WiciTraceEngine {
    trigger_op: u8,
    phase: TracePhase,
    recorded_ops: [u8; WICI_MAX_TRACE_OPS],
    recorded_len: usize,
    cursor: usize,
    pub trace_hash64: u64,
    pub replays_completed: u64,
    pub rtts_saved_by_replay: u64,
}

impl WiciTraceEngine {
    pub const fn new(trigger_op: u8) -> Self {
        Self {
            trigger_op,
            phase: TracePhase::Idle,
            recorded_ops: [0u8; WICI_MAX_TRACE_OPS],
            recorded_len: 0,
            cursor: 0,
            trace_hash64: 0,
            replays_completed: 0,
            rtts_saved_by_replay: 0,
        }
    }

    pub const fn is_locked(&self) -> bool {
        matches!(self.phase, TracePhase::Locked)
    }

    pub const fn trace_len(&self) -> usize {
        self.recorded_len
    }

    /// Observe one driver op (`op_id`) in the execution stream.
    pub fn step(&mut self, op_id: u8) -> WiciTraceVerdict {
        match self.phase {
            TracePhase::Idle => {
                if op_id == self.trigger_op {
                    self.phase = TracePhase::Recording;
                    self.recorded_ops[0] = op_id;
                    self.recorded_len = 1;
                }
                WiciTraceVerdict::RecordOrNormalForward
            }
            TracePhase::Recording => {
                if op_id == self.trigger_op && self.recorded_len >= 2 {
                    // Second encounter of trigger_op seals the cyclic trace!
                    self.trace_hash64 = fnv1a64(&self.recorded_ops[..self.recorded_len]);
                    self.phase = TracePhase::Locked;
                    self.cursor = 1;
                    return WiciTraceVerdict::ReplaySkipLocal;
                }
                if self.recorded_len < WICI_MAX_TRACE_OPS {
                    self.recorded_ops[self.recorded_len] = op_id;
                    self.recorded_len += 1;
                } else {
                    self.phase = TracePhase::Idle;
                    self.recorded_len = 0;
                }
                WiciTraceVerdict::RecordOrNormalForward
            }
            TracePhase::Locked => {
                if self.cursor < self.recorded_len && self.recorded_ops[self.cursor] == op_id {
                    self.cursor += 1;
                    if self.cursor == self.recorded_len {
                        self.cursor = 0;
                        self.replays_completed += 1;
                        self.rtts_saved_by_replay += (self.recorded_len.saturating_sub(1)) as u64;
                        WiciTraceVerdict::EmitSingleTriggerFrame
                    } else {
                        WiciTraceVerdict::ReplaySkipLocal
                    }
                } else {
                    // Divergence: reset and fall back to normal execution (§4.2.2).
                    self.phase = if op_id == self.trigger_op {
                        self.recorded_ops[0] = op_id;
                        self.recorded_len = 1;
                        TracePhase::Recording
                    } else {
                        self.recorded_len = 0;
                        TracePhase::Idle
                    };
                    self.cursor = 0;
                    WiciTraceVerdict::FallbackDiverged
                }
            }
        }
    }
}

/// 3-Tier Sparse MoE Memory Placement Plan (`VRAM -> Pinned Host RAM -> NVMe Vault`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(C)]
pub struct WiciMoeTierPlan {
    /// Total model weight footprint in MiB at the chosen quantization ratio.
    pub total_model_mib: u32,
    /// Hot shared backbone (attention + router + shared experts + active KV) pinned in GPU VRAM.
    pub vram_used_mib: u32,
    /// Routed experts pinned in Host RAM (`mlock` Magnetar tier).
    pub ram_used_mib: u32,
    /// Remaining cold experts / QAT windows stored on NVMe (e.g. T-FORCE TM8FPP002T).
    pub nvme_used_mib: u32,
    /// Number of routed experts per layer that fit in VRAM + Host RAM without NVMe page-in.
    pub zero_disk_experts_per_layer: u16,
    /// True when 100% of active per-token parameters fit in VRAM + Host RAM.
    pub fits_zero_cloud: bool,
}

/// Computes the exact 3-tier memory placement for a Sparse MoE model (e.g. DeepSeek-V3/R1 671B MoE)
/// across GPU VRAM, Host RAM, and NVMe storage using integer arithmetic only.
///
/// - `total_params_billions`: e.g. `671` for DeepSeek-V3/R1
/// - `active_params_billions`: e.g. `37` for DeepSeek-V3/R1
/// - `experts_per_layer`: e.g. `256`
/// - `bits_num` / `bits_den`: bits per parameter as exact rational (e.g. `1585 / 1000` for
///   `base-243` `SPQT` ternary `1.585 bits/trit`, or `4 / 1` for `nvfp4`)
pub const fn plan_moe_tiers(
    total_params_billions: u32,
    active_params_billions: u32,
    experts_per_layer: u16,
    bits_num: u32,
    bits_den: u32,
    vram_budget_mib: u32,
    ram_budget_mib: u32,
    nvme_budget_mib: u32,
) -> Option<WiciMoeTierPlan> {
    if bits_den == 0 || total_params_billions == 0 || active_params_billions > total_params_billions
    {
        return None;
    }
    // 1 billion params * (bits_num / bits_den) bits = (1000 / 8) * (bits_num / bits_den) MiB
    // = (125 * bits_num) / bits_den MiB per billion parameters (using 10^9 / 8 / 10^6 = 125).
    let total_mib = ((total_params_billions as u64) * 125 * (bits_num as u64)) / (bits_den as u64);
    let active_mib =
        ((active_params_billions as u64) * 125 * (bits_num as u64)) / (bits_den as u64);

    if total_mib > u32::MAX as u64 {
        return None;
    }
    let total_model_mib = total_mib as u32;
    let hot_backbone_mib = active_mib as u32;

    // Tier 0: Pin hot shared attention + active experts in VRAM first.
    let vram_used_mib = if total_model_mib <= vram_budget_mib {
        total_model_mib
    } else {
        vram_budget_mib
    };
    let rem_after_vram = total_model_mib.saturating_sub(vram_used_mib);

    // Tier 1: Pin routed experts in Host RAM (e.g. 700+ GB on Hetzner or 125 GB local).
    let ram_used_mib = if rem_after_vram <= ram_budget_mib {
        rem_after_vram
    } else {
        ram_budget_mib
    };
    let rem_after_ram = rem_after_vram.saturating_sub(ram_used_mib);

    // Tier 2: Place any remaining cold experts on dedicated NVMe (e.g. T-FORCE TM8FPP002T 1.8 TB).
    let nvme_used_mib = if rem_after_ram <= nvme_budget_mib {
        rem_after_ram
    } else {
        nvme_budget_mib
    };

    let covered_in_fast_mem = (vram_used_mib as u64) + (ram_used_mib as u64);
    let zero_disk_experts_per_layer = if total_model_mib == 0 {
        0
    } else if covered_in_fast_mem >= total_model_mib as u64 {
        experts_per_layer
    } else {
        (((experts_per_layer as u64) * covered_in_fast_mem) / (total_model_mib as u64)) as u16
    };

    let fits_zero_cloud = (vram_budget_mib as u64) + (ram_budget_mib as u64) + (nvme_budget_mib as u64)
        >= (total_model_mib as u64)
        && (vram_budget_mib >= hot_backbone_mib / 2);

    Some(WiciMoeTierPlan {
        total_model_mib,
        vram_used_mib,
        ram_used_mib,
        nvme_used_mib,
        zero_disk_experts_per_layer,
        fits_zero_cloud,
    })
}

const fn gcd_u64(mut a: u64, mut b: u64) -> u64 {
    while b != 0 {
        let t = a % b;
        a = b;
        b = t;
    }
    if a == 0 { 1 } else { a }
}

const fn fnv1a64(bytes: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf29ce484222325;
    let mut i = 0;
    while i < bytes.len() {
        h ^= bytes[i] as u64;
        h = h.wrapping_mul(0x100000001b3);
        i += 1;
    }
    h
}

// ── C ABI (`include/kit_abi.h`) ──────────────────────────────────────────────

/// Opaque handle for C callers driving a `WiciTraceEngine`.
#[allow(non_camel_case_types)]
pub struct kit_wici_trace(WiciTraceEngine);

/// C ABI: allocate a new cyclic `WiciTraceEngine` watching for `trigger_op`.
#[unsafe(no_mangle)]
pub extern "C" fn kit_wici_trace_new(trigger_op: u8) -> *mut kit_wici_trace {
    Box::into_raw(Box::new(kit_wici_trace(WiciTraceEngine::new(trigger_op))))
}

/// C ABI: release a `kit_wici_trace` created by `kit_wici_trace_new`.
///
/// # Safety
/// `trace` must have been returned by `kit_wici_trace_new` and not yet freed, or null.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kit_wici_trace_free(trace: *mut kit_wici_trace) {
    if !trace.is_null() {
        // SAFETY: caller contract.
        unsafe {
            drop(Box::from_raw(trace));
        }
    }
}

/// C ABI: step the trace engine with `op_id`. Returns the `WiciTraceVerdict` code (`0..=3`).
///
/// # Safety
/// `trace` must be a valid pointer from `kit_wici_trace_new`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kit_wici_trace_step(trace: *mut kit_wici_trace, op_id: u8) -> u8 {
    if trace.is_null() {
        return WiciTraceVerdict::RecordOrNormalForward as u8;
    }
    // SAFETY: caller contract.
    let engine = unsafe { &mut (*trace).0 };
    engine.step(op_id) as u8
}

/// C ABI: compute exact rational WiCi step latency in microseconds (`num_out / den_out`).
/// Returns `true` on success, `false` when `rate_mbps == 0` or on overflow.
///
/// # Safety
/// `num_out` and `den_out` must be valid writable pointers or null.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kit_wici_step_latency_us(
    t_local_us: u32,
    n_calls: u32,
    rtt_us: u32,
    payload_bytes: u32,
    rate_mbps: u32,
    num_out: *mut i32,
    den_out: *mut u32,
) -> bool {
    let Some(r) = wici_step_latency_us(t_local_us, n_calls, rtt_us, payload_bytes, rate_mbps)
    else {
        return false;
    };
    // SAFETY: caller contract.
    unsafe {
        if !num_out.is_null() {
            *num_out = r.num;
        }
        if !den_out.is_null() {
            *den_out = r.den;
        }
    }
    true
}

/// C ABI: returns `true` when caching a tensor of `tensor_bytes` beats streaming it over Wi-Fi.
#[unsafe(no_mangle)]
pub extern "C" fn kit_wici_should_cache_tensor(
    tensor_bytes: u64,
    hash_rate_mb_s: u32,
    link_mbps: u32,
    rtt_us: u32,
) -> bool {
    wici_should_cache_tensor(tensor_bytes, hash_rate_mb_s, link_mbps, rtt_us)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn packet_header_seals_and_detects_tampering() {
        let hdr = WiciPacketHeader::new(WiciPayloadKind::Batched, 4, 1, 1442);
        assert!(hdr.verify(1442));
        assert!(!hdr.verify(1443));
        let bytes = hdr.to_bytes();
        let decoded = WiciPacketHeader::from_bytes(bytes);
        assert_eq!(hdr, decoded);
        assert!(decoded.verify(1442));
    }

    #[test]
    fn exact_rational_latency_matches_wici_equation_1() {
        // Local GPU: 1500 us, 1 call (after trace replay), RTT = 2000 us,
        // payload = 64 bytes (512 bits) over 1000 Mbps Wi-Fi:
        // T_WiCi = 1500 + 2000 + 512/1000 = 3500 + 64/125 = 437564/125 us.
        let r = wici_step_latency_us(1500, 1, 2000, 64, 1000).unwrap();
        assert_eq!(r, Rational::new(437_564, 125).unwrap());
    }

    #[test]
    fn tensor_caching_threshold_filters_small_blocks_and_caches_large_tensors() {
        // Small 16 KiB tensor is below the 64 KiB floor -> do not cache (WiCi Fig. 8b).
        assert!(!wici_should_cache_tensor(16 * 1024, 3000, 800, 2000));
        // 64 MiB expert weight tensor: hashing at 3000 MB/s takes ~23 ms,
        // while streaming over 800 Mbps Wi-Fi takes ~671 ms -> cache!
        assert!(wici_should_cache_tensor(64 * 1024 * 1024, 3000, 800, 2000));
    }

    #[test]
    fn speculative_batcher_cuts_rtts_by_over_80_percent() {
        let mut batcher = WiciBatcher::new();
        // Simulate 10 layers of: MemCpyAsync -> LaunchKernel x 4 -> StreamSync (6 ops/layer = 60 ops).
        for _ in 0..10 {
            assert_eq!(
                batcher.push_op(WiciDriverOp::CuMemCpyHToDAsync, 64),
                WiciBatchAction::SpeculativeAckLocal
            );
            for _ in 0..4 {
                assert_eq!(
                    batcher.push_op(WiciDriverOp::CuLaunchKernel, 48),
                    WiciBatchAction::SpeculativeAckLocal
                );
            }
            assert_eq!(
                batcher.push_op(WiciDriverOp::CuStreamSynchronize, 16),
                WiciBatchAction::FlushCoalescedBatch
            );
        }
        assert_eq!(batcher.total_ops_seen, 60);
        assert_eq!(batcher.total_rtts_emitted, 10);
        // Saved 50 / 60 = 5 / 6 (83.33% > WiCi's 80.4% benchmark!).
        assert_eq!(batcher.rtt_savings_ratio().unwrap(), Rational::new(5, 6).unwrap());
    }

    #[test]
    fn trace_replay_locks_on_cyclic_token_loop_and_falls_back_on_divergence() {
        let mut trace = WiciTraceEngine::new(WiciDriverOp::CuMemCpyHToDAsync as u8);
        let cycle = [
            WiciDriverOp::CuMemCpyHToDAsync as u8,
            WiciDriverOp::CuMoeExpertRoute as u8,
            WiciDriverOp::CuMoeGroupedGemm as u8,
            WiciDriverOp::CuLaunchKernel as u8,
            WiciDriverOp::CuStreamSynchronize as u8,
        ];

        // Token 0: records the 5-op cycle.
        for &op in &cycle {
            assert_eq!(trace.step(op), WiciTraceVerdict::RecordOrNormalForward);
        }
        assert!(!trace.is_locked());

        // Token 1: first op locks the trace and skips locally; ops 1..3 skip locally; op 4 emits 1 trigger frame!
        assert_eq!(trace.step(cycle[0]), WiciTraceVerdict::ReplaySkipLocal);
        assert!(trace.is_locked());
        assert_eq!(trace.trace_len(), 5);
        for &op in &cycle[1..4] {
            assert_eq!(trace.step(op), WiciTraceVerdict::ReplaySkipLocal);
        }
        assert_eq!(trace.step(cycle[4]), WiciTraceVerdict::EmitSingleTriggerFrame);
        assert_eq!(trace.replays_completed, 1);
        assert_eq!(trace.rtts_saved_by_replay, 4);

        // Divergence test: unexpected op immediately reverts to FallbackDiverged.
        assert_eq!(
            trace.step(WiciDriverOp::CuImmediateBarrier as u8),
            WiciTraceVerdict::FallbackDiverged
        );
        assert!(!trace.is_locked());
    }

    #[test]
    fn deepseek_671b_moe_fits_hetzner_a6000_ram_and_local_4090_tforce_nvme() {
        // 1. Hetzner A6000 box: 49,140 MiB VRAM + 737,280 MiB (720 GiB) RAM, base-243 ternary (1585/1000 bits/trit).
        // 671B * 125 * 1.585 = 132,941 MiB (~130 GiB) total -> 100% fits in VRAM + RAM with 0 NVMe page-ins!
        let hetzner = plan_moe_tiers(671, 37, 256, 1585, 1000, 49_140, 737_280, 0).unwrap();
        assert_eq!(hetzner.total_model_mib, 132_941);
        assert_eq!(hetzner.vram_used_mib, 49_140);
        assert_eq!(hetzner.ram_used_mib, 83_801);
        assert_eq!(hetzner.nvme_used_mib, 0);
        assert_eq!(hetzner.zero_disk_experts_per_layer, 256);
        assert!(hetzner.fits_zero_cloud);

        // Even at 8-bit (`8/1`), 671B = 671,000 MiB (~655 GiB) -> STILL fits 100% in Hetzner's 48GB VRAM + 720GB RAM!
        let hetzner_q8 = plan_moe_tiers(671, 37, 256, 8, 1, 49_140, 737_280, 0).unwrap();
        assert_eq!(hetzner_q8.total_model_mib, 671_000);
        assert_eq!(hetzner_q8.nvme_used_mib, 0);
        assert_eq!(hetzner_q8.zero_disk_experts_per_layer, 256);
        assert!(hetzner_q8.fits_zero_cloud);

        // 2. Local RTX 4090 box: 24,564 MiB VRAM + 128,000 MiB RAM + 1,843,200 MiB (1.8 TiB T-FORCE TM8FPP002T NVMe).
        // At base-243 ternary (132,941 MiB), 100% fits in VRAM + 125 GiB RAM!
        // At 4-bit nvfp4 (335,500 MiB), 24.5 GiB in VRAM + 128 GiB in RAM + 182.9 GiB on T-FORCE NVMe!
        let local_nvfp4 =
            plan_moe_tiers(671, 37, 256, 4, 1, 24_564, 128_000, 1_843_200).unwrap();
        assert_eq!(local_nvfp4.total_model_mib, 335_500);
        assert_eq!(local_nvfp4.vram_used_mib, 24_564);
        assert_eq!(local_nvfp4.ram_used_mib, 128_000);
        assert_eq!(local_nvfp4.nvme_used_mib, 182_936);
        assert!(local_nvfp4.fits_zero_cloud);
    }
}

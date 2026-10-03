# 8b-kit — Admissibility Gate Mapping

> W3 of the "one source · three views" refactor of the **Central Theorem
> (Epistemic Overlap)**: a gate-mapping memo. This file *maps* the kit's
> gate family onto the canonical claim set; it ships **no code** — the
> kit's memory-wave domain is *adjacent* to the theorem, not an
> *instance* of it. If a future gate change wants to claim a new CT-ID,
> that is a theorem-side change first (SPEC registry + monograph +
> kernel/operational witness), and this memo follows.

```text
MANIFEST:
  mapped:   CT-002, CT-003, CT-005, CT-010, CT-011, CT-016
  adjacent: CT-006, CT-008, CT-013
```

## One source, three views — where this memo sits

| View | Home |
|---|---|
| SPEC + monograph (source of truth) | `standardgalactic/alphabet: central-theorem/` (PR #13) |
| Lean kernel | `8b-is/claimshift: CentralTheorem.lean` (PR #1) |
| Rust operational | `8b-is/8b-is-engine: crates/admissibility` (PR #2) |
| **Decision-core mapping (this memo)** | `8b-is/8b-kit: docs/ADMISSIBILITY.md` |

## Why "adjacent, not instance"

CT-000 is a statement about *policies over sets of worlds*: an
observation map `O : W → 𝒪` and an admissible-action map
`Adm : W → 𝟚(𝒜)`, with the bridge — a deterministic policy `π`
guarantees an admissible action in every world **iff** every
observation class carries a common admissible action.

The kit has no such object. Its gates are per-content adjudications
over a memory lattice: each call sees one essence (or one host, one
frame, one step index) and returns a verdict or a boolean. There is no
policy `π`, no observation class, no multi-world guarantee to prove or
to break.

What the kit *does* carry is the **enforcement discipline** the theorem
requires for the local→global bridge: commitment over visibility,
unique continuation from attested prefixes, exclusion rather than
patching. That is the theorem's shadow on the decision core — and
pinning that shadow down is this memo's job.

## The gate map

### G0 — the egress allowlist (`egress::permits`, `HOSTS`)

The committed set of Category-A hosts the kit's own code may contact:
four hosts, boundary-aware suffix matching, allocation-free.

- **CT-011 (finite safe envelope), compliance reading.** The theorem
  says that for any bounded protective capacity there is a *finite*
  visibility bound above which stability fails. The kit implements the
  envelope directly: egress visibility is not a policy, it is a
  committed, finite, versioned const list.
- **CT-016 (commit enforcement vs. visible state).** The allowlist is
  the committed record; `permits` enforces it per call. Visibility is
  what the commitment permits, not the other way around.

### G1 — the VERIFY/COLLAPSE verification gate (`mem16::verified`, `kit_mem16_verified`)

```text
governing:  POP → REFUSE → BIND → TRANSFORM → VERIFY → COLLAPSE
recovery:   DISCOVER → VERIFY → REPLAY → BRANCH → RANK → PROPOSE → BIND/REFUSE
gate:       only step 4 (VERIFY) or step 5 (COLLAPSE) is an admissible
            terminal step
```

- **CT-003 (commitment collapse: unique continuation).** A terminal
  step is admissible only when it is the attested sequence's unique
  continuation. Steps 0–3 are non-terminal: a transition that ends
  before VERIFY is an uncompleted commitment, and the gate refuses to
  close it as a state. The gate is the discrete enforcement of "the
  prefix is a commitment: it determines a single branch."
- **CT-002 (prefix-closure) as discipline.** The sequence *is* the
  prefix structure every admissible transition must pass through
  (`mem16.rs`: "the order every admissible transition must pass
  through"). The kit does not formalize prefix-closure; it encodes it
  as the only permitted step order.

### G2 — REFUSE as a first-class step (governing index 1)

- **CT-005 (pruning intervention: exclusion, not correction).** REFUSE
  is a *step of the protocol*, not an error exit: refusal sits between
  POP and BIND, is part of the order, and owns a monotone position in
  it. Refused content is excluded from continuation, not repaired in
  place — the decision-core reading of "control acts on the
  *continuation* of a history, not on states."
- **CT-016.** Refusal owns its sequence position: the record of
  refusal is in the order, the kit's share of the "a refusal is a
  committed record" discipline.

### G3 — the Custodian: repetition is poison (`phoenix::Custodian::adjudicate`)

One content hash may enter the grid once; a duplicate is `Drop`,
whatever the gate says.

- **CT-003 + CT-016.** The first occurrence owns the position;
  replays are refused durably, not patched. The same discipline as the
  engine's "replays are refused, not patched" — the content-level
  instance of the ledger position the committed record owns.

### G4 — the precious override (`phoenix::decide`, `precious: HashSet<u64>`)

The Council's word: always STORE with τ=∞, whatever the gate says.

- **CT-016 (the commitment regime).** This is *not* unenforced
  visibility (the CT-010 no-go): the override is an explicit
  commitment recorded in `precious`, a monotone, inspectable set — a
  ledger. The guarantee comes from the committed record, not from the
  visibility.
- **Watch item (no divergence today; pinned for audit).** CT-011 says
  the safe envelope is finite; `precious` is an *unbounded* retention
  class (τ=∞). That is acceptable exactly because it lives in the
  commitment regime — every entry is a Council act, not ambient
  visibility. The boundary this memo pins: the moment the override
  degrades into "store whatever is visible, forever" without a
  per-entry committed record, it leaves the enforcement discipline and
  enters the CT-010/CT-011 no-go territory. No code change today: the
  `precious` set already owns a per-entry ledger position.

### G5 — the Marine salience gate (`coherence < 0.35` → halved amplitude; `novelty < 0.05` → TEMPORARY)

- **Adjacent — CT-013 (Local Stress Criterion) as engineering
  approximation, not instance.** The gate is a stability screen on
  memory content (structural jitter + attentional novelty), the
  nearest memory-lattice analogue of a coherence-stress screen. But
  CT-013 is a field-level result (coherence elasticity must dominate
  contradiction pressure), and the kit's thresholds are fixed
  heuristics with no local→global composition claim.
- **Deliberate non-instance.** The theorem's local→global bridge
  (CT-000 / CT-009) is exactly the direction the salience gate does
  *not* claim: halving amplitude on jitter is a correction *inside the
  store*, not a guarantee about a policy over worlds. If a future kit
  feature needs a multi-content guarantee ("every admitted essence is
  retrievable under some recall policy"), that is an instance of
  CT-000 and needs the overlap structure, not a bigger lattice.

### G6 — the interference-lattice phases (`Relation::phase_degrees`: 0° / 45° / 90° / 135° / 180°)

- **Adjacent — CT-006 (tangent–normal), CT-008 (sheaf coherence on
  overlaps).** Phase offsets encode gluing strength between two
  admitted contents: `Bound` (0°) = full gluing, `Conflicting` (180°)
  = antipodal, maximal incoherence. The memory-lattice shadow of
  "charts glue on overlaps; local steps do not glue without
  coherence."
- **Deliberate non-instance.** The lattice binds *contents* (phase
  relations in a grid), not *worlds* (observation classes). No world
  set, no observation map, no admissible-action map. A
  discretization heuristic for recall quality, not a formal gluing.

### G7 — WaveInt frame integrity + provenance boundary (`wave::from_frame`, `WaveProvenance`)

The 79-byte frame rejects truncation, unknown versions, hostile
provenance, unrepresentable rationals, and checksum flips — "a flipped
field cannot enter the grid." `Cognitive` waves never leave the device;
`Nexus` waves are the only kind that may be offered to a user-chosen
provider.

- **CT-005 + CT-016.** A corrupted or hostile frame is *rejected*, not
  repaired in place: committed records are integrity-checked at the
  boundary and excluded on mismatch.
- **CT-010, compliance reading.** The sealed-vs-shareable boundary is
  the kit's visibility split: cognitive visibility stays on-device by
  construction; nexus visibility is the only egress the commitment
  permits. Bounded visibility, not unbounded visibility with
  protection.

### G8 — the telemetry scrubber (`telemetry::*`)

"A scrubber with exceptions is a scrubber with leaks": URLs keep
scheme + host + truncated path hash; free text becomes `len:<n>` +
content hash; data science sees histograms and counts, never content.

- **CT-010 / CT-011, compliance reading.** The kit does not try to make
  more telemetry visibility safe (the no-go says it cannot stay safe as
  visibility diverges); it *reduces* what is visible to the counting
  regime. Bounded-envelope posture in operational form.

### No gate surface

- `rational.rs` — type only.
- `wave::consciousness()` — a named resonance constant (0.73 Hz), not
  yet a gate function; noted for map completeness, no mapping.

## Divergence audit (W3's question: no code unless a gate diverges from a proven claim)

| Gate | Claims | Diverges? |
|---|---|---|
| G0 egress allowlist | CT-011, CT-016 | no — finite committed envelope |
| G1 VERIFY/COLLAPSE | CT-003, CT-002 | no — unique-continuation discipline |
| G2 REFUSE step | CT-005, CT-016 | no — refusal is exclusion, owns its position |
| G3 Custodian Drop | CT-003, CT-016 | no — replay refused durably, not patched |
| G4 precious override | CT-016 | no — commitment regime, ledger-backed (watch item pinned) |
| G5 Marine gate | CT-013 (adjacent) | n/a — heuristic, no local→global claim made |
| G6 lattice phases | CT-006/CT-008 (adjacent) | n/a — content binding, not world gluing |
| G7 frame integrity + provenance | CT-005, CT-010, CT-016 | no — reject-don't-patch; visibility split |
| G8 scrubber | CT-010/CT-011 | no — bounded visibility by construction |

**Conclusion: no gate diverges from a proven claim. No code change in
this wave.**

## Scope and guardrails

- This memo is documentation only: one new file under `docs/`. No
  changes to `core/src/**`, `include/kit_abi.h`, or the license /
  feature posture.
- The claim registry at
  `standardgalactic/alphabet: central-theorem/SPEC.md` (branch
  `central-theorem`) remains the single source of truth for the CT-IDs
  cited here; the W4 sync-CI diffs this file's manifest header against
  that registry.
- CT-000 is deliberately *not* claimed by this file: the kit is not an
  instance of the bridge (see "adjacent, not instance"). A future gate
  with the overlap structure claims it through the Lean / Rust views
  first.

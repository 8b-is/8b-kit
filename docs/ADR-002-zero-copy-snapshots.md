# ADR-002 — zero-copy snapshots stay out of the dependency core

Status: accepted · 2026-10-29 · relates to ADR-001 (the language/ABI contract)

## Context

A zero-copy node snapshot served straight from the page cache would let a
consumer read ids and payloads without deserialisation or heap copies. The
design is sound, and it has a sharp edge: `mmap` reserves address space eagerly
and loads blocks on first touch, so a page fault on an inode that has since
shrunk, or past EOF, delivers `SIGBUS` and kills the process. The remedy is
structural, not a signal handler: immutable inode replacement (`write tmp` →
`fsync` → `0400` → `rename`), POSIX unlinked-inode lifetime, a pre-map `fstat`
envelope check, and a verified footer digest after `MADV_WILLNEED`.

The question for this repository is only where such a mechanism may live.

## Decision

**`kit-core` does not acquire an `mmap` path, and does not add `libc` or any
other dependency to do so.**

The blocking reason is the charter in ADR-001 and `Cargo.toml`: `kit-core` is
the zero-dependency decision core. `mmap`, `munmap`, and `madvise` have no `std`
equivalent, so a mapping requires a foreign crate — which the charter forbids
without a stdlib blocker, and this is not a blocker the core faces, because the
core owns no persistent node store.

`phoenix.rs` consumes a salience decision over admitted hashes; it does not read
files. There is no consumer, so there is no mapping.

If a consumer appears, the mapping belongs **outside** `kit-core`: an opt-in
crate, a C-ABI adapter that crosses at `include/kit_abi.h`, or the Python side
where `mmap` is in the standard library.

## Canonical implementation

[Aurora PR #5](https://github.com/8b-is/aurora/pull/5) — `tools/zero_copy_snapshot.py`. Stdlib only, `AURSNAP1`
format, no third-party dependency, with a self-check covering rotation under a
live reader, truncation rejected before mapping, and digest failure on a
flipped byte. That is the reference; do not write a second layout.

## Consequences

- `kit-core` keeps its zero-dependency posture and its `#![forbid(unsafe_code)]`
  status untouched.
- A future file-backed consumer pays for the dependency itself, in its own
  crate, rather than taxing every user of the core.
- The `SIGBUS` invariants are recorded here so the next implementer does not
  rediscover them by crashing.

## Limits of the mapping contract

Immutable replacement protects readers only when all writers cooperate. Mode
`0400` does not stop a privileged process, the owner changing permissions, or
an already-open writable descriptor from truncating an inode. `MADV_WILLNEED`
is advisory, and a successful digest check cannot prevent later I/O faults.
Validate physical size before mapping and header bounds before exposing data;
never treat these checks as a guarantee that `SIGBUS` is impossible.

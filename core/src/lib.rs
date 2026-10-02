//! The 8b-kit decision core, in zero-dependency Rust.
//!
//! This crate carries the parts of the constellation that must not depend
//! on a UI framework or a host OS: the Category-A egress allowlist, the
//! MEM8 wave substrate (the 79-byte `WaveInt` frame), the Phoenix protocol,
//! and the telemetry scrubber. Host glue (Swift on Apple hosts, Rust on the
//! rest) keeps the shell; the decisions live here, behind one small C ABI
//! (`include/kit_abi.h`) that any language can call.
//!
//! Stable = MIT. The AGPL sovereign library (`mem-16-10`) and vendor GPU
//! code link only under explicit cargo features — the default build carries
//! neither.

pub mod egress;
pub mod mem16;
pub mod phoenix;
pub mod rational;
pub mod telemetry;
pub mod wave;

pub use rational::Rational;

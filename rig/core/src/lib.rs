//! The rig primitives the appliance and the JPEG XS link share (issue 1487).
//!
//! `camera-box` re-exports each module at its old path (`camera_box::genlock_grid`, ...), and
//! `camera-box-jxs` depends on this crate instead of on the appliance, so the static library the
//! OBS build links carries none of the appliance. Each module exists once: `genlock_grid` is the
//! Rust authority of `obs-genlock-grid.h`, pinned by `tests/genlock_grid_parity_1355.rs`.

#[cfg(target_os = "linux")]
pub mod affinity;
pub mod camera_number;
// The genlock FIFO's backlog-storm threshold, made latency-relative (camera-box #859). The C
// `GENLOCK_QDEPTH_RELOCK` in obs-source.c derives from here.
pub mod genlock_backlog;
pub mod genlock_grid;
// The N==1 source's held depth after a restart (camera-box #1367); `genlock_backlog` routes an
// N==1 tick here. The C `genlock_n1_*` helpers in obs-source.c mirror it.
pub mod genlock_n1_depth;
// The one wall-step detector of the genlock render tick (camera-box #1372): a dantesync fleet DATE
// step re-grids the tick in one tick. It reacts to the clock; the clock itself stays in dsync. The C
// twin is `libobs/obs-genlock-wall-step.h`.
pub mod genlock_wall_step;
pub mod irq_offload;

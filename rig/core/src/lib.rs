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
// The receive FIFO's relabel of old-epoch frames by the booked wall step (camera-box #1372). C twin
// `libobs/obs-genlock-fifo-relabel.h` (vstack#8).
pub mod genlock_fifo_relabel;
pub mod genlock_grid;
// The in-OBS JPEG XS indicator (OFF / OK / DEGRADED / DOWN) and the Sources-dock transport tag
// (camera-box #1387). C twin `frontend/widgets/GenlockJxsState.hpp` (vstack#8).
pub mod genlock_jxs_state;
// The LOCKED / DEGRADED / UNLOCKED decision of the in-OBS genlock indicator (camera-box #1298). C
// twin `frontend/widgets/GenlockLockState.hpp` (vstack#8).
pub mod genlock_lock_state;
// The N==1 source's held depth after a restart (camera-box #1367); `genlock_backlog` routes an
// N==1 tick here. The C `genlock_n1_*` helpers in obs-source.c mirror it.
pub mod genlock_n1_depth;
// The grid-exact N>=2 conveyor (camera-box #1367): an N>=2 source presents a pure function of the
// render tick and the pin. The C `genlock_n2_*` in obs-source.c mirrors it (vstack#8).
pub mod genlock_n2_grid;
// The one wall-step detector of the genlock render tick (camera-box #1372): a dantesync fleet DATE
// step re-grids the tick in one tick. It reacts to the clock; the clock itself stays in dsync. The C
// twin is `libobs/obs-genlock-wall-step.h`.
pub mod genlock_wall_step;
pub mod irq_offload;

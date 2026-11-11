//! The test-burn geometry shared with the OBS/DistroAV C++ (vstack#8 batch 3, from camera-box dev
//! 389622d). Pure std, no I/O, so every module unit-tests on default features. camera-box keeps
//! the probe code that blits and decodes these burns, the rig verdicts built on them, and the
//! parity tests against the C++ mirrors (`burn-geom.hpp`, `burn-tick-cache.hpp`).

// #367 — the colour-reference scale beside the dual-QR: the patch table and the gap geometry.
// camera-box's probe-gated `probe::qr` blits it.
pub mod colour_scale;

// #1260 — the within-tick "prepare once, reuse" state of the DistroAV QR burn filter. The authority
// the C mirror `vendor/distroav/src/burn-tick-cache.hpp` is checked against.
pub mod burn_tick_cache;

// issue 1370 — the node-burn SLOT table (the camera capture burn and the four `burn-geom.hpp`
// corners) on a W×H frame. camera-box pins the corners to the C++ in
// `tests/burn_regions_cpp_parity_1370.rs`.
pub mod burn_regions;

// issue 1367 — find a node burn's white quiet-zone box inside its slot crop.
pub mod burn_quiet_zone;

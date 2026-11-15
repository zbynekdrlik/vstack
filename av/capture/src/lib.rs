//! The cambox V4L2 capture side (vstack #8 batch 2, from camera-box dev 4ee558aba).
//!
//! `camera-box` re-exports each module at its old path (`camera_box::capture`, ...). Issue numbers
//! in the module docs are camera-box's. The box's own reaction to a bad capture rate (the USB
//! reset, `capture_rate_selfheal`) stays in camera-box; these modules only decide.

// The V4L2 capture device: open, controls, dequeue, the frame types the NDI sender takes. Linux
// only (V4L2).
#[cfg(target_os = "linux")]
pub mod capture;
// The per-stream capture phase tracker: the camera period and phase fitted from the V4L2 sequence
// and timestamp, so the genlock stamp decides on a smoothed capture instant (camera-box #1367, #1482).
pub mod capture_phase;
// Capture-delivery-rate sanity check and the grabber model table (camera-box #656).
pub mod capture_rate_health;
// The blocking V4L2 dequeue stall decision (camera-box #707).
pub mod capture_stall;
// The capture/emit-thread wedge watchdog decision (camera-box #945).
pub mod capture_wedge;

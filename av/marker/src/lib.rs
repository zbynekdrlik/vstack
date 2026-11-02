//! The QPSK audio-marker protocol (moved from camera-box `src/qpsk_marker.rs` and
//! `src/qpsk_marker_scan.rs`, vstack#1). Pure std: the ALSA emitter, the recording decoder and the
//! live dock stay in their apps and call into this crate.
pub mod qpsk_marker;
pub mod qpsk_marker_scan;

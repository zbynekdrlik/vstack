//! The cambox NDI side (vstack #8 batch 2, from camera-box dev 4ee558aba).
//!
//! `camera-box` re-exports each module at its old path (`camera_box::ndi`, ...). Issue numbers in
//! the module docs are camera-box's. The NDI SDK (`libndi.so`) is loaded at run time, never linked.

// The NDI sender and receiver: the SDK FFI, the genlock pacing and stamps, the pixel conversions.
// Linux only (it takes av-capture's V4L2 frame types and reads the interfaces with getifaddrs).
#[cfg(target_os = "linux")]
pub mod ndi;
// The cameraman preview source, resolved from what is on the LAN (camera-box #1362).
pub mod preview_source;
// The NDI sender re-announce trigger on a network change (camera-box #297).
pub mod reannounce;
// The per-camera NDI send stagger inside the genlock slot (camera-box #1242).
pub mod send_stagger;

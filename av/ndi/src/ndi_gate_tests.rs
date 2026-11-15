//! vstack#8 batch 2 tag C: the tests that pin `ndi.rs` for the mutants diff gate.
//!
//! The NDI SDK is loaded at run time, so `NdiLib` is built here over a stand-in function table:
//! plain `extern "C"` functions that record every call in a thread-local log and answer from a
//! thread-local script. `NdiLib` only needs a `Library` to keep alive; the test binary's own
//! handle (`dlopen(NULL)`) serves. That lets the tests drive the real `NdiSender` and
//! `NdiReceiver` code down to the function-pointer calls.

use super::*;
use std::cell::RefCell;
use std::sync::Mutex;
use std::time::{Duration, Instant};

// ---------------------------------------------------------------------------------------------
// The stand-in NDI function table
// ---------------------------------------------------------------------------------------------

/// One frame as `NDIlib_send_send_video_v2` received it, with `line_stride x yres` bytes copied
/// out of `p_data`.
#[derive(Debug, Clone, PartialEq)]
struct Sent {
    sender: usize,
    xres: c_int,
    yres: c_int,
    fourcc: u32,
    frame_rate_n: c_int,
    frame_rate_d: c_int,
    picture_aspect_ratio: f32,
    frame_format_type: c_int,
    timecode: i64,
    line_stride: c_int,
    metadata_null: bool,
    timestamp: i64,
    data: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq)]
enum Call {
    LibDestroy,
    SendCreate {
        name: String,
        groups_null: bool,
        clock_video: bool,
        clock_audio: bool,
    },
    SendDestroy(usize),
    SendVideo(Sent),
    FindCreate {
        show_local: bool,
        groups_null: bool,
        extra_ips_null: bool,
    },
    FindWait(usize, u32),
    FindGet(usize),
    FindDestroy(usize),
    RecvCreate {
        source: String,
        url: Option<String>,
        color_format: c_int,
        bandwidth: c_int,
        allow_video_fields: bool,
        recv_name: String,
    },
    RecvDestroy(usize),
    RecvCapture {
        recv: usize,
        timeout_ms: u32,
        audio_null: bool,
        metadata_null: bool,
    },
    RecvFree {
        recv: usize,
        xres: c_int,
        data: usize,
    },
    RecvMetadata {
        recv: usize,
        length: c_int,
        timecode: i64,
        xml: String,
    },
}

/// What the stand-in `recv_capture_v3` hands back.
#[derive(Clone)]
struct CaptureScript {
    frame_type: c_int,
    xres: c_int,
    yres: c_int,
    fourcc: u32,
    line_stride: c_int,
    timecode: i64,
    /// `None` leaves `p_data` null.
    data: Option<Vec<u8>>,
}

#[derive(Default)]
struct Fake {
    calls: Vec<Call>,
    /// The handle `send_create` returns (0 = null).
    create_result: usize,
    /// How long `send_send_video_v2` blocks.
    send_block: Duration,
    /// The handle `find_create_v2` returns (0 = null).
    find_result: usize,
    /// The finder's sources: name and url, `None` = null pointer.
    sources: Vec<(Option<CString>, Option<CString>)>,
    source_table: Vec<NDIlib_source_t>,
    /// When set, `find_get_current_sources` reports this count with a null array.
    null_sources_with_count: Option<u32>,
    /// The handle `recv_create_v3` returns (0 = null).
    recv_result: usize,
    capture: Option<CaptureScript>,
    capture_buffer: Vec<u8>,
}

thread_local! {
    static FAKE: RefCell<Fake> = RefCell::new(Fake::default());
}

fn with_fake<R>(f: impl FnOnce(&mut Fake) -> R) -> R {
    FAKE.with(|fake| f(&mut fake.borrow_mut()))
}

fn record(call: Call) {
    with_fake(|fake| fake.calls.push(call));
}

fn take_calls() -> Vec<Call> {
    with_fake(|fake| std::mem::take(&mut fake.calls))
}

fn opt_str(p: *const c_char) -> Option<String> {
    (!p.is_null()).then(|| unsafe { CStr::from_ptr(p) }.to_string_lossy().into_owned())
}

unsafe extern "C" fn fake_destroy() {
    record(Call::LibDestroy);
}

unsafe extern "C" fn fake_send_create(settings: *const NDIlib_send_create_t) -> *mut c_void {
    let s = unsafe { &*settings };
    record(Call::SendCreate {
        name: opt_str(s.p_ndi_name).unwrap_or_default(),
        groups_null: s.p_groups.is_null(),
        clock_video: s.clock_video,
        clock_audio: s.clock_audio,
    });
    with_fake(|fake| fake.create_result) as *mut c_void
}

unsafe extern "C" fn fake_send_destroy(sender: *mut c_void) {
    record(Call::SendDestroy(sender as usize));
}

unsafe extern "C" fn fake_send_video(sender: *mut c_void, frame: *const NDIlib_video_frame_v2_t) {
    let f = unsafe { &*frame };
    let len = (f.line_stride_in_bytes.max(0) * f.yres.max(0)) as usize;
    let data = if f.p_data.is_null() {
        Vec::new()
    } else {
        unsafe { std::slice::from_raw_parts(f.p_data, len) }.to_vec()
    };
    record(Call::SendVideo(Sent {
        sender: sender as usize,
        xres: f.xres,
        yres: f.yres,
        fourcc: f.fourcc,
        frame_rate_n: f.frame_rate_n,
        frame_rate_d: f.frame_rate_d,
        picture_aspect_ratio: f.picture_aspect_ratio,
        frame_format_type: f.frame_format_type,
        timecode: f.timecode,
        line_stride: f.line_stride_in_bytes,
        metadata_null: f.p_metadata.is_null(),
        timestamp: f.timestamp,
        data,
    }));
    let block = with_fake(|fake| fake.send_block);
    if !block.is_zero() {
        std::thread::sleep(block);
    }
}

unsafe extern "C" fn fake_find_create(settings: *const NDIlib_find_create_t) -> *mut c_void {
    let s = unsafe { &*settings };
    record(Call::FindCreate {
        show_local: s.show_local_sources,
        groups_null: s.p_groups.is_null(),
        extra_ips_null: s.p_extra_ips.is_null(),
    });
    with_fake(|fake| fake.find_result) as *mut c_void
}

unsafe extern "C" fn fake_find_destroy(finder: *mut c_void) {
    record(Call::FindDestroy(finder as usize));
}

unsafe extern "C" fn fake_find_wait(finder: *mut c_void, timeout_ms: u32) -> bool {
    record(Call::FindWait(finder as usize, timeout_ms));
    true
}

unsafe extern "C" fn fake_find_get(finder: *mut c_void, count: *mut u32) -> *const NDIlib_source_t {
    record(Call::FindGet(finder as usize));
    with_fake(|fake| {
        if let Some(n) = fake.null_sources_with_count {
            unsafe { *count = n };
            return ptr::null();
        }
        let table: Vec<NDIlib_source_t> = fake
            .sources
            .iter()
            .map(|(name, url)| NDIlib_source_t {
                p_ndi_name: name.as_ref().map_or(ptr::null(), |n| n.as_ptr()),
                p_url_address: url.as_ref().map_or(ptr::null(), |u| u.as_ptr()),
            })
            .collect();
        fake.source_table = table;
        unsafe { *count = fake.source_table.len() as u32 };
        fake.source_table.as_ptr()
    })
}

unsafe extern "C" fn fake_recv_create(settings: *const NDIlib_recv_create_v3_t) -> *mut c_void {
    let s = unsafe { &*settings };
    record(Call::RecvCreate {
        source: opt_str(s.source_to_connect_to.p_ndi_name).unwrap_or_default(),
        url: opt_str(s.source_to_connect_to.p_url_address),
        color_format: s.color_format,
        bandwidth: s.bandwidth,
        allow_video_fields: s.allow_video_fields,
        recv_name: opt_str(s.p_ndi_recv_name).unwrap_or_default(),
    });
    with_fake(|fake| fake.recv_result) as *mut c_void
}

unsafe extern "C" fn fake_recv_destroy(recv: *mut c_void) {
    record(Call::RecvDestroy(recv as usize));
}

unsafe extern "C" fn fake_recv_capture(
    recv: *mut c_void,
    video: *mut NDIlib_video_frame_v2_recv_t,
    audio: *mut c_void,
    metadata: *mut c_void,
    timeout_ms: u32,
) -> c_int {
    record(Call::RecvCapture {
        recv: recv as usize,
        timeout_ms,
        audio_null: audio.is_null(),
        metadata_null: metadata.is_null(),
    });
    with_fake(|fake| {
        let Some(script) = fake.capture.clone() else {
            return NDILIB_FRAME_TYPE_NONE;
        };
        let p_data = match script.data {
            Some(bytes) => {
                fake.capture_buffer = bytes;
                fake.capture_buffer.as_mut_ptr()
            }
            None => ptr::null_mut(),
        };
        let v = unsafe { &mut *video };
        v.xres = script.xres;
        v.yres = script.yres;
        v.fourcc = script.fourcc;
        v.line_stride_in_bytes = script.line_stride;
        v.timecode = script.timecode;
        v.p_data = p_data;
        script.frame_type
    })
}

unsafe extern "C" fn fake_recv_free(recv: *mut c_void, video: *const NDIlib_video_frame_v2_recv_t) {
    let v = unsafe { &*video };
    record(Call::RecvFree {
        recv: recv as usize,
        xres: v.xres,
        data: v.p_data as usize,
    });
}

unsafe extern "C" fn fake_recv_metadata(
    recv: *mut c_void,
    frame: *const NDIlib_metadata_frame_t,
) -> bool {
    let f = unsafe { &*frame };
    record(Call::RecvMetadata {
        recv: recv as usize,
        length: f.length,
        timecode: f.timecode,
        xml: opt_str(f.p_data).unwrap_or_default(),
    });
    true
}

/// An `NdiLib` over the stand-in table. Its `Library` is the test binary's own handle.
fn fake_lib() -> NdiLib {
    NdiLib {
        _library: Library::from(libloading::os::unix::Library::this()),
        destroy: fake_destroy,
        send_create: fake_send_create,
        send_destroy: fake_send_destroy,
        send_send_video_v2: fake_send_video,
        send_send_video_async_v2: fake_send_video,
        find_create_v2: fake_find_create,
        find_destroy: fake_find_destroy,
        find_wait_for_sources: fake_find_wait,
        find_get_current_sources: fake_find_get,
        recv_create_v3: fake_recv_create,
        recv_destroy: fake_recv_destroy,
        recv_capture_v3: fake_recv_capture,
        recv_free_video_v2: fake_recv_free,
        recv_send_metadata: fake_recv_metadata,
    }
}

const SENDER: usize = 0x5E0;
const NEW_SENDER: usize = 0x5E1;
const RECEIVER: usize = 0x4EC;
const FINDER: usize = 0xF1D;

fn sig(addrs: &[&str]) -> crate::reannounce::NetworkSignature {
    crate::reannounce::NetworkSignature::from_addrs(addrs.iter().copied())
}

fn fake_sender(numerator: u32, denominator: u32, has_avx2: bool) -> NdiSender {
    NdiSender {
        lib: fake_lib(),
        sender: SENDER as *mut c_void,
        ndi_name: CString::new("CAM-T (usb)").unwrap(),
        frame_rate: FrameRate {
            numerator,
            denominator,
        },
        frame_count: 0,
        reannounce: crate::reannounce::ReannounceState::new(sig(&["10.77.9.62"])),
        last_reannounce_check: Instant::now(),
        uyvy_buffer: Vec::new(),
        has_avx2,
        external_pacing: false,
    }
}

fn fake_receiver(handle: usize) -> NdiReceiver {
    NdiReceiver {
        lib: Arc::new(fake_lib()),
        receiver: handle as *mut c_void,
        source_name: "STRIH (program)".to_string(),
    }
}

fn sent_frames(calls: &[Call]) -> Vec<Sent> {
    calls
        .iter()
        .filter_map(|c| match c {
            Call::SendVideo(s) => Some(s.clone()),
            _ => None,
        })
        .collect()
}

fn fourcc(code: &[u8; 4]) -> v4l::FourCC {
    v4l::FourCC::new(code)
}

// ---------------------------------------------------------------------------------------------
// A tracing subscriber that keeps every event (level + message)
// ---------------------------------------------------------------------------------------------

#[derive(Clone, Default)]
struct Events(Arc<Mutex<Vec<(tracing::Level, String)>>>);

impl Events {
    fn of(&self, level: tracing::Level, needle: &str) -> usize {
        self.0
            .lock()
            .unwrap()
            .iter()
            .filter(|(l, m)| *l == level && m.contains(needle))
            .count()
    }
}

struct MessageVisitor<'a>(&'a mut String);

impl tracing::field::Visit for MessageVisitor<'_> {
    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        if field.name() == "message" {
            use std::fmt::Write;
            write!(self.0, "{value:?}").unwrap();
        }
    }
}

impl tracing::Subscriber for Events {
    fn register_callsite(
        &self,
        _: &'static tracing::Metadata<'static>,
    ) -> tracing::subscriber::Interest {
        tracing::subscriber::Interest::always()
    }
    fn enabled(&self, _: &tracing::Metadata<'_>) -> bool {
        true
    }
    fn new_span(&self, _: &tracing::span::Attributes<'_>) -> tracing::span::Id {
        tracing::span::Id::from_u64(1)
    }
    fn record(&self, _: &tracing::span::Id, _: &tracing::span::Record<'_>) {}
    fn record_follows_from(&self, _: &tracing::span::Id, _: &tracing::span::Id) {}
    fn event(&self, event: &tracing::Event<'_>) {
        let mut message = String::new();
        event.record(&mut MessageVisitor(&mut message));
        self.0
            .lock()
            .unwrap()
            .push((*event.metadata().level(), message));
    }
    fn enter(&self, _: &tracing::span::Id) {}
    fn exit(&self, _: &tracing::span::Id) {}
}

fn capture_events<R>(f: impl FnOnce() -> R) -> (R, Events) {
    let events = Events::default();
    let r = tracing::subscriber::with_default(events.clone(), f);
    (r, events)
}

// ---------------------------------------------------------------------------------------------
// Pure helpers: boundaries, CPU detection, the poll throttle, the getifaddrs walk
// ---------------------------------------------------------------------------------------------

#[test]
fn next_boundary_is_relative_to_the_current_second_not_the_epoch() {
    let second = 5 * UNITS_PER_SECOND;
    assert_eq!(next_boundary_100ns(second + 1, 60), second + 166_666);
    assert_eq!(
        next_boundary_100ns(second + 9_900_000, 60),
        6 * UNITS_PER_SECOND
    );
    assert_eq!(next_boundary_100ns(second + 400_000, 30), second + 666_666);
}

#[test]
fn boundary_timecode_is_the_floor_boundary_of_the_wall_clock() {
    let before = get_wall_clock_100ns();
    let tc = boundary_timecode_100ns(30);
    let after = get_wall_clock_100ns();
    assert!(
        tc >= floor_boundary_100ns(before, 30) && tc <= after,
        "{before} <= {tc} <= {after}"
    );
    assert_eq!(
        tc,
        floor_boundary_100ns(tc, 30),
        "a boundary of the 30 fps grid"
    );
    // fps 0 is the raw clock.
    let before = get_wall_clock_100ns();
    let raw = boundary_timecode_100ns(0);
    assert!(raw >= before && raw <= get_wall_clock_100ns());
}

fn cpuinfo_has_avx2() -> bool {
    std::fs::read_to_string("/proc/cpuinfo")
        .unwrap()
        .lines()
        .filter(|l| l.starts_with("flags"))
        .any(|l| l.split_whitespace().any(|f| f == "avx2"))
}

#[cfg(target_arch = "x86_64")]
#[test]
fn avx2_detection_matches_the_cpu_flags() {
    let expected = cpuinfo_has_avx2();
    assert_eq!(NdiSender::detect_avx2(), expected);
    assert_eq!(has_avx2(), expected);
}

#[test]
fn the_reannounce_poll_is_due_from_the_interval_on() {
    let interval = crate::reannounce::REANNOUNCE_POLL_INTERVAL;
    assert!(!reannounce_poll_due(Duration::ZERO));
    assert!(!reannounce_poll_due(interval - Duration::from_nanos(1)));
    assert!(reannounce_poll_due(interval));
    assert!(reannounce_poll_due(interval + Duration::from_nanos(1)));
}

/// One hand-built `ifaddrs` entry. The vectors own the name and the address.
struct IfEntry {
    name: Option<CString>,
    flags: i32,
    addr: Option<Box<libc::sockaddr_in6>>,
}

fn v4(ip: [u8; 4]) -> Option<Box<libc::sockaddr_in6>> {
    // A sockaddr_in6 is large enough to hold a sockaddr_in; the walk reads it as one.
    let mut storage: Box<libc::sockaddr_in6> = Box::new(unsafe { std::mem::zeroed() });
    let sin = &mut *storage as *mut libc::sockaddr_in6 as *mut libc::sockaddr_in;
    unsafe {
        (*sin).sin_family = libc::AF_INET as libc::sa_family_t;
        (*sin).sin_addr.s_addr = u32::from(std::net::Ipv4Addr::from(ip)).to_be();
    }
    Some(storage)
}

fn v6() -> Option<Box<libc::sockaddr_in6>> {
    let mut storage: Box<libc::sockaddr_in6> = Box::new(unsafe { std::mem::zeroed() });
    storage.sin6_family = libc::AF_INET6 as libc::sa_family_t;
    Some(storage)
}

fn walk(entries: &mut [IfEntry]) -> Vec<String> {
    let mut nodes: Vec<libc::ifaddrs> = entries
        .iter_mut()
        .map(|e| {
            let mut node: libc::ifaddrs = unsafe { std::mem::zeroed() };
            node.ifa_name = e
                .name
                .as_ref()
                .map_or(ptr::null_mut(), |n| n.as_ptr() as *mut _);
            node.ifa_flags = e.flags as libc::c_uint;
            node.ifa_addr = e.addr.as_mut().map_or(ptr::null_mut(), |a| {
                &mut **a as *mut _ as *mut libc::sockaddr
            });
            node
        })
        .collect();
    let base = nodes.as_mut_ptr();
    for i in 0..nodes.len().saturating_sub(1) {
        nodes[i].ifa_next = unsafe { base.add(i + 1) };
    }
    unsafe { usable_ipv4_addrs(if nodes.is_empty() { ptr::null() } else { base }) }
}

#[test]
fn the_ifaddrs_walk_keeps_up_non_loopback_ipv4_lan_addresses_in_order() {
    let name = |n: &str| Some(CString::new(n).unwrap());
    let up = libc::IFF_UP;
    let mut entries = vec![
        IfEntry {
            name: name("enT0"),
            flags: up,
            addr: v4([10, 1, 2, 3]),
        },
        IfEntry {
            name: name("enT1"),
            flags: 0,
            addr: v4([10, 9, 9, 9]),
        },
        IfEntry {
            name: name("enT2"),
            flags: up | libc::IFF_LOOPBACK,
            addr: v4([127, 0, 0, 1]),
        },
        IfEntry {
            name: name("enT3"),
            flags: up,
            addr: v6(),
        },
        IfEntry {
            name: name("enT4"),
            flags: up,
            addr: None,
        },
        IfEntry {
            name: name("docker0"),
            flags: up,
            addr: v4([172, 17, 0, 1]),
        },
        IfEntry {
            name: None,
            flags: up | libc::IFF_RUNNING | libc::IFF_BROADCAST,
            addr: v4([192, 168, 5, 6]),
        },
        IfEntry {
            name: name("enT5"),
            flags: up | libc::IFF_RUNNING,
            addr: v4([10, 4, 5, 6]),
        },
    ];
    assert_eq!(walk(&mut entries), ["10.1.2.3", "192.168.5.6", "10.4.5.6"]);
    assert!(walk(&mut []).is_empty());
}

// ---------------------------------------------------------------------------------------------
// Pixel conversions: hand-computed bytes (BT.601 integer coefficients) and method = free fn
// ---------------------------------------------------------------------------------------------

fn pattern(n: usize, mul: usize, add: usize) -> Vec<u8> {
    (0..n).map(|i| ((i * mul + add) % 256) as u8).collect()
}

/// xorshift bytes, for the wide comparisons.
fn noise(n: usize, seed: u64) -> Vec<u8> {
    let mut x = seed | 1;
    (0..n)
        .map(|_| {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            (x >> 24) as u8
        })
        .collect()
}

const BGRA_5X3: [u8; 36] = [
    106, 63, 147, 190, 125, 98, 91, 159, 162, 132, 138, 64, 162, 64, 138, 166, 106, 73, 147, 201,
    125, 108, 91, 169, 125, 169, 91, 142, 162, 74, 138, 177, 117, 84, 137, 16,
];
/// Blue, green, red, white (alpha ignored).
const BGRA_PRIMARIES: [u8; 16] = [
    0, 0, 255, 255, 0, 255, 0, 255, 255, 0, 0, 255, 255, 255, 255, 0,
];
const UYVY_PRIMARIES: [u8; 8] = [72, 82, 137, 144, 184, 41, 119, 235];

#[test]
fn bgra_to_uyvy_free_fn_gives_the_hand_computed_bytes() {
    // Odd width: each row's last pair takes its second pixel from the next row's first, and the
    // last row's from past the end (black).
    assert_eq!(convert_bgra_to_uyvy(&pattern(60, 37, 11), 5, 3), BGRA_5X3);
    assert_eq!(convert_bgra_to_uyvy(&BGRA_PRIMARIES, 4, 1), UYVY_PRIMARIES);
    // A buffer short of width x height x 4 reads the missing pixels as black.
    assert_eq!(
        convert_bgra_to_uyvy(&pattern(20, 37, 11), 4, 2),
        [106, 63, 147, 190, 125, 98, 91, 159, 117, 132, 137, 16, 128, 16, 128, 16]
    );
}

#[test]
fn bgra_to_uyvy_method_gives_the_hand_computed_bytes() {
    let mut s = fake_sender(30, 1, false);
    s.convert_bgra_to_uyvy(&pattern(60, 37, 11), 5, 3);
    assert_eq!(s.uyvy_buffer, BGRA_5X3);
    s.convert_bgra_to_uyvy(&BGRA_PRIMARIES, 4, 1);
    assert_eq!(s.uyvy_buffer, UYVY_PRIMARIES);
}

#[test]
fn bgra_to_uyvy_method_matches_the_free_fn_on_noise() {
    let mut s = fake_sender(30, 1, false);
    for width in 1..=9 {
        for height in 1..=4 {
            let bgra = noise(width * height * 4, (width * 31 + height) as u64);
            s.convert_bgra_to_uyvy(&bgra, width, height);
            assert_eq!(
                s.uyvy_buffer,
                convert_bgra_to_uyvy(&bgra, width, height),
                "{width}x{height}"
            );
        }
    }
    // Extremes: every channel at 0 and 255 in all combinations, pairwise.
    let mut bgra = Vec::new();
    for c in 0..8u8 {
        for d in 0..8u8 {
            for p in [c, d] {
                bgra.extend([p & 1, (p >> 1) & 1, (p >> 2) & 1, 1].map(|b| b * 255));
            }
        }
    }
    s.convert_bgra_to_uyvy(&bgra, 128, 1);
    assert_eq!(s.uyvy_buffer, convert_bgra_to_uyvy(&bgra, 128, 1));
}

#[test]
fn nv12_to_uyvy_free_fn_gives_the_hand_computed_bytes() {
    let full = pattern(20, 53, 7);
    assert_eq!(
        convert_nv12_to_uyvy(&full, 4, 3),
        [
            131, 7, 184, 60, 237, 113, 34, 166, 131, 219, 184, 16, 237, 69, 34, 122, 87, 175, 140,
            228, 193, 25, 246, 78
        ]
    );
    // Chroma cut short: the missing U/V read as neutral 128.
    assert_eq!(
        convert_nv12_to_uyvy(&full[..14], 4, 3),
        [
            131, 7, 184, 60, 128, 113, 128, 166, 131, 219, 184, 16, 128, 69, 128, 122, 128, 175,
            128, 228, 128, 25, 128, 78
        ]
    );
    // Luma cut short too: the missing Y read as 128, and there is no chroma at all.
    assert_eq!(
        convert_nv12_to_uyvy(&full[..9], 4, 3),
        [
            128, 7, 128, 60, 128, 113, 128, 166, 128, 219, 128, 16, 128, 69, 128, 122, 128, 175,
            128, 128, 128, 128, 128, 128
        ]
    );
    // Exactly the luma plane: no chroma.
    assert_eq!(
        convert_nv12_to_uyvy(&full[..12], 4, 3),
        [
            128, 7, 128, 60, 128, 113, 128, 166, 128, 219, 128, 16, 128, 69, 128, 122, 128, 175,
            128, 228, 128, 25, 128, 78
        ]
    );
    // Odd width: the last pair's second Y is the next row's first, then past the plane (128).
    assert_eq!(
        convert_nv12_to_uyvy(&pattern(15, 53, 7), 5, 2),
        [
            25, 7, 78, 60, 131, 113, 184, 166, 237, 219, 128, 16, 25, 16, 78, 69, 131, 122, 184,
            175, 237, 228, 128, 128
        ]
    );
}

#[test]
fn nv12_to_uyvy_method_gives_the_hand_computed_bytes() {
    let mut s = fake_sender(30, 1, false);
    let full = pattern(20, 53, 7);
    s.convert_nv12_to_uyvy(&full, 4, 3);
    assert_eq!(
        s.uyvy_buffer,
        [
            131, 7, 184, 60, 237, 113, 34, 166, 131, 219, 184, 16, 237, 69, 34, 122, 87, 175, 140,
            228, 193, 25, 246, 78
        ]
    );
    // Four rows over two chroma rows' worth of bytes: rows 2-3 read neutral chroma.
    s.convert_nv12_to_uyvy(&full, 4, 4);
    assert_eq!(
        s.uyvy_buffer,
        [
            87, 7, 140, 60, 193, 113, 246, 166, 87, 219, 140, 16, 193, 69, 246, 122, 128, 175, 128,
            228, 128, 25, 128, 78, 128, 131, 128, 184, 128, 237, 128, 34
        ]
    );
}

#[test]
fn nv12_to_uyvy_method_matches_the_free_fn_on_noise() {
    let mut s = fake_sender(30, 1, false);
    for width in [2usize, 4, 6, 10] {
        for height in 1usize..=5 {
            let nv12 = noise(
                width * height + width * height.div_ceil(2),
                (width * 7 + height) as u64,
            );
            s.convert_nv12_to_uyvy(&nv12, width, height);
            assert_eq!(
                s.uyvy_buffer,
                convert_nv12_to_uyvy(&nv12, width, height),
                "{width}x{height}"
            );
        }
    }
}

#[test]
fn yuyv_to_uyvy_method_swaps_each_pair_on_both_paths() {
    let yuyv = pattern(200, 1, 0); // 50 pairs: 3 AVX2 blocks of 16 pairs, then a scalar tail
    let expected = convert_yuyv_to_uyvy_scalar(&yuyv);
    assert_eq!(&expected[..8], &[1, 0, 3, 2, 5, 4, 7, 6]);
    let mut scalar = fake_sender(30, 1, false);
    scalar.convert_yuyv_to_uyvy(&yuyv);
    assert_eq!(scalar.uyvy_buffer, expected);
    scalar.uyvy_buffer.clear();
    scalar.convert_yuyv_to_uyvy_scalar(&yuyv);
    assert_eq!(scalar.uyvy_buffer, expected);
    if cpuinfo_has_avx2() {
        let mut simd = fake_sender(30, 1, true);
        simd.convert_yuyv_to_uyvy(&yuyv);
        assert_eq!(simd.uyvy_buffer, expected);
    }
}

#[test]
fn the_mjpeg_decoder_output_becomes_the_frame_and_a_failure_is_an_error() {
    let mut s = fake_sender(30, 1, false);
    let mut ok = std::process::Command::new("sh");
    ok.args(["-c", "cat > /dev/null; printf UYVYuyvy"]);
    s.run_mjpeg_decoder(ok, b"\xff\xd8 jpeg").unwrap();
    assert_eq!(s.uyvy_buffer, b"UYVYuyvy");

    let mut failing = std::process::Command::new("sh");
    failing.args(["-c", "cat > /dev/null; printf partial; exit 3"]);
    let err = s.run_mjpeg_decoder(failing, b"\xff\xd8 jpeg").unwrap_err();
    assert_eq!(err.to_string(), "ffmpeg MJPEG decode failed");
    assert_eq!(
        s.uyvy_buffer, b"UYVYuyvy",
        "a failed decode keeps the last frame"
    );
}

#[test]
fn decode_mjpeg_of_garbage_never_reports_a_frame_it_did_not_decode() {
    // With ffmpeg installed the garbage fails to decode (Err, or an empty frame); without it the
    // spawn fails. Either way the old buffer must not come back as a decoded frame.
    let mut s = fake_sender(30, 1, false);
    s.uyvy_buffer = b"stale frame".to_vec();
    let r = s.decode_mjpeg_to_uyvy(b"not a jpeg at all", 4, 2);
    assert!(r.is_err() || s.uyvy_buffer != b"stale frame", "{r:?}");
}

// ---------------------------------------------------------------------------------------------
// NdiSender over the stand-in SDK
// ---------------------------------------------------------------------------------------------

fn expected_sent(width: c_int, height: c_int, stride: c_int, timecode: i64, data: Vec<u8>) -> Sent {
    Sent {
        sender: SENDER,
        xres: width,
        yres: height,
        fourcc: NDILIBD_FOURCC_UYVY,
        frame_rate_n: 30000,
        frame_rate_d: 1001,
        picture_aspect_ratio: 0.0,
        frame_format_type: NDILIB_FRAME_FORMAT_TYPE_PROGRESSIVE,
        timecode,
        line_stride: stride,
        metadata_null: true,
        timestamp: 0,
        data,
    }
}

#[test]
fn each_capture_format_reaches_the_sdk_as_uyvy_with_its_stride() {
    let mut s = fake_sender(30000, 1001, false);
    take_calls();

    // UYVY passes through with the capture stride (padded rows here).
    let uyvy = pattern(20, 3, 1);
    s.send_frame_data_with_timecode(&uyvy, 4, 2, fourcc(b"UYVY"), 10, 111)
        .unwrap();
    // YUYV is swapped into the buffer, stride width x 2.
    let yuyv = pattern(16, 5, 2);
    s.send_frame_data_with_timecode(&yuyv, 4, 2, fourcc(b"YUYV"), 8, 222)
        .unwrap();
    // NV12 converts, stride width x 2.
    let nv12 = pattern(12, 53, 7);
    s.send_frame_data_with_timecode(&nv12, 4, 2, fourcc(b"NV12"), 4, 333)
        .unwrap();
    // The three BGRA-family codes convert, stride width x 2.
    let bgra = pattern(32, 37, 11);
    for (code, tc) in [(b"BGRA", 444), (b"BGR4", 555), (b"RX24", 666)] {
        s.send_frame_data_with_timecode(&bgra, 4, 2, fourcc(code), 16, tc)
            .unwrap();
    }

    let bgra_uyvy = convert_bgra_to_uyvy(&bgra, 4, 2);
    assert_eq!(
        sent_frames(&take_calls()),
        [
            expected_sent(4, 2, 10, 111, uyvy),
            expected_sent(4, 2, 8, 222, convert_yuyv_to_uyvy_scalar(&yuyv)),
            expected_sent(4, 2, 8, 333, convert_nv12_to_uyvy(&nv12, 4, 2)),
            expected_sent(4, 2, 8, 444, bgra_uyvy.clone()),
            expected_sent(4, 2, 8, 555, bgra_uyvy.clone()),
            expected_sent(4, 2, 8, 666, bgra_uyvy),
        ]
    );
    assert_eq!(s.frame_count(), 6);
}

#[test]
fn an_unsupported_format_or_a_null_sender_sends_nothing() {
    let mut s = fake_sender(30000, 1001, false);
    take_calls();
    let err = s
        .send_frame_data_with_timecode(&[0; 8], 4, 1, fourcc(b"GREY"), 4, 1)
        .unwrap_err();
    assert!(
        err.to_string()
            .starts_with("Unsupported video format: GREY"),
        "{err}"
    );
    s.sender = ptr::null_mut();
    s.send_frame_data_with_timecode(&[0; 8], 2, 1, fourcc(b"UYVY"), 4, 1)
        .unwrap();
    assert_eq!(take_calls(), []);
    assert_eq!(s.frame_count(), 0);
}

#[test]
fn the_frame_count_counts_every_send() {
    let mut s = fake_sender(30000, 1001, false);
    assert_eq!(s.frame_count(), 0);
    for n in 1..=3u64 {
        s.send_frame_data_with_timecode(&[0; 4], 2, 1, fourcc(b"UYVY"), 4, 0)
            .unwrap();
        assert_eq!(s.frame_count(), n);
    }
}

#[test]
fn a_send_that_blocks_past_one_and_a_half_intervals_warns() {
    // 1000 fps: a 1 ms interval, and the SDK call blocks 20 ms.
    let mut s = fake_sender(1000, 1, false);
    with_fake(|f| f.send_block = Duration::from_millis(20));
    let (r, events) =
        capture_events(|| s.send_frame_data_with_timecode(&[0; 4], 2, 1, fourcc(b"UYVY"), 4, 0));
    r.unwrap();
    assert_eq!(
        events.of(
            tracing::Level::WARN,
            "#707 NDI blocking send STALL on 'CAM-T (usb)'"
        ),
        1
    );
}

#[test]
fn a_quick_send_or_an_unknown_rate_does_not_warn() {
    // 30 fps: a 33 ms interval, and the SDK call returns at once.
    let mut s = fake_sender(30, 1, false);
    let (r, events) =
        capture_events(|| s.send_frame_data_with_timecode(&[0; 4], 2, 1, fourcc(b"UYVY"), 4, 0));
    r.unwrap();
    assert_eq!(events.of(tracing::Level::WARN, "#707"), 0);

    // A zero denominator is fps 0: no interval, so never a stall, however long the call.
    let mut s = fake_sender(30, 0, false);
    with_fake(|f| f.send_block = Duration::from_millis(20));
    let (r, events) =
        capture_events(|| s.send_frame_data_with_timecode(&[0; 4], 2, 1, fourcc(b"UYVY"), 4, 0));
    r.unwrap();
    assert_eq!(events.of(tracing::Level::WARN, "#707"), 0);
}

#[test]
fn send_frame_sends_the_owned_frame() {
    let mut s = fake_sender(1000, 1, false);
    take_calls();
    let frame = Frame {
        data: pattern(8, 9, 4),
        width: 2,
        height: 1,
        fourcc: fourcc(b"YUYV"),
        stride: 4,
    };
    let before = get_wall_clock_100ns();
    s.send_frame(&frame).unwrap();
    let sent = sent_frames(&take_calls());
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].data, convert_yuyv_to_uyvy_scalar(&frame.data)[..4]);
    // Self-paced: stamped with the next 1000 fps boundary after the call began.
    assert!(sent[0].timecode > before && sent[0].timecode <= get_wall_clock_100ns());
}

#[test]
fn external_pacing_stamps_the_boundary_already_passed_without_waiting() {
    // At 1 fps the self-paced path would sleep to the next whole second; the externally paced
    // path stamps the second it is in. Retry if a second boundary falls inside the call.
    let mut s = fake_sender(1, 1, false);
    s.set_external_pacing(true);
    let mut checked = false;
    for _ in 0..3 {
        take_calls();
        let before = get_wall_clock_100ns();
        s.send_frame_data(&[0; 4], 2, 1, fourcc(b"UYVY"), 4)
            .unwrap();
        let after = get_wall_clock_100ns();
        if before / UNITS_PER_SECOND != after / UNITS_PER_SECOND {
            continue;
        }
        let sent = sent_frames(&take_calls());
        assert_eq!(sent.len(), 1);
        assert_eq!(
            sent[0].timecode,
            before / UNITS_PER_SECOND * UNITS_PER_SECOND
        );
        checked = true;
        break;
    }
    assert!(
        checked,
        "every send crossed a second boundary: it must not wait"
    );
}

#[test]
fn zero_copy_under_external_pacing_stamps_the_capture_timecode() {
    let mut s = fake_sender(30000, 1001, false);
    s.set_external_pacing(true);
    take_calls();
    let info = av_capture::capture::FrameInfo {
        width: 2,
        height: 1,
        fourcc: fourcc(b"UYVY"),
        stride: 4,
        capture_monotonic_100ns: 0,
        dequeue_duration_ms: 0.0,
        sequence: 0,
    };
    s.send_frame_zero_copy(&[9, 8, 7, 6], info, 17_000_000_123)
        .unwrap();
    assert_eq!(
        sent_frames(&take_calls()),
        [expected_sent(2, 1, 4, 17_000_000_123, vec![9, 8, 7, 6])]
    );
}

#[test]
fn dropping_a_sender_destroys_its_handle_then_the_library() {
    let s = fake_sender(30, 1, false);
    take_calls();
    drop(s);
    assert_eq!(take_calls(), [Call::SendDestroy(SENDER), Call::LibDestroy]);

    let mut s = fake_sender(30, 1, false);
    s.sender = ptr::null_mut();
    drop(s);
    assert_eq!(
        take_calls(),
        [Call::LibDestroy],
        "a null sender is never destroyed"
    );

    drop(fake_lib());
    assert_eq!(take_calls(), [Call::LibDestroy]);
}

fn create_call() -> Call {
    Call::SendCreate {
        name: "CAM-T (usb)".to_string(),
        groups_null: true,
        clock_video: false,
        clock_audio: false,
    }
}

#[test]
fn reannounce_now_destroys_then_creates_the_same_name_sender() {
    let mut s = fake_sender(30, 1, false);
    with_fake(|f| f.create_result = NEW_SENDER);
    take_calls();
    let new_net = sig(&["10.77.9.70"]);
    assert!(s.reannounce_now(new_net.clone()).unwrap());
    assert_eq!(take_calls(), [Call::SendDestroy(SENDER), create_call()]);
    assert_eq!(s.sender as usize, NEW_SENDER);
    assert_eq!(s.reannounce.announced(), &new_net);
}

#[test]
fn reannounce_now_reports_a_null_create_and_leaves_the_sender_null() {
    let mut s = fake_sender(30, 1, false);
    with_fake(|f| f.create_result = 0);
    take_calls();
    let err = s.reannounce_now(sig(&["10.77.9.70"])).unwrap_err();
    assert!(
        err.to_string().contains("NDIlib_send_create returned null"),
        "{err}"
    );
    assert_eq!(take_calls(), [Call::SendDestroy(SENDER), create_call()]);
    assert!(s.sender.is_null());
    assert_eq!(
        s.reannounce.announced(),
        &sig(&["10.77.9.62"]),
        "trigger unchanged"
    );
}

fn overdue(s: &mut NdiSender) {
    s.last_reannounce_check = Instant::now()
        .checked_sub(crate::reannounce::REANNOUNCE_POLL_INTERVAL + Duration::from_secs(1))
        .unwrap();
}

#[test]
fn maybe_reannounce_does_not_read_the_network_before_the_interval() {
    let mut s = fake_sender(30, 1, false);
    with_fake(|f| f.create_result = NEW_SENDER);
    take_calls();
    let read = std::cell::Cell::new(false);
    let r = s.maybe_reannounce_with(|| {
        read.set(true);
        sig(&["10.77.9.70"])
    });
    assert!(!r.unwrap());
    assert!(!read.get(), "throttled: the network is not read");
    assert_eq!(take_calls(), []);
    // The real-network wrapper honours the same throttle.
    assert!(!s.maybe_reannounce().unwrap());
    assert_eq!(take_calls(), []);
    assert!(!s.reannounce.saw_down());
}

#[test]
fn maybe_reannounce_does_nothing_on_an_unchanged_network() {
    let mut s = fake_sender(30, 1, false);
    with_fake(|f| f.create_result = NEW_SENDER);
    take_calls();
    overdue(&mut s);
    assert!(!s.maybe_reannounce_with(|| sig(&["10.77.9.62"])).unwrap());
    assert_eq!(take_calls(), []);
    assert!(
        s.last_reannounce_check.elapsed() < Duration::from_secs(1),
        "poll time restamped"
    );
    assert_eq!(s.sender as usize, SENDER);
}

#[test]
fn maybe_reannounce_re_registers_on_a_changed_network() {
    let mut s = fake_sender(30, 1, false);
    with_fake(|f| f.create_result = NEW_SENDER);
    take_calls();
    overdue(&mut s);
    assert!(s.maybe_reannounce_with(|| sig(&["10.77.9.70"])).unwrap());
    assert_eq!(take_calls(), [Call::SendDestroy(SENDER), create_call()]);
    assert_eq!(s.reannounce.announced(), &sig(&["10.77.9.70"]));
}

#[test]
fn maybe_reannounce_marks_a_down_network_and_waits() {
    let mut s = fake_sender(30, 1, false);
    take_calls();
    overdue(&mut s);
    assert!(!s
        .maybe_reannounce_with(crate::reannounce::NetworkSignature::default)
        .unwrap());
    assert_eq!(take_calls(), []);
    assert!(s.reannounce.saw_down());
}

#[test]
fn maybe_reannounce_reads_the_real_network_once_due() {
    // Announced on a documentation address the host cannot have: once due, the real read either
    // finds a LAN address (a change: re-register) or none (down: marked).
    let mut s = fake_sender(30, 1, false);
    s.reannounce = crate::reannounce::ReannounceState::new(sig(&["198.51.100.77"]));
    with_fake(|f| f.create_result = NEW_SENDER);
    overdue(&mut s);
    let r = s.maybe_reannounce().unwrap();
    assert!(
        r || s.reannounce.saw_down(),
        "neither re-registered nor marked down"
    );
}

// ---------------------------------------------------------------------------------------------
// NdiReceiver over the stand-in SDK
// ---------------------------------------------------------------------------------------------

#[test]
fn receiver_getters_metadata_and_drop() {
    let mut r = fake_receiver(RECEIVER);
    assert_eq!(r.source_name(), "STRIH (program)");
    let xml = r#"<ndi_video_codec type="hardware"/>"#;
    r.send_metadata(xml).unwrap();
    assert_eq!(
        take_calls(),
        [Call::RecvMetadata {
            recv: RECEIVER,
            length: xml.len() as c_int,
            timecode: 0,
            xml: xml.to_string(),
        }]
    );
    assert!(r.send_metadata("nul \0 inside").is_err());
    assert_eq!(take_calls(), []);
    drop(r);
    assert_eq!(
        take_calls(),
        [Call::RecvDestroy(RECEIVER), Call::LibDestroy]
    );

    drop(fake_receiver(0));
    assert_eq!(
        take_calls(),
        [Call::LibDestroy],
        "a null receiver is never destroyed"
    );
}

fn capture_call() -> Call {
    Call::RecvCapture {
        recv: RECEIVER,
        timeout_ms: 40,
        audio_null: true,
        metadata_null: true,
    }
}

/// The only test that calls `capture_frame`: its debug-log counter is a process-wide static.
#[test]
fn capture_frame_copies_video_frames_and_logs_the_first_five_then_every_hundredth() {
    let mut r = fake_receiver(RECEIVER);
    let video = |data: Option<Vec<u8>>, yres: c_int| CaptureScript {
        frame_type: NDILIB_FRAME_TYPE_VIDEO,
        xres: 3,
        yres,
        fourcc: NDILIBD_FOURCC_BGRA,
        line_stride: 16,
        timecode: 16_000_000_000_123,
        data,
    };
    let ((), events) = capture_events(|| {
        // 1: nothing arrived.
        assert!(r.capture_frame(40).unwrap().is_none());
        assert_eq!(take_calls(), [capture_call()]);

        // 2: a video frame (3 pixels wide, padded to 16-byte rows) is copied out, then freed.
        let bytes = pattern(32, 7, 3);
        with_fake(|f| f.capture = Some(video(Some(bytes.clone()), 2)));
        let frame = r.capture_frame(40).unwrap().expect("a video frame");
        assert_eq!(
            (frame.width, frame.height, frame.fourcc, frame.stride),
            (3, 2, NDILIBD_FOURCC_BGRA, 16)
        );
        assert_eq!(frame.data, bytes);
        assert_eq!(frame.timecode_100ns, 16_000_000_000_123);
        let calls = take_calls();
        assert_eq!(calls[0], capture_call());
        assert!(
            matches!(calls[1], Call::RecvFree { recv: RECEIVER, xres: 3, data } if data != 0),
            "{calls:?}"
        );
        assert_eq!(calls.len(), 2);

        // 3: a zero-height frame and 4: a frame without data give nothing.
        with_fake(|f| f.capture = Some(video(Some(vec![1; 8]), 0)));
        assert!(r.capture_frame(40).unwrap().is_none());
        with_fake(|f| f.capture = Some(video(None, 2)));
        assert!(r.capture_frame(40).unwrap().is_none());
        assert_eq!(take_calls(), [capture_call(), capture_call()]);

        // 5: a non-video frame type gives nothing.
        with_fake(|f| {
            f.capture = Some(CaptureScript {
                frame_type: NDILIB_FRAME_TYPE_METADATA,
                ..video(Some(vec![1; 32]), 2)
            })
        });
        assert!(r.capture_frame(40).unwrap().is_none());
        assert_eq!(take_calls(), [capture_call()]);

        with_fake(|f| f.capture = None);
        for _ in 6..=100 {
            assert!(r.capture_frame(40).unwrap().is_none());
        }
    });
    // Calls 1-5 and call 100 log their frame type.
    assert_eq!(events.of(tracing::Level::DEBUG, "NDI recv frame_type="), 6);
    assert_eq!(
        events.of(tracing::Level::DEBUG, "NDI recv frame_type=1 "),
        3
    );
}

fn source(name: Option<&str>, url: Option<&str>) -> (Option<CString>, Option<CString>) {
    (
        name.map(|n| CString::new(n).unwrap()),
        url.map(|u| CString::new(u).unwrap()),
    )
}

fn discovered(name: &str, address: Option<&str>) -> crate::preview_source::DiscoveredSource {
    crate::preview_source::DiscoveredSource {
        name: name.to_string(),
        address: address.map(str::to_string),
    }
}

#[test]
fn connect_lists_the_named_sources_and_connects_to_the_exact_pick() {
    with_fake(|f| {
        f.find_result = FINDER;
        f.recv_result = RECEIVER;
        f.sources = vec![
            source(Some("STRIH (program)"), Some("10.77.8.5:5961")),
            source(None, Some("10.77.8.6:5961")),
            source(Some("STRIH (program) 2"), None),
        ];
    });
    let mut passes = Vec::new();
    let r = NdiReceiver::connect_with_lib(Arc::new(fake_lib()), 5, "strih", |sources, elapsed| {
        passes.push((sources.to_vec(), elapsed));
        Some("STRIH (program) 2".to_string())
    })
    .unwrap();
    assert_eq!(
        passes,
        [(
            vec![
                discovered("STRIH (program)", Some("10.77.8.5:5961")),
                discovered("STRIH (program) 2", None),
            ],
            false
        )]
    );
    assert_eq!(r.source_name(), "STRIH (program) 2");
    assert_eq!(r.receiver as usize, RECEIVER);
    assert_eq!(
        take_calls(),
        [
            Call::FindCreate {
                show_local: true,
                groups_null: true,
                extra_ips_null: true
            },
            Call::FindWait(FINDER, 1000),
            Call::FindGet(FINDER),
            Call::RecvCreate {
                source: "STRIH (program) 2".to_string(),
                url: None,
                color_format: NDILIB_RECV_COLOR_FORMAT_BGRX_BGRA,
                bandwidth: NDILIB_RECV_BANDWIDTH_HIGHEST,
                allow_video_fields: false,
                recv_name: "camera-box-display".to_string(),
            },
            Call::FindDestroy(FINDER),
        ]
    );
}

#[test]
fn connect_gives_up_once_the_window_has_elapsed() {
    with_fake(|f| {
        f.find_result = FINDER;
        f.recv_result = RECEIVER;
        f.sources = vec![source(Some("CAM1 (usb)"), None)];
    });
    let mut passes = Vec::new();
    let err =
        NdiReceiver::connect_with_lib(Arc::new(fake_lib()), 0, "strih", |sources, elapsed| {
            passes.push((sources.len(), elapsed));
            assert!(passes.len() <= 3, "the finder never stops: {passes:?}");
            Some("STRIH (program)".to_string()) // not listed
        })
        .err()
        .expect("no match");
    assert_eq!(
        err.to_string(),
        "NDI source 'strih' not found within timeout"
    );
    assert_eq!(passes, [(1, true)]);
    let calls = take_calls();
    assert_eq!(
        &calls[calls.len() - 2..],
        [Call::FindDestroy(FINDER), Call::LibDestroy]
    );
    assert!(!calls.iter().any(|c| matches!(c, Call::RecvCreate { .. })));
}

#[test]
fn connect_ignores_a_null_source_array() {
    with_fake(|f| {
        f.find_result = FINDER;
        f.null_sources_with_count = Some(2);
    });
    let mut passes = Vec::new();
    let r = NdiReceiver::connect_with_lib(Arc::new(fake_lib()), 0, "strih", |sources, elapsed| {
        passes.push((sources.len(), elapsed));
        None
    });
    assert!(r.is_err());
    assert_eq!(passes, [(0, true)]);
}

#[test]
fn connect_fails_cleanly_without_a_finder_or_a_receiver() {
    with_fake(|f| f.find_result = 0);
    let err = NdiReceiver::connect_with_lib(Arc::new(fake_lib()), 5, "strih", |_, _| None)
        .err()
        .unwrap();
    assert_eq!(err.to_string(), "Failed to create NDI finder");
    assert_eq!(take_calls().len(), 2, "find_create, then the library drop");

    with_fake(|f| {
        f.find_result = FINDER;
        f.recv_result = 0;
        f.sources = vec![source(Some("STRIH (program)"), None)];
    });
    let err = NdiReceiver::connect_with_lib(Arc::new(fake_lib()), 5, "strih", |s, _| {
        s.first().map(|s| s.name.clone())
    })
    .err()
    .unwrap();
    assert_eq!(err.to_string(), "Failed to create NDI receiver");
    let calls = take_calls();
    assert_eq!(
        &calls[calls.len() - 2..],
        [Call::FindDestroy(FINDER), Call::LibDestroy]
    );
}

#[test]
fn the_uyvy_stride_is_two_bytes_a_pixel() {
    assert_eq!(uyvy_stride(0), 0);
    assert_eq!(uyvy_stride(3), 6);
    assert_eq!(uyvy_stride(1920), 3840);
}

#[test]
fn a_zero_height_frame_converts_to_nothing() {
    let mut s = fake_sender(30, 1, false);
    s.uyvy_buffer = vec![1, 2, 3];
    s.convert_bgra_to_uyvy(&[], 4, 0);
    assert!(s.uyvy_buffer.is_empty());
    s.uyvy_buffer = vec![1, 2, 3];
    s.convert_nv12_to_uyvy(&[], 4, 0);
    assert!(s.uyvy_buffer.is_empty());
}

#[test]
fn wait_for_next_boundary_sleeps_until_the_boundary_it_returns() {
    // A short sleep that returns a future boundary passes one call only when it began within
    // that sleep of a boundary; 7 ms between calls puts the next start mid-interval at 60 fps.
    for _ in 0..3 {
        let b = wait_for_next_boundary_100ns(60);
        let now = get_wall_clock_100ns();
        assert!(b <= now, "returned {b} before reaching it (now {now})");
        assert_eq!(
            b,
            floor_boundary_100ns(b, 60),
            "a boundary of the 60 fps grid"
        );
        std::thread::sleep(Duration::from_millis(7));
    }
}

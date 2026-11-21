//! vstack#30: on strih-snv the JPEG XS decoders shared every core with the program encoder while
//! OBS started, and one encode took up to 110 ms. `cbjxs_set_decoder_cores` puts a receiver's
//! decoder threads (`jxd-*`) on the cores the caller names; the thread that polls the receiver
//! keeps its own cores.
//!
//! Its own test binary: one process, one test, so every `jxd-*` thread is this test's decoder.
#![cfg(target_os = "linux")]

use camera_box_jxs::jxs_link::{build_slice_datagrams, split_gro, PacketHeader, DATAGRAM_LEN};
use jxs_recv_ffi::*;
use std::ffi::CString;
use std::net::UdpSocket;
use std::time::{Duration, Instant};
use svt_jpegxs_sys::{Encoder, EncoderSettings};

const W: u32 = 640;
const H: u32 = 128;

/// The `Cpus_allowed_list` of this process's thread `tid`.
fn allowed(tid: &str) -> String {
    let status =
        std::fs::read_to_string(format!("/proc/self/task/{tid}/status")).expect("thread status");
    status
        .lines()
        .find_map(|l| l.strip_prefix("Cpus_allowed_list:"))
        .expect("Cpus_allowed_list")
        .trim()
        .to_string()
}

fn own_allowed() -> String {
    let status = std::fs::read_to_string("/proc/thread-self/status").expect("own status");
    status
        .lines()
        .find_map(|l| l.strip_prefix("Cpus_allowed_list:"))
        .expect("Cpus_allowed_list")
        .trim()
        .to_string()
}

/// `(tid, name)` of every `jxd-*` thread of this process.
fn decoder_threads() -> Vec<(String, String)> {
    std::fs::read_dir("/proc/self/task")
        .expect("task list")
        .filter_map(|t| {
            let t = t.ok()?;
            let name = std::fs::read_to_string(t.path().join("comm")).ok()?;
            let name = name.trim().to_string();
            name.starts_with("jxd-")
                .then(|| (t.file_name().to_string_lossy().into_owned(), name))
        })
        .collect()
}

fn send_frame(enc: &mut Encoder, tx: &UdpSocket, port: u16, timecode: u64) {
    let mut packets: Vec<Vec<u8>> = Vec::new();
    enc.encode_frame(|_, b, _| packets.push(b.to_vec()))
        .unwrap();
    let count = packets.len() as u8;
    for (i, pk) in packets.iter().enumerate() {
        let base = PacketHeader {
            camera_id: 5,
            timecode_100ns: timecode,
            slice_index: i as u8,
            slice_count: count,
            offset: 0,
            slice_len: 0,
        };
        let mut buf = Vec::new();
        build_slice_datagrams(base, pk, &mut buf).unwrap();
        for d in split_gro(&buf, Some(DATAGRAM_LEN)) {
            tx.send_to(d, ("127.0.0.1", port)).unwrap();
        }
    }
}

#[test]
fn the_decoder_threads_run_on_the_cores_the_caller_names() {
    let mut err = [0 as std::ffi::c_char; 128];
    let rx = unsafe { cbjxs_open(0, 5, 0, err.as_mut_ptr(), err.len()) };
    assert!(!rx.is_null());
    // the last core this process may use: a real subset whenever the runner has more than one
    let own_before = own_allowed();
    let core = rig_core::affinity::current_thread_cores()
        .and_then(|c| c.last().copied())
        .expect("own cores");
    let list = CString::new(core.to_string()).unwrap();
    assert_eq!(unsafe { cbjxs_set_decoder_cores(rx, list.as_ptr()) }, 0);
    // a malformed list is refused and named; the earlier setting stays
    let bad = CString::new("eight").unwrap();
    assert_eq!(unsafe { cbjxs_set_decoder_cores(rx, bad.as_ptr()) }, -2);
    let n = unsafe { cbjxs_take_error(rx, err.as_mut_ptr(), err.len()) };
    let msg = unsafe { std::ffi::CStr::from_ptr(err.as_ptr()) }.to_string_lossy();
    assert!(n > 0 && msg.contains("not a cpulist"), "{msg}");
    assert_eq!(
        unsafe { cbjxs_set_decoder_cores(std::ptr::null_mut(), list.as_ptr()) },
        -1
    );
    assert!(decoder_threads().is_empty(), "no decoder before a frame");

    let port = unsafe { cbjxs_local_port(rx) };
    let tx = UdpSocket::bind("127.0.0.1:0").unwrap();
    let mut enc = Encoder::open(EncoderSettings::issue_1387(W, H)).unwrap();
    let base: u64 = 17_618_456_000_000_000;
    let mut decoded = 0;
    let mut sent = 0u64;
    let deadline = Instant::now() + Duration::from_secs(15);
    while decoded < 2 && Instant::now() < deadline {
        if sent < 4 {
            send_frame(&mut enc, &tx, port, base + sent * 333_333);
            sent += 1;
        }
        let mut f = CbjxsFrame::default();
        if unsafe { cbjxs_next_frame(rx, 300, &mut f) } == 1 {
            decoded += 1;
        }
    }
    assert_eq!(decoded, 2, "frames decoded");

    let threads = decoder_threads();
    assert!(!threads.is_empty(), "the decoder runs library threads");
    for (tid, name) in &threads {
        assert_eq!(
            allowed(tid),
            core.to_string(),
            "{name} ({tid}) is not on the decoder cores"
        );
    }
    assert_eq!(
        own_allowed(),
        own_before,
        "the polling thread keeps its own cores"
    );
    unsafe { cbjxs_close(rx) };
}

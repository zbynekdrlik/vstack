//! vstack#30: the codec's library threads carry names, so a host's thread list (`ps -L`,
//! `top -H`) shows where the encoder (`jxe-*`) and the decoder (`jxd-*`) run. Unnamed, every one
//! read as its process (`obs`) on strih-snv, and the start-up stall could not be placed.
//!
//! Its own test binary: one process, one test, so the counts are exactly this test's threads.
//! Linux only: the names come from `pthread_setname_np` and are read from `/proc`.
#![cfg(target_os = "linux")]

use std::collections::BTreeMap;
use svt_jpegxs_sys::{Decoder, Encoder, EncoderSettings};

/// The names of this process's threads, counted.
fn thread_names() -> BTreeMap<String, usize> {
    let mut names = BTreeMap::new();
    for task in std::fs::read_dir("/proc/self/task").expect("task list") {
        let comm = task.expect("task").path().join("comm");
        // a thread that ended between the listing and the read has no comm any more
        if let Ok(name) = std::fs::read_to_string(comm) {
            *names.entry(name.trim().to_string()).or_insert(0) += 1;
        }
    }
    names
}

fn count(names: &BTreeMap<String, usize>, name: &str) -> usize {
    names.get(name).copied().unwrap_or(0)
}

#[test]
fn every_codec_thread_names_its_codec_and_stage() {
    let before = thread_names();
    assert!(
        before.keys().all(|n| !n.starts_with("jx")),
        "no codec thread yet: {before:?}"
    );

    let settings = EncoderSettings::issue_1387(256, 64).with_slice_threads(3);
    let mut enc = Encoder::open(settings).expect("encoder open");
    let names = thread_names();
    assert_eq!(count(&names, "jxe-init"), 1, "{names:?}");
    assert_eq!(
        count(&names, "jxe-pack"),
        3,
        "one per slice thread: {names:?}"
    );
    assert_eq!(count(&names, "jxe-final"), 1, "{names:?}");
    assert_eq!(
        count(&names, "jxs-worker"),
        0,
        "every stage is mapped: {names:?}"
    );

    let mut packets: Vec<Vec<u8>> = Vec::new();
    enc.encode_frame(|_, b, _| packets.push(b.to_vec()))
        .expect("encode");
    let dec = Decoder::open(&packets.concat()).expect("decoder open");
    let names = thread_names();
    // the decoder runs in packetization mode (no input stage thread, `DecHandle.c`)
    assert_eq!(count(&names, "jxd-init"), 0, "{names:?}");
    assert!(count(&names, "jxd-work") >= 1, "{names:?}");
    assert_eq!(count(&names, "jxd-final"), 1, "{names:?}");
    assert_eq!(
        count(&names, "jxs-worker"),
        0,
        "every stage is mapped: {names:?}"
    );
    // the encoder's names are its own, never a decoder's
    assert_eq!(count(&names, "jxe-pack"), 3, "{names:?}");
    drop(dec);
    drop(enc);
}

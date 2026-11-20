//! Issue 1387: the clock half of the JPEG XS send pacing. The schedule itself is pure
//! (`crate::jxs_link::pace`); [`super::JxsSocket::send_slice_paced`] asks it for each send's
//! departure and waits here, on the sending thread, with an absolute `CLOCK_MONOTONIC` sleep.

/// `CLOCK_MONOTONIC` in nanoseconds (the pace schedule's clock).
pub fn monotonic_ns() -> u64 {
    let mut ts = libc::timespec {
        tv_sec: 0,
        tv_nsec: 0,
    };
    // SAFETY: `ts` is a valid out-pointer; CLOCK_MONOTONIC always exists on Linux.
    unsafe { libc::clock_gettime(libc::CLOCK_MONOTONIC, &mut ts) };
    ts.tv_sec as u64 * 1_000_000_000 + ts.tv_nsec as u64
}

/// Sleep until `CLOCK_MONOTONIC` reaches `at_ns` (an absolute deadline, so a late wake never
/// shifts the next one; returns at once when it already passed; a signal resumes the wait).
pub fn sleep_until_monotonic_ns(at_ns: u64) {
    let ts = libc::timespec {
        tv_sec: (at_ns / 1_000_000_000) as libc::time_t,
        tv_nsec: (at_ns % 1_000_000_000) as libc::c_long,
    };
    loop {
        // SAFETY: `ts` is a valid timespec; the remainder pointer is unused with TIMER_ABSTIME.
        let rc = unsafe {
            libc::clock_nanosleep(
                libc::CLOCK_MONOTONIC,
                libc::TIMER_ABSTIME,
                &ts,
                std::ptr::null_mut(),
            )
        };
        if rc != libc::EINTR {
            return;
        }
    }
}

/// Timer slack of the calling thread down to 1 ns while it paces: the default 50 us slack would
/// delay most wake-ups of a ~0.2 ms send schedule by up to a quarter of it. Per thread, so only
/// the sending thread changes.
#[cfg_attr(not(feature = "jxs"), allow(dead_code))]
pub(super) fn tighten_timer_slack() {
    // SAFETY: PR_SET_TIMERSLACK takes a plain integer and touches only this thread.
    if unsafe { libc::prctl(libc::PR_SET_TIMERSLACK, 1 as libc::c_ulong) } != 0 {
        tracing::warn!(
            "jxs send: the pacing thread's timer slack stays at its default ({}) -- each paced send may wake up to ~50 us late, the schedule catches it up (issue 1387)",
            std::io::Error::last_os_error()
        );
    }
}

#[cfg(test)]
mod tests {
    use super::super::*;
    use crate::jxs_link::DATAGRAM_LEN;
    use std::net::UdpSocket;
    use std::os::fd::AsRawFd;

    /// Issue 1387: a paced frame over loopback. Timed on the SENDING side with the schedule's own
    /// clock: the schedule anchors at or after `t0` and the last send waits for its departure, so
    /// `t1 - t0 >= last_departure` holds exactly, whatever the runner's load (a descheduled thread
    /// only adds time). The upper bound is generous; the receiver proves every datagram arrived.
    #[test]
    fn a_paced_frame_is_spread_over_its_schedule_and_arrives_whole() {
        use jxs_link::pace::{segments_per_send, send_ranges, wire_bytes, PaceSchedule};
        let rx = UdpSocket::bind("127.0.0.1:0").unwrap();
        // a 1 MB burst would overflow a small default receive buffer
        set_int_opt(rx.as_raw_fd(), libc::SOL_SOCKET, libc::SO_RCVBUF, 4 << 20);
        rx.set_read_timeout(Some(std::time::Duration::from_secs(3)))
            .unwrap();
        let port = rx.local_addr().unwrap().port();
        let tx = JxsSocket::connect_with(&format!("127.0.0.1:{port}"), false).unwrap();
        // 40 slices of a 4 bpp 1080p frame (15360 bytes each)
        let slice: Vec<u8> = (0..15_360u32).map(|i| (i % 251) as u8).collect();
        let bufs: Vec<Vec<u8>> = (0..40u8)
            .map(|i| {
                let base = jxs_link::PacketHeader {
                    camera_id: 100,
                    timecode_100ns: 42,
                    slice_index: i + 1,
                    slice_count: 41,
                    offset: 0,
                    slice_len: 0,
                };
                let mut b = Vec::new();
                jxs_link::build_slice_datagrams(base, &slice, &mut b).unwrap();
                b
            })
            .collect();
        let datagrams: usize = bufs.iter().map(|b| b.len().div_ceil(DATAGRAM_LEN)).sum();
        // the schedule's span: every send ready at once, at 500 Mb/s
        let mut plan = PaceSchedule::new(500).unwrap();
        let mut last_departure = 0;
        for b in &bufs {
            for r in send_ranges(b.len(), segments_per_send(true)) {
                last_departure = plan.depart(0, wire_bytes(r.len()));
            }
        }
        assert!(
            (9_000_000..12_000_000).contains(&last_departure),
            "~10.4 ms of wire at 500 Mb/s: {last_departure} ns"
        );
        let reader = std::thread::spawn(move || {
            let mut d = [0u8; 2048];
            let mut n = 0usize;
            while n < datagrams && rx.recv(&mut d).is_ok() {
                n += 1;
            }
            n
        });
        let mut pace = PaceSchedule::new(500).unwrap();
        let mut paced_ns = 0;
        let t0 = monotonic_ns();
        for b in &bufs {
            let sent = tx.send_slice_paced(b, Some(&mut pace));
            assert_eq!(sent.errors, 0);
            paced_ns += sent.paced_ns;
        }
        let t1 = monotonic_ns();
        assert_eq!(reader.join().unwrap(), datagrams, "every datagram arrived");
        assert!(
            t1 - t0 >= last_departure,
            "{} ns < the schedule's {last_departure} ns: sent faster than the pace",
            t1 - t0
        );
        assert!(t1 - t0 < 500_000_000, "{} ns", t1 - t0);
        // the waits it reports are what held the sends back: at most the whole span
        assert!(
            paced_ns > 0 && paced_ns <= t1 - t0,
            "{paced_ns} of {}",
            t1 - t0
        );
        // line rate: the same frame, no waits, every send at once
        let line = bufs
            .iter()
            .map(|b| tx.send_slice_paced(b, None).paced_ns)
            .sum::<u64>();
        assert_eq!(line, 0);
    }

    /// vstack#27: a paced frame queued whole sends only its first burst at once (the schedule
    /// starts there, the next one is not due yet), and the rest on the schedule afterwards,
    /// spread as `send_slice_paced` spreads it, every datagram arriving.
    #[test]
    fn a_queued_frame_sends_what_is_due_at_once_and_the_rest_on_the_schedule() {
        use jxs_link::pace::{segments_per_send, send_ranges, wire_bytes, PaceSchedule};
        let rx = UdpSocket::bind("127.0.0.1:0").unwrap();
        set_int_opt(rx.as_raw_fd(), libc::SOL_SOCKET, libc::SO_RCVBUF, 4 << 20);
        rx.set_read_timeout(Some(std::time::Duration::from_secs(3)))
            .unwrap();
        let port = rx.local_addr().unwrap().port();
        let tx = JxsSocket::connect_with(&format!("127.0.0.1:{port}"), false).unwrap();
        let slice: Vec<u8> = (0..15_360u32).map(|i| (i % 251) as u8).collect();
        let mut queue = PacedQueue::default();
        let mut bursts = 0;
        let mut datagrams = 0;
        let mut plan = PaceSchedule::new(500).unwrap();
        let mut last_departure = 0;
        for i in 0..40u8 {
            let base = jxs_link::PacketHeader {
                camera_id: 100,
                timecode_100ns: 42,
                slice_index: i + 1,
                slice_count: 41,
                offset: 0,
                slice_len: 0,
            };
            let mut b = Vec::new();
            jxs_link::build_slice_datagrams(base, &slice, &mut b).unwrap();
            for r in send_ranges(b.len(), segments_per_send(true)) {
                last_departure = plan.depart(0, wire_bytes(r.len()));
                bursts += 1;
            }
            datagrams += b.len().div_ceil(DATAGRAM_LEN);
            queue.push(&b);
        }
        assert_eq!(queue.pending(), bursts);
        let reader = std::thread::spawn(move || {
            let mut d = [0u8; 2048];
            let mut n = 0usize;
            while n < datagrams && rx.recv(&mut d).is_ok() {
                n += 1;
            }
            n
        });
        let mut pace = PaceSchedule::new(500).unwrap();
        let t0 = monotonic_ns();
        let now = tx.send_due(&mut queue, &mut pace, t0);
        assert_eq!((now.errors, now.paced_ns), (0, 0), "never waits");
        assert!(now.first.is_some());
        assert_eq!(queue.pending(), bursts - 1, "only the first burst was due");
        // nothing more is due right after it
        let again = tx.send_due(&mut queue, &mut pace, t0);
        assert!(again.first.is_none());
        assert_eq!(queue.pending(), bursts - 1);
        let rest = tx.send_queued_paced(&mut queue, &mut pace);
        let t1 = monotonic_ns();
        assert_eq!(rest.errors, 0);
        assert_eq!(queue.pending(), 0);
        assert!(rest.first.is_some());
        assert!(rest.paced_ns > 0 && rest.paced_ns <= t1 - t0);
        assert!(
            t1 - t0 >= last_departure,
            "{} ns < the schedule's {last_departure} ns: sent faster than the pace",
            t1 - t0
        );
        assert_eq!(reader.join().unwrap(), datagrams, "every datagram arrived");
        // an emptied queue sends nothing
        assert!(tx.send_queued_paced(&mut queue, &mut pace).first.is_none());
        queue.push(&vec![0u8; DATAGRAM_LEN * 20]);
        assert_eq!(queue.pending(), 2, "20 datagrams are bursts of 16 and 4");
        queue.clear();
        assert_eq!(queue.pending(), 0);
        assert!(tx
            .send_due(&mut queue, &mut pace, monotonic_ns())
            .first
            .is_none());
    }

    /// A queued send goes out at the very nanosecond it is due, not one later (vstack#27).
    #[test]
    fn a_queued_send_is_due_at_its_departure_time() {
        use jxs_link::pace::PaceSchedule;
        let rx = UdpSocket::bind("127.0.0.1:0").unwrap();
        let port = rx.local_addr().unwrap().port();
        let tx = JxsSocket::connect_with(&format!("127.0.0.1:{port}"), false).unwrap();
        let mut queue = PacedQueue::default();
        queue.push(&vec![0u8; DATAGRAM_LEN * 32]);
        let mut pace = PaceSchedule::new(500).unwrap();
        assert!(tx.send_due(&mut queue, &mut pace, 1_000).first.is_some());
        let due = pace.due_ns().unwrap();
        assert!(tx.send_due(&mut queue, &mut pace, due - 1).first.is_none());
        assert_eq!(queue.pending(), 1);
        assert!(tx.send_due(&mut queue, &mut pace, due).first.is_some());
        assert_eq!(queue.pending(), 0);
    }

    /// The queued sends count a failed send (a receiver that is gone answers "port unreachable",
    /// which the next send reports), at once and on the schedule alike (vstack#27).
    #[test]
    fn queued_sends_to_a_gone_receiver_count_their_errors() {
        use jxs_link::pace::PaceSchedule;
        let gone = UdpSocket::bind("127.0.0.1:0").unwrap();
        let port = gone.local_addr().unwrap().port();
        drop(gone);
        let tx = JxsSocket::connect_with(&format!("127.0.0.1:{port}"), false).unwrap();
        let mut queue = PacedQueue::default();
        let mut pace = PaceSchedule::new(10_000).unwrap();
        // a refused send is not sent, so the refusals land on every other send: one way at a time.
        // The kernel rate-limits ICMP (parallel tests hit dead ports too): keep sending a while.
        let start = std::time::Instant::now();
        let mut due_errors = 0u64;
        while due_errors == 0 && start.elapsed() < std::time::Duration::from_secs(5) {
            queue.push(&[0x4A; 64]);
            due_errors += tx.send_due(&mut queue, &mut pace, monotonic_ns()).errors;
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        let mut paced_errors = 0u64;
        while paced_errors == 0 && start.elapsed() < std::time::Duration::from_secs(10) {
            queue.push(&[0x4A; 64]);
            paced_errors += tx.send_queued_paced(&mut queue, &mut pace).errors;
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        assert!(
            due_errors >= 1,
            "a refusal reported to a send at once is counted"
        );
        assert!(
            paced_errors >= 1,
            "a refusal reported to a paced send is counted"
        );
    }

    /// A frame's sends fold up: errors and waits add, the first send stays the first.
    #[test]
    fn slice_sends_fold_into_the_frames_sum() {
        let t0 = std::time::Instant::now();
        let t1 = t0 + std::time::Duration::from_millis(1);
        let mut frame = SliceSent::default();
        frame.add(SliceSent::default());
        assert!(frame.first.is_none());
        frame.add(SliceSent {
            errors: 2,
            first: Some(t0),
            paced_ns: 300,
        });
        frame.add(SliceSent {
            errors: 3,
            first: Some(t1),
            paced_ns: 400,
        });
        assert_eq!(
            (frame.errors, frame.paced_ns, frame.first),
            (5, 700, Some(t0))
        );
    }

    /// A paced sender polling the encoder looks again at the next departure, never later than
    /// the poll period, never in the past (vstack#27).
    #[test]
    fn the_next_look_is_the_next_departure_within_the_poll_period() {
        use super::super::socket::PACED_POLL_NS;
        use jxs_link::pace::PaceSchedule;
        let mut queue = PacedQueue::default();
        let mut pace = PaceSchedule::new(500).unwrap();
        // nothing scheduled yet: the poll period
        assert_eq!(queue.next_look_ns(&pace, 1_000), 1_000 + PACED_POLL_NS);
        pace.depart(1_000, 16 * 1538);
        let due = pace.due_ns().unwrap();
        // a send due and nothing queued: the poll period
        assert_eq!(
            queue.next_look_ns(&pace, due - 10_000),
            due - 10_000 + PACED_POLL_NS
        );
        queue.push(&vec![0u8; DATAGRAM_LEN]);
        // due within the period: its departure; past: now; beyond: the period
        assert_eq!(queue.next_look_ns(&pace, due - 10_000), due);
        assert_eq!(queue.next_look_ns(&pace, due + 5), due + 5);
        let early = due - PACED_POLL_NS - 1;
        assert_eq!(queue.next_look_ns(&pace, early), early + PACED_POLL_NS);
    }
}

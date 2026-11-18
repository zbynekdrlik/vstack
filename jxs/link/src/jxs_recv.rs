//! Issue 1387 -- the receive side of the JPEG XS link: a `recvmmsg` + UDP GRO socket with kernel
//! receive timestamps, and (with the `jxs` feature) the slice-wise decoder with concealment that
//! the `jxs-recv-probe` bin and the loopback test use.
//!
//! Loss is concealed, never waited for (owner decision 6489239017): a packet the assembler hands
//! on as lost is replaced by the SAME packet index of the previous frame. JPEG XS slices are coded
//! independently and carry their own index, so the previous frame's bytes decode as that
//! region of the previous picture, and the decoder keeps working packet by packet.

use std::net::{SocketAddr, UdpSocket};
use std::os::fd::AsRawFd;

use crate::jxs_send::{set_int_opt, UDP_GRO};

/// Datagram buffers per `recvmmsg` call.
pub const RECV_BATCH: usize = 32;
/// One GRO buffer (the kernel merges at most 64 KB).
pub const RECV_BUF_LEN: usize = 65_536;
/// Socket receive buffer asked for (the kernel caps it at `net.core.rmem_max`).
pub const RECV_BUFFER_BYTES: i32 = 16 * 1024 * 1024;

/// The current CLOCK_REALTIME in 100 ns since the epoch (the sender timecode's clock).
pub fn realtime_100ns() -> i64 {
    let d = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    (d.as_nanos() / 100) as i64
}

/// A bound receive socket with GRO + kernel timestamps, buffers allocated once.
pub struct JxsRecvSocket {
    sock: UdpSocket,
    bufs: Vec<u8>,
    ctrl: Vec<u8>,
    gro: bool,
    timestamps: bool,
    /// The interface the newest datagram arrived on (`IP_PKTINFO`), 0 = none yet / unknown.
    last_ifindex: u32,
}

const CTRL_LEN: usize = 128;

impl JxsRecvSocket {
    pub fn bind(addr: SocketAddr) -> std::io::Result<Self> {
        let sock = UdpSocket::bind(addr)?;
        let fd = sock.as_raw_fd();
        set_int_opt(fd, libc::SOL_SOCKET, libc::SO_RCVBUF, RECV_BUFFER_BYTES);
        let gro = set_int_opt(fd, libc::SOL_UDP, UDP_GRO, 1);
        let timestamps = set_int_opt(fd, libc::SOL_SOCKET, libc::SO_TIMESTAMPNS, 1);
        // the receiving interface of every datagram: its link speed goes into the report (the
        // sender's pace, issue 1387); refused = the report says "unknown"
        set_int_opt(fd, libc::IPPROTO_IP, libc::IP_PKTINFO, 1);
        // the reports carry the link's DSCP too (issue 1387)
        set_int_opt(
            fd,
            libc::IPPROTO_IP,
            libc::IP_TOS,
            crate::jxs_send::JXS_IP_TOS,
        );
        Ok(Self {
            sock,
            bufs: vec![0; RECV_BATCH * RECV_BUF_LEN],
            ctrl: vec![0; RECV_BATCH * CTRL_LEN],
            gro,
            timestamps,
            last_ifindex: 0,
        })
    }

    /// The interface index the newest datagram arrived on (`None` before the first datagram or
    /// when the kernel named none).
    pub fn last_ifindex(&self) -> Option<u32> {
        (self.last_ifindex > 0).then_some(self.last_ifindex)
    }

    /// The `IP_TOS` byte the report datagrams leave with.
    pub fn tos(&self) -> i64 {
        crate::jxs_send::get_int_opt(self.sock.as_raw_fd(), libc::IPPROTO_IP, libc::IP_TOS)
    }

    pub fn local_addr(&self) -> std::io::Result<SocketAddr> {
        self.sock.local_addr()
    }

    /// Whether UDP GRO and kernel receive timestamps were accepted.
    pub fn features(&self) -> (bool, bool) {
        (self.gro, self.timestamps)
    }

    /// The effective receive buffer (bytes, as the kernel reports it).
    pub fn recv_buffer(&self) -> i64 {
        let mut v: libc::c_int = 0;
        let mut len = std::mem::size_of::<libc::c_int>() as libc::socklen_t;
        // SAFETY: valid out-pointers sized for a c_int.
        let ok = unsafe {
            libc::getsockopt(
                self.sock.as_raw_fd(),
                libc::SOL_SOCKET,
                libc::SO_RCVBUF,
                (&mut v as *mut libc::c_int).cast(),
                &mut len,
            )
        };
        if ok == 0 {
            v as i64
        } else {
            -1
        }
    }

    pub fn set_timeout(&self, t: Option<std::time::Duration>) -> std::io::Result<()> {
        self.sock.set_read_timeout(t)
    }

    /// One `recvmmsg` call (blocking up to the read timeout). `f(datagram, arrival_100ns)` is
    /// called for every datagram, GRO buffers split back into datagrams. `arrival_100ns` is the
    /// kernel receive time (CLOCK_REALTIME) when timestamps are on, else the time the call
    /// returned. Returns the number of datagrams; a timeout returns 0.
    pub fn recv_batch(&mut self, mut f: impl FnMut(&[u8], i64)) -> std::io::Result<usize> {
        self.recv_batch_from(|d, at, _| f(d, at))
    }

    /// [`Self::recv_batch`] that also names each datagram's source address (`None` when the
    /// kernel gave none this can read): the receiver reports back to it (issue 1387 auto bpp).
    pub fn recv_batch_from(
        &mut self,
        mut f: impl FnMut(&[u8], i64, Option<SocketAddr>),
    ) -> std::io::Result<usize> {
        // SAFETY (whole block): every iovec/msghdr points into `self.bufs` / `self.ctrl`, which
        // live for the call and are sized RECV_BATCH x RECV_BUF_LEN / CTRL_LEN; the kernel
        // writes only within the given lengths.
        let mut iov: [libc::iovec; RECV_BATCH] = unsafe { std::mem::zeroed() };
        let mut msgs: [libc::mmsghdr; RECV_BATCH] = unsafe { std::mem::zeroed() };
        let mut names: [libc::sockaddr_storage; RECV_BATCH] = unsafe { std::mem::zeroed() };
        // One base pointer per buffer, offsets with `add`: no re-borrow of the Vec per entry.
        let bufs = self.bufs.as_mut_ptr();
        let ctrl = self.ctrl.as_mut_ptr();
        for i in 0..RECV_BATCH {
            iov[i].iov_base = unsafe { bufs.add(i * RECV_BUF_LEN) }.cast();
            iov[i].iov_len = RECV_BUF_LEN;
            msgs[i].msg_hdr.msg_iov = &mut iov[i];
            msgs[i].msg_hdr.msg_iovlen = 1;
            msgs[i].msg_hdr.msg_control = unsafe { ctrl.add(i * CTRL_LEN) }.cast();
            msgs[i].msg_hdr.msg_controllen = CTRL_LEN as _;
            msgs[i].msg_hdr.msg_name = (&mut names[i] as *mut libc::sockaddr_storage).cast();
            msgs[i].msg_hdr.msg_namelen =
                std::mem::size_of::<libc::sockaddr_storage>() as libc::socklen_t;
        }
        let n = unsafe {
            libc::recvmmsg(
                self.sock.as_raw_fd(),
                msgs.as_mut_ptr(),
                RECV_BATCH as libc::c_uint,
                libc::MSG_WAITFORONE,
                std::ptr::null_mut(),
            )
        };
        let returned = realtime_100ns();
        if n < 0 {
            let e = std::io::Error::last_os_error();
            return match e.kind() {
                std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut => Ok(0),
                _ if e.raw_os_error() == Some(libc::EINTR) => Ok(0),
                _ => Err(e),
            };
        }
        let mut count = 0;
        for (i, m) in msgs.iter().enumerate().take(n as usize) {
            let c = unsafe { parse_cmsgs(&m.msg_hdr) };
            let (gso, arrival) = (c.gso, c.ts.unwrap_or(returned));
            if let Some(i) = c.ifindex {
                self.last_ifindex = i;
            }
            let len = m.msg_len as usize;
            let buf = &self.bufs[i * RECV_BUF_LEN..i * RECV_BUF_LEN + len];
            let from = sockaddr_to_std(&names[i], m.msg_hdr.msg_namelen);
            for d in crate::jxs_link::split_gro(buf, gso) {
                f(d, arrival, from);
                count += 1;
            }
        }
        Ok(count)
    }

    /// Send one receiver report (issue 1387) from the listening socket to `to`, the address the
    /// slices came from: the sender's connected socket takes datagrams only from its peer, which
    /// is this socket.
    pub fn send_report(
        &self,
        to: SocketAddr,
        report: &crate::jxs_link::ReceiverReport,
    ) -> std::io::Result<()> {
        let b = report
            .encode()
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidInput, format!("{e:?}")))?;
        self.sock.send_to(&b, to).map(|_| ())
    }
}

/// A kernel-filled source address as a `SocketAddr` (IPv4 / IPv6; anything else `None`).
fn sockaddr_to_std(ss: &libc::sockaddr_storage, len: libc::socklen_t) -> Option<SocketAddr> {
    use std::net::{Ipv4Addr, Ipv6Addr, SocketAddrV4, SocketAddrV6};
    match ss.ss_family as libc::c_int {
        libc::AF_INET if len as usize >= std::mem::size_of::<libc::sockaddr_in>() => {
            // SAFETY: the family says the storage holds a sockaddr_in, and it is that large.
            let a = unsafe { &*(ss as *const libc::sockaddr_storage).cast::<libc::sockaddr_in>() };
            Some(SocketAddr::V4(SocketAddrV4::new(
                Ipv4Addr::from(u32::from_be(a.sin_addr.s_addr)),
                u16::from_be(a.sin_port),
            )))
        }
        libc::AF_INET6 if len as usize >= std::mem::size_of::<libc::sockaddr_in6>() => {
            // SAFETY: as above, for sockaddr_in6.
            let a = unsafe { &*(ss as *const libc::sockaddr_storage).cast::<libc::sockaddr_in6>() };
            Some(SocketAddr::V6(SocketAddrV6::new(
                Ipv6Addr::from(a.sin6_addr.s6_addr),
                u16::from_be(a.sin6_port),
                a.sin6_flowinfo,
                a.sin6_scope_id,
            )))
        }
        _ => None,
    }
}

/// What a received message's control data said.
#[derive(Debug, Default)]
struct Cmsgs {
    /// The GRO segment size.
    gso: Option<usize>,
    /// The SO_TIMESTAMPNS time (100 ns, CLOCK_REALTIME).
    ts: Option<i64>,
    /// The `IP_PKTINFO` receiving interface.
    ifindex: Option<u32>,
}

/// Read the GRO segment size, the SO_TIMESTAMPNS time and the receiving interface out of a
/// received message's control data.
///
/// # Safety
/// `h` must be a msghdr the kernel just filled (its control pointer and length valid).
unsafe fn parse_cmsgs(h: &libc::msghdr) -> Cmsgs {
    let mut out = Cmsgs::default();
    let mut c = libc::CMSG_FIRSTHDR(h);
    while !c.is_null() {
        let level = (*c).cmsg_level;
        let kind = (*c).cmsg_type;
        let data = libc::CMSG_DATA(c);
        if level == libc::SOL_UDP && kind == UDP_GRO {
            let v = std::ptr::read_unaligned(data.cast::<libc::c_int>());
            if v > 0 {
                out.gso = Some(v as usize);
            }
        } else if level == libc::SOL_SOCKET && kind == libc::SCM_TIMESTAMPNS {
            let t = std::ptr::read_unaligned(data.cast::<libc::timespec>());
            out.ts = Some(t.tv_sec * 10_000_000 + t.tv_nsec / 100);
        } else if level == libc::IPPROTO_IP && kind == libc::IP_PKTINFO {
            let p = std::ptr::read_unaligned(data.cast::<libc::in_pktinfo>());
            if p.ipi_ifindex > 0 {
                out.ifindex = Some(p.ipi_ifindex as u32);
            }
        }
        c = libc::CMSG_NXTHDR(h, c);
    }
    out
}

#[cfg(feature = "jxs")]
pub use decode::{DecodeSink, FINISH_TIMEOUT};

// Issue 1387 slice 2: the one receive loop the probe and the strih OBS source (jxs/recv-ffi) run.
#[cfg(feature = "jxs")]
mod receiver;
#[cfg(feature = "jxs")]
pub use receiver::{
    late_ms_for_latency, DecodedFrame, JxsReceiver, ReceiverConfig, ReceiverTotals,
    MAX_READY_FRAMES,
};

#[cfg(feature = "jxs")]
mod decode {
    use crate::jxs_link::{FrameReport, SliceSink};
    use std::time::Duration;
    use svt_jpegxs_sys::{Decoder, Fed, YuvPlanes};

    /// How long the last packet's decode may take before the frame counts as failed (the N100
    /// measured 0.15 ms after the last slice; a frame the library still waits on never ends).
    pub const FINISH_TIMEOUT: Duration = Duration::from_millis(200);

    /// Decoders kept for picture headers seen before (vstack #20): the auto bpp steps one level at a
    /// time, so the two levels next to the current one cover every swap after the first visit.
    /// Each kept 1080p decoder holds ~13.6 MB and 2 idle library threads (measured 18.11.2026:
    /// 1 -> 5 open decoders, RSS 24 -> 79 MB, threads 7 -> 15), per camera input.
    pub const DECODER_CACHE: usize = 2;

    /// The receiver's [`SliceSink`]: decodes packet by packet, conceals lost packets from the
    /// previous frame, and reports every frame to `on_frame` (`None` planes = not decoded; the
    /// flag = the frame came through a decoder open from the whole frame, so it is late by the
    /// receiver's own init, not by the path: vstack #21).
    ///
    /// A frame FAILS (and the decoder is reopened from the next whole frame) when the library
    /// refuses a packet, lets a frame out before its last packet (its answer to a corrupt slice,
    /// which must never be fed further: every further packet would queue another failed frame
    /// until its pools run dry and `send_packet` blocks for good), or still wants more after the
    /// last packet. No path ever waits on the library without a bound.
    pub struct DecodeSink<F: FnMut(&FrameReport, Option<&YuvPlanes>, bool)> {
        dec: Option<Decoder>,
        /// The last good bytes of every packet index (the concealment source), reused.
        prev: Vec<Vec<u8>>,
        has_prev: Vec<bool>,
        /// The packets of the frame before the decoder is open (its init reads a whole frame).
        pending: Vec<Vec<u8>>,
        pending_ok: bool,
        whole: Vec<u8>,
        count: usize,
        /// The last picture header packet (index 0) that arrived, decoder open or not. A frame
        /// whose header differs (a bpp change: the header carries the codestream length `Lcod`)
        /// forgets the concealment source and takes that header's kept decoder, else opens one from
        /// that frame (issue 1387, vstack #20).
        last_header: Vec<u8>,
        /// The decoders of other picture headers seen before, oldest first (vstack #20): a swap
        /// back to one of them decodes at once, with no open from a whole frame.
        cache: Vec<(Vec<u8>, Decoder)>,
        /// This frame is decoded through an open from the whole frame.
        opened: bool,
        frame_failed: bool,
        on_frame: F,
        /// Frames that could not be decoded.
        pub errors: u64,
        pub last_error: Option<String>,
        /// Picture header changes seen (each one a sender bpp change).
        pub header_changes: u64,
        /// Decoders opened from a whole frame (vstack #20: each one costs that frame's decode
        /// until the whole frame has arrived).
        pub opens: u64,
    }

    impl<F: FnMut(&FrameReport, Option<&YuvPlanes>, bool)> DecodeSink<F> {
        pub fn new(on_frame: F) -> Self {
            Self {
                dec: None,
                prev: Vec::new(),
                has_prev: Vec::new(),
                pending: Vec::new(),
                pending_ok: true,
                whole: Vec::new(),
                count: 0,
                last_header: Vec::new(),
                cache: Vec::new(),
                opened: false,
                frame_failed: false,
                on_frame,
                errors: 0,
                last_error: None,
                header_changes: 0,
                opens: 0,
            }
        }

        fn remember(&mut self, index: usize, bytes: &[u8]) {
            if self.prev.len() <= index {
                self.prev.resize_with(index + 1, Vec::new);
                self.has_prev.resize(index + 1, false);
            }
            self.prev[index].clear();
            self.prev[index].extend_from_slice(bytes);
            self.has_prev[index] = true;
        }

        fn fail(&mut self, e: impl std::fmt::Display) {
            self.frame_failed = true;
            self.last_error = Some(e.to_string());
        }

        /// Feed one packet of an open decoder; marks the frame failed on any sign of trouble.
        fn feed(&mut self, index: usize, bytes: &[u8]) {
            let last = index + 1 == self.count;
            let dec = self.dec.as_mut().expect("open");
            match dec.send_packet(bytes) {
                Err(e) => self.fail(e),
                Ok(Fed::NeedMore) if last => self.fail("the last packet left the frame incomplete"),
                // Before the last packet the library answers NeedMore; OK there means it gave
                // the frame up (a corrupt slice). Deterministic, before the race-prone poll.
                Ok(Fed::Taken) if !last => self.fail("the frame ended before its last packet"),
                Ok(_) if !last => match dec.poll_frame() {
                    Ok(false) => {}
                    Ok(true) => self.fail("a frame came out before its last packet"),
                    Err(e) => self.fail(e),
                },
                Ok(_) => {}
            }
        }

        /// Report a frame that could not be decoded and drop the decoder (it reopens from the
        /// next whole frame; a failed frame may have left it mid-frame).
        fn give_up(&mut self, report: &FrameReport) {
            self.errors += 1;
            self.dec = None;
            (self.on_frame)(report, None, self.opened);
        }

        fn open_from_pending(&mut self, report: &FrameReport) {
            if !self.pending_ok {
                self.errors += 1;
                (self.on_frame)(report, None, self.opened);
                return;
            }
            self.whole.clear();
            for p in &self.pending[..self.count] {
                self.whole.extend_from_slice(p);
            }
            match Decoder::open(&self.whole) {
                Ok(d) => {
                    self.dec = Some(d);
                    self.opens += 1;
                    self.opened = true;
                }
                Err(e) => {
                    self.last_error = Some(e.to_string());
                    self.errors += 1;
                    (self.on_frame)(report, None, self.opened);
                    return;
                }
            }
            self.frame_failed = false;
            for i in 0..self.count {
                let p = std::mem::take(&mut self.pending[i]);
                if !self.frame_failed {
                    self.feed(i, &p);
                }
                self.remember(i, &p);
                self.pending[i] = p;
            }
            self.finish(report);
        }

        fn finish(&mut self, report: &FrameReport) {
            if self.frame_failed {
                self.give_up(report);
                return;
            }
            let dec = self.dec.as_mut().expect("open");
            match dec.finish_frame(FINISH_TIMEOUT) {
                Ok(planes) => (self.on_frame)(report, Some(planes), self.opened),
                Err(e) => {
                    self.last_error = Some(e.to_string());
                    self.give_up(report);
                }
            }
        }
    }

    impl<F: FnMut(&FrameReport, Option<&YuvPlanes>, bool)> SliceSink for DecodeSink<F> {
        fn packet(&mut self, _tc: u64, index: u8, count: u8, bytes: Option<&[u8]>) {
            let index = index as usize;
            if index == 0 {
                self.frame_failed = false;
                self.opened = false;
                if count as usize != self.count {
                    // A new geometry (or the first frame): start the decoder over.
                    self.count = count as usize;
                    self.dec = None;
                    self.cache.clear();
                    self.last_header.clear();
                    self.has_prev.iter_mut().for_each(|h| *h = false);
                }
                if let Some(b) = bytes {
                    if b != self.last_header.as_slice() {
                        if !self.last_header.is_empty() {
                            // A new picture header: the sender changed its bpp (issue 1387;
                            // the packet count stays). An open decoder sized its frame from the
                            // old header's codestream length and would end this frame early (or
                            // wait past it): start over from this whole frame like a new
                            // geometry. Checked whether a decoder is open or not (one may have
                            // been given up just before), the old bpp's slices have other sizes
                            // and are never a concealment source for the new one.
                            self.header_changes += 1;
                            self.has_prev.iter_mut().for_each(|h| *h = false);
                            // vstack #20: keep the old header's decoder for a swap back, and take
                            // this header's own if it was seen before (no open from a whole frame,
                            // which made the swap's frame late and cost the strih's genlock)
                            if let Some(d) = self.dec.take() {
                                if self.cache.len() == DECODER_CACHE {
                                    self.cache.remove(0);
                                }
                                self.cache.push((std::mem::take(&mut self.last_header), d));
                            }
                            // (a reused decoder starts with no concealment source: a loss in its
                            // first frame gives it up and the next frame opens anew, as before)
                            if let Some(i) = self.cache.iter().position(|(hd, _)| hd == b) {
                                self.dec = Some(self.cache.remove(i).1);
                            }
                        }
                        self.last_header.clear();
                        self.last_header.extend_from_slice(b);
                    }
                }
                if self.dec.is_none() {
                    self.pending.resize_with(self.count, Vec::new);
                    self.pending_ok = true;
                }
            }
            if index >= self.count {
                self.frame_failed = true;
                return;
            }
            if self.dec.is_none() {
                // Collect the whole first frame for the decoder's init.
                match bytes {
                    Some(b) => {
                        self.pending[index].clear();
                        self.pending[index].extend_from_slice(b);
                    }
                    // A lost packet of a frame the decoder opens from: conceal it from
                    // the previous frame when that is known (the same packet count).
                    None if self.has_prev.get(index).copied().unwrap_or(false) => {
                        let (prev, pending) = (&self.prev[index], &mut self.pending[index]);
                        pending.clear();
                        pending.extend_from_slice(prev);
                    }
                    None => self.pending_ok = false,
                }
                return;
            }
            match bytes {
                Some(b) => {
                    if !self.frame_failed {
                        self.feed(index, b);
                    }
                    // Only a packet the decoder took cleanly is a concealment source (the slice
                    // that broke a frame must never be replayed for a later loss).
                    if !self.frame_failed {
                        self.remember(index, b);
                    }
                }
                None if self.frame_failed => {}
                None if self.has_prev.get(index).copied().unwrap_or(false) => {
                    // Conceal: the same packet of the previous frame.
                    let concealed = std::mem::take(&mut self.prev[index]);
                    self.feed(index, &concealed);
                    self.prev[index] = concealed;
                }
                None => self.fail("a lost packet with no previous frame to conceal it from"),
            }
        }

        fn frame_done(&mut self, report: &FrameReport) {
            if self.dec.is_none() {
                self.open_from_pending(report);
            } else {
                self.finish(report);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::jxs_link::{self, PacketHeader, DATAGRAM_LEN};
    use crate::jxs_send::JxsSocket;

    /// vstack #20 (camera-box#1536, PP 18.11.2026): the auto bpp swaps 4 <-> 5 every few minutes;
    /// each swap reopened the decoder from a whole frame and cost frames on the strih's genlock.
    /// A swap back to a bpp the receiver has already decoded must reuse its decoder: frames at
    /// 4, then 5, then 4 again open two decoders, not three, and every frame decodes.
    /// Frames through a [`DecodeSink`]: (decoded, came through an open) per frame, and the sink's
    /// counters. `frames` = the packets of each frame (`None` = lost).
    #[cfg(feature = "jxs")]
    fn run_sink(frames: &[Vec<Option<Vec<u8>>>]) -> (Vec<(bool, bool)>, u64, u64) {
        use crate::jxs_link::{FrameReport, SliceSink};
        let mut out = Vec::new();
        let mut sink = DecodeSink::new(
            |_: &FrameReport, p: Option<&svt_jpegxs_sys::YuvPlanes>, opened: bool| {
                out.push((p.is_some(), opened))
            },
        );
        for (k, f) in frames.iter().enumerate() {
            let tc = 333_333 * (k as u64 + 1);
            let n = f.len() as u8;
            for (i, p) in f.iter().enumerate() {
                sink.packet(tc, i as u8, n, p.as_deref());
            }
            sink.frame_done(&FrameReport {
                timecode: tc,
                camera_id: 1,
                slice_count: n,
                lost_slices: f.iter().filter(|p| p.is_none()).count() as u32,
                first_arrival: 0,
                last_arrival: 0,
            });
        }
        let (errors, opens) = (sink.errors, sink.opens);
        drop(sink);
        (out, errors, opens)
    }

    #[cfg(feature = "jxs")]
    fn encoded_frame() -> Vec<Option<Vec<u8>>> {
        use svt_jpegxs_sys::{Encoder, EncoderSettings};
        let mut enc = Encoder::open(EncoderSettings::issue_1387(256, 64)).expect("encoder");
        let mut packets = Vec::new();
        enc.encode_frame(|_, b, _| packets.push(Some(b.to_vec())))
            .expect("encode");
        packets
    }

    #[cfg(feature = "jxs")]
    #[test]
    fn a_first_frame_that_cannot_open_is_an_error_and_the_next_whole_one_opens() {
        let good = encoded_frame();
        // a lost packet with no previous frame to conceal it from: nothing to open from
        let mut lossy = good.clone();
        lossy[2] = None;
        let (out, errors, opens) = run_sink(&[lossy, good.clone()]);
        assert_eq!(out, [(false, false), (true, true)]);
        assert_eq!((errors, opens), (1, 1));
        // a frame the library refuses to open from
        let garbage: Vec<Option<Vec<u8>>> = good
            .iter()
            .map(|p| p.as_ref().map(|b| vec![0x5a; b.len()]))
            .collect();
        let (out, errors, opens) = run_sink(&[garbage, good]);
        assert_eq!(out, [(false, false), (true, true)]);
        assert_eq!((errors, opens), (1, 1));
    }

    #[cfg(feature = "jxs")]
    #[test]
    fn a_corrupt_slice_gives_the_decoder_up_and_the_next_frame_reopens() {
        let good = encoded_frame();
        let mut bad = good.clone();
        let half = bad[2].as_ref().unwrap().len() / 2;
        bad[2].as_mut().unwrap().truncate(half);
        let (out, errors, opens) = run_sink(&[good.clone(), bad, good]);
        assert_eq!(out, [(true, true), (false, false), (true, true)]);
        assert_eq!((errors, opens), (1, 2));
    }

    #[cfg(feature = "jxs")]
    #[test]
    fn a_swap_back_to_a_bpp_already_seen_reuses_its_decoder() {
        use crate::jxs_link::{FrameReport, SliceSink};
        use svt_jpegxs_sys::{Encoder, EncoderSettings};
        let (w, h) = (256u32, 64u32);
        let enc_at = |bpp| {
            Encoder::open(EncoderSettings {
                bpp_numerator: bpp,
                ..EncoderSettings::issue_1387(w, h)
            })
            .expect("encoder")
        };
        let (mut e4, mut e5) = (enc_at(4), enc_at(5));
        let mut decoded = Vec::new();
        let mut sink = DecodeSink::new(
            |r: &FrameReport, p: Option<&svt_jpegxs_sys::YuvPlanes>, opened: bool| {
                decoded.push((r.timecode, p.is_some(), opened))
            },
        );
        let mut tc = 0u64;
        for bpp in [4, 4, 4, 5, 5, 5, 4, 4] {
            let enc = if bpp == 4 { &mut e4 } else { &mut e5 };
            tc += 333_333;
            let mut packets: Vec<Vec<u8>> = Vec::new();
            enc.encode_frame(|_, b, _| packets.push(b.to_vec()))
                .expect("encode");
            let n = packets.len() as u8;
            for (i, p) in packets.iter().enumerate() {
                sink.packet(tc, i as u8, n, Some(p));
            }
            sink.frame_done(&FrameReport {
                timecode: tc,
                camera_id: 1,
                slice_count: n,
                lost_slices: 0,
                first_arrival: 0,
                last_arrival: 0,
            });
        }
        assert_eq!(sink.header_changes, 2, "4 -> 5 -> 4");
        assert_eq!(sink.opens, 2, "the swap back to 4 reuses the 4 bpp decoder");
        assert_eq!(sink.errors, 0);
        drop(sink);
        assert_eq!(decoded.len(), 8);
        assert!(decoded.iter().all(|(_, ok, _)| *ok), "{decoded:?}");
        // vstack #21: the frames that came through an open are flagged (the first 4 bpp frame and
        // the first 5 bpp frame), the swap back to 4 is not
        let opened: Vec<bool> = decoded.iter().map(|(_, _, o)| *o).collect();
        assert_eq!(
            opened,
            [true, false, false, true, false, false, false, false]
        );
    }

    #[test]
    fn recvmmsg_with_gro_returns_every_datagram_of_a_gso_slice() {
        let mut rx = JxsRecvSocket::bind("127.0.0.1:0".parse().unwrap()).unwrap();
        rx.set_timeout(Some(std::time::Duration::from_millis(500)))
            .unwrap();
        assert_eq!(
            rx.features(),
            (true, true),
            "GRO + kernel timestamps accepted"
        );
        let port = rx.local_addr().unwrap().port();
        let tx = JxsSocket::connect(&format!("127.0.0.1:{port}")).unwrap();
        let slice: Vec<u8> = (0..30_000u32).map(|i| (i * 7 % 256) as u8).collect();
        let mut buf = Vec::new();
        let base = PacketHeader {
            camera_id: 9,
            timecode_100ns: 1_234_567,
            slice_index: 4,
            slice_count: 9,
            offset: 0,
            slice_len: 0,
        };
        let n = jxs_link::build_slice_datagrams(base, &slice, &mut buf).unwrap();
        let before = realtime_100ns();
        assert_eq!(tx.send_slice(&buf), 0);
        let mut rebuilt = vec![0u8; slice.len()];
        let mut got = 0;
        while got < n {
            let k = rx
                .recv_batch(|d, at| {
                    assert!(d.len() <= DATAGRAM_LEN);
                    assert!(at >= before - 10_000_000, "arrival on the wall clock");
                    let h = PacketHeader::decode(d).unwrap();
                    assert_eq!((h.camera_id, h.slice_index), (9, 4));
                    let o = h.offset as usize;
                    rebuilt[o..o + d.len() - jxs_link::HEADER_LEN]
                        .copy_from_slice(&d[jxs_link::HEADER_LEN..]);
                })
                .unwrap();
            assert!(k > 0, "timed out after {got} of {n}");
            got += k;
        }
        assert_eq!(got, n);
        assert_eq!(rebuilt, slice);
        // the receiving interface is named (issue 1387: its link speed goes into the report)
        assert_eq!(
            rx.last_ifindex(),
            Some(1),
            "127.0.0.1 arrives on the loopback, ifindex 1"
        );
        // the reports leave with the link's DSCP (AF41)
        assert_eq!(rx.tos(), crate::jxs_send::JXS_IP_TOS as i64);
        // and so do the slices
        assert_eq!(tx.tos(), crate::jxs_send::JXS_IP_TOS as i64);
    }
}

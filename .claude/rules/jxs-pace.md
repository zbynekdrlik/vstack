---
paths:
  - "jxs/link/src/jxs_link/pace_auto.rs"
  - "jxs/link/src/jxs_link/pace.rs"
  - "jxs/link/src/jxs_congestion.rs"
  - "jxs/link/src/jxs_congestion_bench.rs"
  - "jxs/link/src/jxs_send/pace_control.rs"
  - "jxs/link/src/jxs_send/auto_tests.rs"
  - "jxs/link/src/jxs_recv.rs"
  - "jxs/link/src/jxs_recv/receiver.rs"
---

# JPEG XS send pacing

- **Who is paced.** Cameras (1 GbE into the 10 GbE strih) run at line rate and need low latency. The only paced path is the strih program output to a stream box, a receiver slower than the sender. The owner ruled on 18.11.2026 that this path has no low-latency requirement, so it uses the frame spread (`pace_auto` rule 2, #18). Never burst it at a share of the receiver's link: 90 % of 2.5 GbE overflowed a USB RTL8156B receiver NIC and cost its Dante audio (camera-box#1534).
- **Fix an overflow at the pace source, not with a learning heuristic.** #17 tried a learnt congestion ceiling. Six review rounds each broke it, because timing cannot tell a download sawing at the top from a NIC overflowing there. If an overflow ever needs learning again, first check whether the pace rule itself bursts too hard.
- **The bench reproduces live faults.** Model a new path fault as a `jxs_congestion_bench` scenario. Show the old pace failing (`pace_setting_mbps` replays it as a fixed pace; a fixed pace gets no congestion factor, so it is the worst case), then show the new one loss-free.
- **The sawtooth gate counts only congestion pace changes** (`congestion_pace_changes_after_30s`). A frame-spread pace also changes with every bpp step, and that is not a sawtooth.
- `jxs_send::auto_tests` and the real packet-thread tests run only with `--all-features`. Plain `cargo test -p camera-box-jxs --lib` skips them.
- **A new `PaceSource` needs four changes:** the C code in `jxs/send-ffi/src/lib.rs` (`pace_source_code`), its `#define` in `include/camera_box_jxs_send.h`, its pin in `send-ffi/tests/abi.rs`, and the `pace_text` and header doc wording. Tell obs, because its DistroAV output reads the codes.
- **The program spread is 80 % of the frame interval** (`program_spread_mbps`, 1.25x the frame's rate, #23, owner 19.11.2026: "zero loss frames/samples/no frame copies"; the production boxes carry no other traffic). The 20 % left over is the sender's slack: at ~91 % (`frame_need_mbps`) only ~3 ms remained, and a schedule slip turns into `dropped_busy` behind the two-frame in-flight limit. A thinner spread costs resilience to a download on the stream PC: the bench's download scenarios adapt in ~25 s with ~66-135 lossy frames, against ~1 at half the interval. That is accepted, because the production boxes run nothing else. Do not widen the spread back without the owner. To sweep a spread, temporarily replace rule 2's rate and read `lossy_frames_download` / `last_lossy_s`.
- **The receiver keeps the decoders of the last `DECODER_CACHE` (2) bpp levels** (#20): a bpp swap must never reopen. Each kept 1080p decoder costs ~13.6 MB and 2 threads. A frame decoded through an open is never reported late (#21); otherwise the sender's auto bpp steps straight back on `receiver-late`.
- **Codec-only tests in `jxs_recv.rs` need `#[cfg(feature = "jxs")]`.** CI's no-codec test and clippy jobs compile the crate without it.


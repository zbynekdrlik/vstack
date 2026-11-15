//! vstack#8 batch 2 tag B: the boundary and counter cases the camera-box parity tests left to the
//! mutation gate. Each test pins one exact decision of the tracker, the slot chooser, the stamp
//! driver or the source fit.

use super::source_phase::SOURCE_FIT_MAX_POINTS;
use super::*;

/// An exact integer frame period, so the fitted line is exact and every residual is known.
const P: u64 = 16_000_000;
/// A plausible CLOCK_MONOTONIC origin (~ 3.5 days of uptime).
const T0: u64 = 300_000 * NS_PER_SECOND;
/// The production emit interval (60 fps).
const I60: u64 = NS_PER_SECOND / 60;

/// A tracker fed frames `seq = 1000 + k` at `T0 + k * P + resid[k]`.
fn tracker_from(resid: &[i64]) -> CapturePhaseTracker {
    let mut t = CapturePhaseTracker::new();
    for (k, r) in resid.iter().enumerate() {
        t.observe(
            1000 + k as u32,
            (T0 + k as u64 * P).saturating_add_signed(*r),
        );
    }
    t
}

/// `n` frames exactly on the line (fit RMS 0, outlier bound RESEED_FLOOR_NS).
fn exact_tracker(n: usize) -> CapturePhaseTracker {
    tracker_from(&vec![0; n])
}

/// Frame `k` of an exact 60 fps camera starting at `start`.
fn ts60(start: u64, k: u64) -> u64 {
    start + (k as f64 * NS_PER_SECOND as f64 / 60.0).round() as u64
}

/// A stamp driver fed `n` frames of an exact 60 fps camera from `start` (locked after 120).
fn driven_phase(start: u64, n: u64, offset: i64) -> CapturePhase {
    let mut c = CapturePhase::new();
    for k in 0..n {
        c.stamp_frame(1000 + k as u32, ts60(start, k), offset, I60);
    }
    c
}

#[test]
fn div_round_rounds_to_nearest() {
    assert_eq!(div_round(7, 2), 4);
    assert_eq!(div_round(5, 1), 5);
    assert_eq!(div_round(-7, 2), -3);
    assert_eq!(div_round(10, 4), 3);
}

#[test]
fn run_shape_tolerance_scales_with_the_fit_rms() {
    // rms 200 us: tol = 4 x rms = 800 us, so a 700 us rise is ONE plateau (median 1.35 ms).
    let mut run = vec![1_000_000i64; 10];
    run.extend([1_700_000i64; 10]);
    assert_eq!(
        run_shape(&run, 200_000.0),
        RunShape::Step {
            step_ns: 1_350_000,
            last_from: 0
        }
    );
}

#[test]
fn run_shape_splits_only_strictly_past_the_tolerance() {
    // A clean fit: tol = RESEED_FLOOR_NS / 2 = 500 us. A rise of exactly 500 us stays one plateau.
    let mut run = vec![1_000_000i64; 10];
    run.extend([1_500_000i64; 10]);
    assert_eq!(
        run_shape(&run, 0.0),
        RunShape::Step {
            step_ns: 1_250_000,
            last_from: 0
        }
    );
}

#[test]
fn run_shape_accepts_a_negative_staircase() {
    // Earlier by 1.0 ms, then by 2.5 ms: one way, so a step onto the last plateau.
    let mut run = vec![-1_000_000i64; 10];
    run.extend([-2_500_000i64; 10]);
    assert_eq!(
        run_shape(&run, 0.0),
        RunShape::Step {
            step_ns: -2_500_000,
            last_from: 10
        }
    );
}

#[test]
fn run_shape_last_plateau_needs_four_frames() {
    let mut run = vec![1_000_000i64; 10];
    run.extend([2_000_000i64; 2]);
    assert_eq!(run_shape(&run, 0.0), RunShape::Forming);
    run.extend([2_000_000i64; 2]);
    assert_eq!(
        run_shape(&run, 0.0),
        RunShape::Step {
            step_ns: 2_000_000,
            last_from: 10
        }
    );
}

#[test]
fn a_late_third_plateau_is_held_past_32_outliers_then_absorbed() {
    // Plateaus at +1.5 ms (run index 0..27), +3.0 ms (27..30), +4.5 ms (30..): Forming from run
    // length 30 to 33, a step at 34 (MAX_STAIRCASE_RUN = 60 is far away).
    let mut t = exact_tracker(40);
    assert_eq!(t.jitter_rms_ns(), Some(0.0));
    for i in 0..34u64 {
        let resid = match i {
            0..=26 => 1_500_000,
            27..=29 => 3_000_000,
            _ => 4_500_000,
        };
        let k = 40 + i;
        t.observe(1000 + k as u32, T0 + k * P + resid);
        if i < 33 {
            assert_eq!(t.note(), None, "outlier {i}");
        }
    }
    assert_eq!(t.reseeds(), 0);
    assert_eq!(t.phase_steps(), 1);
    assert_eq!(t.note(), Some(&PhaseNote::PhaseStep { step_ns: 4_500_000 }));
}

#[test]
fn describe_a_sequence_step_of_exactly_max_advance_is_a_gap() {
    let note = PhaseNote::Discontinuity {
        seq_step: MAX_SEQ_ADVANCE as i32,
        dt_ns: 5_000,
    };
    assert_eq!(note.describe(), "re-seed: reason=gap seq_step=8 dt_us=5");
}

#[test]
fn a_gap_of_exactly_the_longest_frame_period_is_not_a_pause() {
    let mut t = CapturePhaseTracker::new();
    t.observe(1000, T0);
    let obs = t.observe(1001, T0 + 2 * MAX_FRAME_PERIOD_NS);
    assert_eq!(obs.seq_advance, 1);
    assert_eq!(t.note(), None);
    assert_eq!(t.reseeds(), 0);
}

#[test]
fn a_residual_of_exactly_the_outlier_bound_is_folded() {
    let mut t = exact_tracker(40);
    assert_eq!(t.jitter_rms_ns(), Some(0.0));
    t.observe(1040, T0 + 40 * P + RESEED_FLOOR_NS);
    assert_eq!(t.note(), None);
    // Folded: the fit is no longer exact.
    assert!(t.jitter_rms_ns().unwrap() > 0.0);
}

#[test]
fn a_hidden_drop_beyond_the_sequence_cap_reseeds() {
    // One sequence step, eight whole periods late: 1 + 8 > MAX_SEQ_ADVANCE, so not a re-index.
    let mut t = exact_tracker(40);
    t.observe(1040, T0 + 48 * P);
    assert_eq!(
        t.note(),
        Some(&PhaseNote::HalfPeriod {
            resid_ns: 8 * P as i64
        })
    );
    assert_eq!(t.hidden_drops(), 0);
    assert_eq!(t.reseeds(), 1);
}

#[test]
fn period_and_jitter_need_three_samples() {
    let mut t = CapturePhaseTracker::new();
    t.observe(1000, T0);
    t.observe(1001, T0 + P);
    assert_eq!(t.period_ns(), None);
    assert_eq!(t.jitter_rms_ns(), None);
    t.observe(1002, T0 + 2 * P);
    assert_eq!(t.period_ns(), Some(P as f64));
    assert_eq!(t.jitter_rms_ns(), Some(0.0));
}

#[test]
fn four_samples_on_the_line_are_white() {
    assert!(exact_tracker(4).residuals_are_white());
}

/// 16 samples: a smooth bowl of amplitude `a` (quarter means far off the line, local jitter
/// small) plus +-20 us alternating jitter (the local estimate).
fn bowl(a: f64) -> CapturePhaseTracker {
    let resid: Vec<i64> = (0..16)
        .map(|k| {
            let s = ((k as f64 - 7.5).powi(2) - 21.25) / 35.0;
            let j = if k % 2 == 0 { 20_000.0 } else { -20_000.0 };
            (a * s + j) as i64
        })
        .collect();
    tracker_from(&resid)
}

#[test]
fn whiteness_uses_the_local_jitter_over_two_n_minus_one() {
    // Largest quarter mean 167.4 us against a bound of 172.3 us (n - 1 = 15 differences); 2n or
    // 2(n + 1) would give 166.8 us / 161.8 us.
    assert!(bowl(360_000.0).residuals_are_white());
    // 277.1 us against 271.2 us; n + 1 in the denominator would give 360.3 us.
    assert!(!bowl(600_000.0).residuals_are_white());
}

#[test]
fn a_tight_interval_clamps_the_hysteresis_to_a_quarter() {
    // 1000 fps: h = 1 ms / 4 = 250 us. An instant 300 us before the predicted slot is a crossing.
    let i = 1_000_000;
    let base = 1_790_176_593 * NS_PER_SECOND;
    let mut s = SlotHysteresis::default();
    assert_eq!(s.choose(base + 100_000, 1, i), (base, SlotEvent::Start));
    assert_eq!(
        s.choose(base + i - 300_000, 1, i),
        (base, SlotEvent::Crossing)
    );
}

#[test]
fn an_instant_exactly_the_hysteresis_before_the_edge_is_held() {
    let base = 1_790_176_593 * NS_PER_SECOND;
    let mut s = SlotHysteresis::default();
    assert_eq!(s.choose(base + 1_000_000, 1, I60), (base, SlotEvent::Start));
    let expected = grid_advance_ns(base, 1, I60);
    assert_eq!(
        s.choose(expected - SLOT_HYSTERESIS_NS, 1, I60),
        (expected, SlotEvent::Held)
    );
}

#[test]
fn a_zero_expected_slot_is_never_stepped_back_from() {
    let mut s = SlotHysteresis::default();
    assert_eq!(s.choose(100, 1, I60), (0, SlotEvent::Start));
    // seq_advance 0 predicts slot 0 again; five slots on is a jump.
    let five = grid_advance_ns(0, 5, I60);
    assert_eq!(s.choose(five + 100, 0, I60), (five, SlotEvent::Jump));
}

#[test]
fn a_realtime_instant_at_or_below_zero_never_drives() {
    let mut c = driven_phase(T0, 130, 0);
    assert_eq!(c.mode(), PhaseMode::Stamp);
    let mut probe = c.clone();
    let s = probe
        .tracker
        .observe(1130, ts60(T0, 130))
        .smoothed_mono_ns
        .unwrap();
    assert_eq!(c.stamp_frame(1130, ts60(T0, 130), -(s as i64), I60), None);
    assert_eq!(c.mode(), PhaseMode::Band);
}

#[test]
fn a_realtime_instant_up_to_u64_max_drives_and_past_it_does_not() {
    let start = u64::MAX - 10 * NS_PER_SECOND;
    let c = driven_phase(start, 130, 0);
    assert_eq!(c.mode(), PhaseMode::Stamp);
    let smoothed = |c: &CapturePhase| {
        let mut probe = c.clone();
        probe
            .tracker
            .observe(1130, ts60(start, 130))
            .smoothed_mono_ns
            .unwrap()
    };
    let s = smoothed(&c);
    let at_max = (u64::MAX - s) as i64;
    let mut on = c.clone();
    let slot = on.stamp_frame(1130, ts60(start, 130), at_max, I60);
    assert_eq!(slot, Some(grid_floor_ns(u64::MAX, I60)));
    assert_eq!(on.mode(), PhaseMode::Stamp);
    let mut past = c.clone();
    assert_eq!(
        past.stamp_frame(1130, ts60(start, 130), at_max + 1, I60),
        None
    );
    assert_eq!(past.mode(), PhaseMode::Band);
}

#[test]
fn a_stream_that_stops_driving_restarts_its_slots() {
    // The realtime instant sits 8 ms into each slot.
    let offset = (1_790_176_593 * NS_PER_SECOND + 8_000_000 - T0) as i64;
    let mut c = driven_phase(T0, 130, offset);
    assert_eq!(c.mode(), PhaseMode::Stamp);
    // One frame off the grid (realtime 0): it stops driving.
    assert_eq!(
        c.stamp_frame(1130, ts60(T0, 130), -(T0 as i64) * 2, I60),
        None
    );
    // Back on the grid two slots later: a fresh start, no crossing.
    let slot = c.stamp_frame(1131, ts60(T0, 131), offset, I60);
    let t_real = (i128::from(ts60(T0, 131)) + i128::from(offset)) as u64;
    assert_eq!(slot, Some(grid_floor_ns(t_real, I60)));
    assert_eq!(c.crossings(), 0);
}

/// `push(2, T0)` builds a non-monotonic fit `observe` never produces: it pins the defensive
/// `p > 0.0` guard of `rate_ppm`. Not an exclude: unlike the source fit (whose `offer` keeps t
/// strictly increasing), the short fit is shifted by `absorb_step` apart from the samples it
/// folds, so `nb > 0` has no clean proof there.
#[test]
fn a_flat_fit_has_no_rate() {
    let mut c = CapturePhase::new();
    c.tracker.push(0, T0);
    c.tracker.push(1, T0 + 1_000);
    c.tracker.push(2, T0);
    assert_eq!(c.tracker.period_ns(), Some(0.0));
    assert_eq!(c.rate_ppm(I60), None);
}

#[test]
fn the_capture_phase_counters_and_note_read_the_tracker() {
    let mut c = CapturePhase::new();
    assert_eq!(c.source_reanchors(), 0);
    for k in 0..40u64 {
        c.stamp_frame(1000 + k as u32, T0 + k * P, 0, I60);
    }
    // One whole period late: a hidden device drop.
    c.stamp_frame(1040, T0 + 41 * P, 0, I60);
    assert_eq!(c.hidden_drops(), 1);
    assert_eq!(
        c.note(),
        Some(&PhaseNote::HiddenDrop {
            frames: 1,
            resid_ns: P as i64
        })
    );
    assert_eq!(c.reseeds(), 0);
    // A backward sequence: a re-seed.
    c.stamp_frame(900, T0 + 42 * P, 0, I60);
    assert_eq!(c.reseeds(), 1);
    assert_eq!(
        c.note(),
        Some(&PhaseNote::Discontinuity {
            seq_step: -140,
            dt_ns: P
        })
    );
    assert_eq!(c.source_reanchors(), 0);
}

#[test]
fn the_source_rate_needs_a_positive_interval() {
    let mut c = CapturePhase::new();
    for k in 0..400u64 {
        c.tracker.source.offer((k * 10) as i64, ts60(T0, k * 10));
    }
    assert!(c.tracker.source().ready());
    assert!(c.source_rate_ppm(I60).unwrap().abs() < 0.01);
    assert_eq!(c.source_rate_ppm(0), None);
}

#[test]
fn slot_mid_is_the_slot_middle_in_100ns_units() {
    assert_eq!(slot_mid_realtime_100ns(1_000_000, I60), 93_333);
    assert_eq!(slot_mid_realtime_100ns(0, 1_000), 5);
}

/// Source points `x = 10 k` at `t = T0 + k * step + a small non-linear wobble`.
fn source_point(k: u64, step: u64) -> (i64, u64) {
    ((k * 10) as i64, T0 + k * step + (k * k % 7) * 1_000)
}

#[test]
fn the_source_window_holds_at_most_max_points_with_exact_sums() {
    let mut s = SourcePhase::default();
    let n = SOURCE_FIT_MAX_POINTS as u64 + 1;
    for k in 0..n {
        let (x, t) = source_point(k, 1_000_000);
        s.offer(x, t);
    }
    assert_eq!(s.points(), SOURCE_FIT_MAX_POINTS);
    let mut fresh = SourcePhase::default();
    for k in 1..n {
        let (x, t) = source_point(k, 1_000_000);
        fresh.offer(x, t);
    }
    assert_eq!(s.span_ns(), fresh.span_ns());
    assert_eq!(s.period_ns(), fresh.period_ns());
}

#[test]
fn the_source_window_keeps_a_point_exactly_its_span_old() {
    let mut s = SourcePhase::default();
    for k in 0..=SOURCE_FIT_SECONDS {
        s.offer((k * 10) as i64, T0 + k * NS_PER_SECOND);
    }
    assert_eq!(s.points(), SOURCE_FIT_SECONDS as usize + 1);
    assert_eq!(s.span_ns(), SOURCE_FIT_SECONDS * NS_PER_SECOND);
    let k = SOURCE_FIT_SECONDS + 1;
    s.offer((k * 10) as i64, T0 + k * NS_PER_SECOND);
    assert_eq!(s.points(), SOURCE_FIT_SECONDS as usize + 1);
    assert_eq!(s.span_ns(), SOURCE_FIT_SECONDS * NS_PER_SECOND);
}

#[test]
fn the_source_period_needs_three_points() {
    let mut s = SourcePhase::default();
    s.offer(0, T0);
    s.offer(10, T0 + 10 * P);
    assert_eq!(s.period_ns(), None);
    s.offer(20, T0 + 20 * P);
    assert_eq!(s.period_ns(), Some(P as f64));
}

#[test]
fn a_source_shift_moves_the_line_by_exactly_its_delta() {
    // Boot-relative times near zero: the shift must add, never subtract.
    let mut s = SourcePhase::default();
    for k in 0..=61u64 {
        s.offer((k * 10) as i64, 1_000 + k * NS_PER_SECOND);
    }
    let before = s.predict_ns(300).unwrap();
    s.shift(5_000);
    assert_eq!(s.predict_ns(300), Some(before + 5_000));
    assert_eq!(s.span_ns(), 61 * NS_PER_SECOND);
}

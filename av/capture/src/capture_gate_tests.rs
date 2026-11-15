//! Tests added when the module moved to vstack (vstack #8): exact values for the chroma and
//! roughness samplers, the range scaling, the control report and the capture-loss tally, each
//! of which the camera-box tests only bounded. A `#[path]` child of `capture`.

use super::*;

/// One YUYV row of `macropixels` macropixels, all `Y0 U Y1 V` = `px`.
fn row(macropixels: usize, px: [u8; 4]) -> Vec<u8> {
    px.iter().copied().cycle().take(macropixels * 4).collect()
}

#[test]
fn chroma_of_a_one_pixel_wide_frame_is_zero() {
    // 1 pixel is no whole macropixel: nothing is sampled, even though the 4 bytes exist.
    assert_eq!(mean_chroma(&[16, 200, 16, 200], 1, 1, 4), (0.0, 0.0));
    assert_eq!(luma_roughness(&[16, 200, 116, 200], 1, 1, 4), 0.0);
}

#[test]
fn chroma_is_the_exact_mean_deviation_of_u_and_v() {
    // U = 128 + 40, V = 128 - 20 on every macropixel: means 40 and 20 exactly.
    let frame = row(2, [16, 168, 16, 108]);
    assert_eq!(mean_chroma(&frame, 4, 1, 8), (40.0, 20.0));
}

#[test]
fn a_macropixel_that_ends_on_the_last_byte_is_skipped_never_read_past() {
    // Three bytes: the V byte of macropixel 0 would be index 3, one past the end.
    assert_eq!(mean_chroma(&[16, 200, 16], 2, 1, 4), (0.0, 0.0));
    assert_eq!(luma_roughness(&[16, 200, 116], 2, 1, 4), 0.0);
}

#[test]
fn every_sixty_fourth_macropixel_is_sampled_at_its_own_offset() {
    // 65 macropixels: samples at 0 and 64 only. Macropixel 64 carries the colour and the luma
    // step; every other one is neutral and flat.
    let mut frame = row(65, [100, 128, 100, 128]);
    let at = 64 * 4;
    frame[at..at + 4].copy_from_slice(&[10, 228, 70, 28]);
    // U: (0 + 100) / 2, V: (0 + 100) / 2, roughness: (0 + 60) / 2.
    assert_eq!(mean_chroma(&frame, 130, 1, 260), (50.0, 50.0));
    assert_eq!(luma_roughness(&frame, 130, 1, 260), 30.0);
}

#[test]
fn a_row_of_exactly_sixty_four_macropixels_samples_only_the_first() {
    // 64 macropixels per row: the next sample (macropixel 64) is past the row, in the padding.
    let mut frame = row(64, [100, 128, 100, 128]);
    frame.extend_from_slice(&[10, 228, 70, 28]);
    frame.resize(1024, 0);
    assert_eq!(mean_chroma(&frame, 128, 1, 1024), (0.0, 0.0));
    assert_eq!(luma_roughness(&frame, 128, 1, 1024), 0.0);
}

#[test]
fn roughness_is_the_exact_mean_adjacent_luma_difference() {
    let frame = row(2, [40, 128, 100, 128]);
    assert_eq!(luma_roughness(&frame, 4, 1, 8), 60.0);
}

#[test]
fn range_scaling_spans_from_the_minimum_not_from_zero() {
    // A 16..235 range (a video-level card), 75 %: 16 + 0.75 * 219 = 180.25.
    let range = ControlRange {
        minimum: 16,
        maximum: 235,
        default_value: 128,
    };
    assert_eq!(scale_to_range(75, range), 180);
}

/// A device whose set always succeeds and whose read-back always fails.
struct UnreadableDevice;

impl ControlIo for UnreadableDevice {
    type Err = String;
    fn set_ctrl(&self, _id: u32, _value: i64) -> std::result::Result<(), String> {
        Ok(())
    }
    fn get_ctrl(&self, _id: u32) -> std::result::Result<i64, String> {
        Err("read-back unsupported".into())
    }
    fn query_range(&self, _id: u32) -> std::result::Result<ControlRange, String> {
        Err("no range".into())
    }
}

#[test]
fn a_failed_read_back_counts_each_control_as_adjusted() {
    let report = apply_controls_with(&UnreadableDevice, &certified_cam1_controls());
    assert_eq!(
        report,
        ControlReport {
            applied: 0,
            adjusted: certified_cam1_controls().len(),
            failed: 0,
        }
    );
}

#[test]
fn a_report_has_issues_only_when_something_was_adjusted_or_failed() {
    let clean = ControlReport {
        applied: 2,
        adjusted: 0,
        failed: 0,
    };
    assert!(!clean.has_issues());
    assert!(ControlReport {
        adjusted: 1,
        ..clean
    }
    .has_issues());
    assert!(ControlReport { failed: 1, ..clean }.has_issues());
}

#[test]
fn the_tally_counts_delivered_buffers_and_sequence_gaps() {
    let mut t = CaptureTally::default();
    assert_eq!(
        (
            t.dropped_captures(),
            t.frames_captured(),
            t.corrupted_frames()
        ),
        (0, 0, 0)
    );
    t.record_sequence(10); // the first buffer: no previous sequence, no gap
    t.record_sequence(11);
    t.record_sequence(14); // 12 and 13 lost
    t.record_sequence(14); // a repeat is no gap
    assert_eq!(t.dropped_captures(), 2);
    assert_eq!(t.frames_captured(), 4);
    t.record_sequence(17); // 15 and 16 lost: the total grows by the gap
    assert_eq!(t.dropped_captures(), 4);
    assert_eq!(t.frames_captured(), 5);
}

#[test]
fn the_tally_counts_corrupted_buffers_apart_from_device_drops() {
    let mut t = CaptureTally::default();
    assert_eq!(t.record_corrupted(), 1);
    assert_eq!(t.record_corrupted(), 2);
    assert_eq!(t.corrupted_frames(), 2);
    assert_eq!((t.dropped_captures(), t.frames_captured()), (0, 0));
}

#[test]
fn a_card_name_is_none_when_the_device_does_not_open() {
    assert_eq!(query_card_name("/nonexistent/vstack-video99"), None);
}

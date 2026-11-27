use super::*;

fn patterned_frame(width: u32, height: u32, seed: u32) -> RgbaFrame {
    let mut pixels = vec![0u8; width as usize * height as usize * 4];
    let mut state = seed | 1;
    for pixel in pixels.as_chunks_mut::<4>().0 {
        state ^= state << 13;
        state ^= state >> 17;
        state ^= state << 5;
        pixel[0] = (state >> 8) as u8;
        pixel[1] = (state >> 16) as u8;
        pixel[2] = (state >> 24) as u8;
        pixel[3] = 255;
    }
    RgbaFrame::from_pixels(width, height, pixels).unwrap()
}

fn gradient_frame(width: u32, height: u32) -> RgbaFrame {
    let mut pixels = vec![0u8; width as usize * height as usize * 4];
    for y in 0..height {
        for x in 0..width {
            let index = (y as usize * width as usize + x as usize) * 4;
            let value = (y * 200 / height.max(1)) as u8;
            let value = if y % 37 == 0 {
                (x * 255 / width.max(1)) as u8
            } else {
                value
            };
            pixels[index] = value;
            pixels[index + 1] = value;
            pixels[index + 2] = value;
            pixels[index + 3] = 255;
        }
    }
    RgbaFrame::from_pixels(width, height, pixels).unwrap()
}

fn slice_rows(frame: &RgbaFrame, start: u32, end: u32) -> RgbaFrame {
    let start_index = start as usize * frame.width() as usize * 4;
    let end_index = end as usize * frame.width() as usize * 4;
    RgbaFrame::from_pixels(
        frame.width(),
        end - start,
        frame.pixels()[start_index..end_index].to_vec(),
    )
    .unwrap()
}

fn stitcher(width: u32) -> Stitcher {
    Stitcher::new(width, StitchConfig::default()).unwrap()
}

fn with_header(content: &RgbaFrame, header: u32) -> RgbaFrame {
    let width = content.width();
    let mut pixels = vec![0u8; width as usize * header as usize * 4];
    for pixel in pixels.as_chunks_mut::<4>().0 {
        pixel.copy_from_slice(&[24, 24, 24, 255]);
    }
    pixels.extend_from_slice(content.pixels());
    RgbaFrame::from_pixels(width, header + content.height(), pixels).unwrap()
}

fn with_footer(content: &RgbaFrame, footer: u32) -> RgbaFrame {
    let width = content.width();
    let mut pixels = content.pixels().to_vec();
    for _ in 0..footer {
        for _ in 0..width {
            pixels.extend_from_slice(&[200, 80, 40, 255]);
        }
    }
    RgbaFrame::from_pixels(width, content.height() + footer, pixels).unwrap()
}

fn sparse_page(width: u32, height: u32, bands: &[u32]) -> RgbaFrame {
    let mut pixels = vec![255u8; width as usize * height as usize * 4];
    for (index, &band) in bands.iter().enumerate() {
        for y in band..(band + 14).min(height) {
            for x in 0..width {
                let offset = (y as usize * width as usize + x as usize) * 4;
                let seed = index as u32 * 11;
                let value = if (x * 7 + y * 13 + seed) % 17 < 8 {
                    30
                } else {
                    210
                };
                pixels[offset..offset + 4].copy_from_slice(&[value, value, value, 255]);
            }
        }
    }
    RgbaFrame::from_pixels(width, height, pixels).unwrap()
}

#[test]
fn blank_margins_are_not_treated_as_chrome() {
    let page = sparse_page(64, 1200, &[40, 120, 260, 400, 520]);
    let mut stitches = stitcher(64);
    stitches.push(&slice_rows(&page, 0, 600)).unwrap();
    let report = stitches.push(&slice_rows(&page, 10, 610)).unwrap();
    assert_eq!(report.trimmed_top_rows, 0);
    assert_eq!(report.trimmed_bottom_rows, 0);
    assert_eq!(report.appended_rows, 10);
    assert_eq!(report.total_height, 610);
}

#[test]
fn sparse_page_displacements_are_stitched() {
    let page = sparse_page(64, 1400, &[0, 120, 260, 400, 520, 586]);
    for shift in [1u32, 5, 10, 30, 60, 120] {
        let mut stitches = stitcher(64);
        stitches.push(&slice_rows(&page, 0, 600)).unwrap();
        let report = stitches
            .push(&slice_rows(&page, shift, 600 + shift))
            .unwrap();
        assert_eq!(report.appended_rows, shift, "shift {shift}");
        assert_eq!(report.total_height, 600 + shift, "shift {shift}");
    }
}

#[test]
fn slow_row_by_row_scroll_is_stitched() {
    let page = sparse_page(64, 1200, &[40, 120, 260, 400, 520]);
    let mut stitches = stitcher(64);
    stitches.push(&slice_rows(&page, 0, 600)).unwrap();
    for step in 1..=10u32 {
        let report = stitches.push(&slice_rows(&page, step, 600 + step)).unwrap();
        assert_eq!(report.appended_rows, 1, "step {step}");
        assert_eq!(report.total_height, 600 + step, "step {step}");
    }
}

#[test]
fn first_frame_is_kept_whole() {
    let frame = patterned_frame(40, 100, 7);
    let mut stitches = stitcher(40);
    let report = stitches.push(&frame).unwrap();
    assert_eq!(report.appended_rows, 100);
    assert_eq!(stitches.height(), 100);
    assert_eq!(stitches.finish().unwrap().pixels(), frame.pixels());
}

#[test]
fn overlapping_frames_are_stitched() {
    let original = patterned_frame(48, 300, 42);
    let mut stitches = stitcher(48);
    stitches.push(&slice_rows(&original, 0, 140)).unwrap();
    stitches.push(&slice_rows(&original, 100, 240)).unwrap();
    stitches.push(&slice_rows(&original, 200, 300)).unwrap();
    let stitched = stitches.finish().unwrap();
    assert_eq!(stitched.width(), 48);
    assert_eq!(stitched.height(), 300);
    assert_eq!(stitched.pixels(), original.pixels());
}

#[test]
fn gentle_scroll_with_large_overlap_is_stitched() {
    let original = patterned_frame(48, 1500, 21);
    let mut stitches = stitcher(48);
    stitches.push(&slice_rows(&original, 0, 1000)).unwrap();
    let report = stitches.push(&slice_rows(&original, 120, 1120)).unwrap();
    assert_eq!(report.overlap_rows, 880);
    assert_eq!(report.appended_rows, 120);
    let stitched = stitches.finish().unwrap();
    assert_eq!(stitched.height(), 1120);
    assert_eq!(stitched.pixels(), &original.pixels()[..1120 * 48 * 4]);
}

#[test]
fn refinement_picks_the_exact_alignment() {
    let original = gradient_frame(64, 600);
    let mut stitches = stitcher(64);
    stitches.push(&slice_rows(&original, 0, 300)).unwrap();
    stitches.push(&slice_rows(&original, 60, 360)).unwrap();
    let stitched = stitches.finish().unwrap();
    assert_eq!(stitched.height(), 360);
    assert_eq!(stitched.pixels(), &original.pixels()[..360 * 64 * 4]);
}

#[test]
fn duplicate_frame_appends_nothing() {
    let frame = patterned_frame(32, 120, 9);
    let mut stitches = stitcher(32);
    stitches.push(&frame).unwrap();
    let report = stitches.push(&frame).unwrap();
    assert_eq!(report.overlap_rows, 120);
    assert_eq!(report.appended_rows, 0);
    assert_eq!(stitches.height(), 120);
}

#[test]
fn fixed_header_is_trimmed_from_later_frames() {
    let original = patterned_frame(48, 300, 17);
    let header = 20;
    let mut stitches = stitcher(48);
    stitches
        .push(&with_header(&slice_rows(&original, 0, 140), header))
        .unwrap();
    let report = stitches
        .push(&with_header(&slice_rows(&original, 100, 240), header))
        .unwrap();
    assert_eq!(report.trimmed_top_rows, header);
    stitches
        .push(&with_header(&slice_rows(&original, 200, 300), header))
        .unwrap();

    let stitched = stitches.finish().unwrap();
    assert_eq!(stitched.height(), header + 300);
    let header_bytes = header as usize * 48 * 4;
    assert_eq!(
        &stitched.pixels()[header_bytes..],
        original.pixels(),
        "content below the header should match the original image"
    );
}

#[test]
fn tall_fixed_header_is_trimmed() {
    let original = patterned_frame(48, 400, 23);
    let header = 200;
    let mut stitches = stitcher(48);
    stitches
        .push(&with_header(&slice_rows(&original, 0, 160), header))
        .unwrap();
    let report = stitches
        .push(&with_header(&slice_rows(&original, 120, 280), header))
        .unwrap();
    assert_eq!(report.trimmed_top_rows, header);
    stitches
        .push(&with_header(&slice_rows(&original, 240, 400), header))
        .unwrap();
    let stitched = stitches.finish().unwrap();
    assert_eq!(stitched.height(), header + 400);
    let header_bytes = header as usize * 48 * 4;
    assert_eq!(&stitched.pixels()[header_bytes..], original.pixels());
}

#[test]
fn fixed_footer_is_appended_once() {
    let original = patterned_frame(48, 400, 51);
    let footer = 32;
    let mut stitches = stitcher(48);
    stitches
        .push(&with_footer(&slice_rows(&original, 0, 160), footer))
        .unwrap();
    stitches
        .push(&with_footer(&slice_rows(&original, 100, 260), footer))
        .unwrap();
    stitches
        .push(&with_footer(&slice_rows(&original, 200, 360), footer))
        .unwrap();
    let stitched = stitches.finish().unwrap();
    assert_eq!(stitched.height(), 360 + footer);
    let content_bytes = 360 * 48 * 4;
    assert_eq!(
        &stitched.pixels()[..content_bytes],
        &original.pixels()[..content_bytes]
    );
    let footer_pixels = &stitched.pixels()[content_bytes..];
    for pixel in footer_pixels.as_chunks::<4>().0 {
        assert_eq!(*pixel, [200, 80, 40, 255]);
    }
}

#[test]
fn fixed_header_and_footer_are_trimmed_together() {
    let original = patterned_frame(48, 300, 61);
    let header = 16;
    let footer = 24;
    let mut stitches = stitcher(48);
    for (start, end) in [(0, 140), (100, 240), (200, 300)] {
        let content = with_footer(
            &with_header(&slice_rows(&original, start, end), header),
            footer,
        );
        stitches.push(&content).unwrap();
    }

    let stitched = stitches.finish().unwrap();
    assert_eq!(stitched.height(), header + 300 + footer);
    let header_bytes = header as usize * 48 * 4;
    let content_bytes = 300 * 48 * 4;
    assert_eq!(
        &stitched.pixels()[header_bytes..header_bytes + content_bytes],
        original.pixels(),
        "content between the fixed header and footer should match the original"
    );
    for pixel in stitched.pixels()[header_bytes + content_bytes..]
        .as_chunks::<4>()
        .0
    {
        assert_eq!(*pixel, [200, 80, 40, 255]);
    }
}

#[test]
fn upward_frames_are_prepended() {
    let original = patterned_frame(48, 300, 81);
    let mut stitches = stitcher(48);
    stitches.push(&slice_rows(&original, 100, 300)).unwrap();
    let report = stitches.push(&slice_rows(&original, 0, 200)).unwrap();
    assert_eq!(report.overlap_rows, 100);
    assert_eq!(report.prepended_rows, 100);
    assert_eq!(report.appended_rows, 0);
    let stitched = stitches.finish().unwrap();
    assert_eq!(stitched.height(), 300);
    assert_eq!(stitched.pixels(), original.pixels());
}

#[test]
fn scrolling_both_ways_keeps_the_content_order() {
    let original = patterned_frame(48, 400, 82);
    let mut stitches = stitcher(48);
    for (start, end) in [(100, 300), (150, 350), (50, 250), (0, 200), (200, 400)] {
        stitches.push(&slice_rows(&original, start, end)).unwrap();
    }
    let stitched = stitches.finish().unwrap();
    assert_eq!(stitched.height(), 400);
    assert_eq!(stitched.pixels(), original.pixels());
}

#[test]
fn frames_inside_the_capture_append_nothing() {
    let original = patterned_frame(48, 400, 83);
    let mut stitches = stitcher(48);
    stitches.push(&slice_rows(&original, 0, 200)).unwrap();
    stitches.push(&slice_rows(&original, 100, 300)).unwrap();
    let report = stitches.push(&slice_rows(&original, 50, 250)).unwrap();
    assert_eq!(report.appended_rows, 0);
    assert_eq!(report.prepended_rows, 0);
    let stitched = stitches.finish().unwrap();
    assert_eq!(stitched.height(), 300);
    assert_eq!(stitched.pixels(), &original.pixels()[..300 * 48 * 4]);
}

#[test]
fn paused_frame_after_scrolling_back_keeps_the_capture() {
    let original = patterned_frame(48, 400, 84);
    let mut stitches = stitcher(48);
    stitches.push(&slice_rows(&original, 0, 300)).unwrap();
    stitches.push(&slice_rows(&original, 100, 400)).unwrap();
    stitches.push(&slice_rows(&original, 50, 350)).unwrap();
    let mut pixels = slice_rows(&original, 50, 350).into_pixels();
    let row_bytes = 48 * 4;
    pixels[100 * row_bytes..102 * row_bytes].fill(0);
    let paused = RgbaFrame::from_pixels(48, 300, pixels).unwrap();
    let report = stitches.push(&paused).unwrap();
    assert_eq!(report.appended_rows, 0);
    assert_eq!(report.prepended_rows, 0);
    assert_eq!(stitches.height(), 400);
}

#[test]
fn fixed_header_and_footer_are_kept_once_when_scrolling_up() {
    let original = patterned_frame(48, 300, 91);
    let header = 16;
    let footer = 24;
    let mut stitches = stitcher(48);
    for (start, end) in [(100, 300), (0, 200)] {
        let content = with_footer(
            &with_header(&slice_rows(&original, start, end), header),
            footer,
        );
        stitches.push(&content).unwrap();
    }
    let stitched = stitches.finish().unwrap();
    assert_eq!(stitched.height(), header + 300 + footer);
    let header_bytes = header as usize * 48 * 4;
    let content_bytes = 300 * 48 * 4;
    assert_eq!(
        &stitched.pixels()[header_bytes..header_bytes + content_bytes],
        original.pixels()
    );
    for pixel in stitched.pixels()[header_bytes + content_bytes..]
        .as_chunks::<4>()
        .0
    {
        assert_eq!(*pixel, [200, 80, 40, 255]);
    }
}

#[test]
fn fixed_header_and_footer_survive_reversing_direction() {
    let original = patterned_frame(48, 400, 92);
    let header = 16;
    let footer = 24;
    let mut stitches = stitcher(48);
    for (start, end) in [(100, 300), (50, 250), (0, 200), (200, 400)] {
        let content = with_footer(
            &with_header(&slice_rows(&original, start, end), header),
            footer,
        );
        stitches.push(&content).unwrap();
    }
    let stitched = stitches.finish().unwrap();
    assert_eq!(stitched.height(), header + 400 + footer);
    let header_bytes = header as usize * 48 * 4;
    let content_bytes = 400 * 48 * 4;
    assert_eq!(
        &stitched.pixels()[header_bytes..header_bytes + content_bytes],
        original.pixels()
    );
    for pixel in stitched.pixels()[header_bytes + content_bytes..]
        .as_chunks::<4>()
        .0
    {
        assert_eq!(*pixel, [200, 80, 40, 255]);
    }
}

#[test]
fn height_limit_stops_upward_growth() {
    let mut stitches = Stitcher::new(
        16,
        StitchConfig {
            max_height: 180,
            ..Default::default()
        },
    )
    .unwrap();
    let original = patterned_frame(16, 300, 5);
    stitches.push(&slice_rows(&original, 100, 250)).unwrap();
    let result = stitches.push(&slice_rows(&original, 0, 150));
    assert!(matches!(result, Err(StitchError::HeightLimit(180))));
}

#[test]
fn blank_frames_append_nothing() {
    let width = 32;
    let pixels = vec![255u8; width as usize * 120 * 4];
    let frame = RgbaFrame::from_pixels(width, 120, pixels).unwrap();
    let mut stitches = stitcher(width);
    stitches.push(&frame).unwrap();
    let report = stitches.push(&frame).unwrap();
    assert_eq!(report.appended_rows, 0);
    assert_eq!(stitches.height(), 120);
}

#[test]
fn width_mismatch_is_rejected() {
    let frame = patterned_frame(30, 120, 3);
    let mut stitches = stitcher(32);
    assert!(matches!(
        stitches.push(&frame),
        Err(StitchError::WidthMismatch { .. })
    ));
}

#[test]
fn disjoint_frames_report_no_overlap() {
    let mut stitches = stitcher(40);
    stitches.push(&patterned_frame(40, 120, 11)).unwrap();
    let result = stitches.push(&patterned_frame(40, 120, 29));
    assert!(matches!(result, Err(StitchError::NoOverlap)));
}

#[test]
fn height_limit_stops_growth() {
    let mut stitches = Stitcher::new(
        16,
        StitchConfig {
            max_height: 180,
            ..Default::default()
        },
    )
    .unwrap();
    let original = patterned_frame(16, 300, 5);
    stitches.push(&slice_rows(&original, 0, 180)).unwrap();
    let result = stitches.push(&slice_rows(&original, 140, 240));
    assert!(matches!(result, Err(StitchError::HeightLimit(180))));
}

#[test]
fn memory_budget_stops_wide_frames() {
    let mut stitches = Stitcher::new(
        64,
        StitchConfig {
            max_height: 10_000,
            max_bytes: (64 * 4 + 48) * 70 + 512 * 48 + 256 * 64 * 4,
            ..Default::default()
        },
    )
    .unwrap();
    let original = patterned_frame(64, 120, 31);
    stitches.push(&slice_rows(&original, 0, 60)).unwrap();
    let result = stitches.push(&slice_rows(&original, 30, 90));
    assert!(matches!(result, Err(StitchError::MemoryLimit(_))));
}

#[test]
fn owned_first_frame_donates_its_buffer() {
    let frame = patterned_frame(40, 100, 13);
    let pointer = frame.pixels().as_ptr();
    let mut stitches = stitcher(40);
    let report = stitches.push_owned(frame).unwrap();
    assert_eq!(report.appended_rows, 100);
    assert_eq!(stitches.height(), 100);
    assert_eq!(stitches.pixels.as_ptr(), pointer);
}

#[test]
fn replacing_the_first_frame_discards_the_previous_buffer() {
    let stale = patterned_frame(48, 200, 7);
    let page = patterned_frame(48, 400, 19);
    let mut stitches = stitcher(48);
    stitches.push(&stale).unwrap();
    stitches
        .replace_with_frame(&slice_rows(&page, 0, 200))
        .unwrap();
    stitches.push(&slice_rows(&page, 100, 300)).unwrap();
    let stitched = stitches.finish().unwrap();
    assert_eq!(stitched.height(), 300);
    assert_eq!(stitched.pixels(), &page.pixels()[..300 * 48 * 4]);
}

#[test]
fn replace_with_frame_rejects_a_width_mismatch_and_keeps_the_buffer() {
    let page = patterned_frame(48, 200, 21);
    let mut stitches = stitcher(48);
    stitches.push(&page).unwrap();
    let other = patterned_frame(40, 200, 22);
    assert!(matches!(
        stitches.replace_with_frame(&other),
        Err(StitchError::WidthMismatch { .. })
    ));
    let stitched = stitches.finish().unwrap();
    assert_eq!(stitched.pixels(), page.pixels());
}

#[test]
fn thumbnail_keeps_the_aspect_ratio() {
    let frame = patterned_frame(40, 100, 71);
    let mut stitches = stitcher(40);
    stitches.push(&frame).unwrap();
    let thumbnail = stitches.thumbnail(20, 100).expect("thumbnail");
    assert_eq!(thumbnail.width(), 20);
    assert_eq!(thumbnail.height(), 50);
    assert_eq!(&thumbnail.pixels()[..4], &frame.pixels()[..4]);
}

#[test]
fn capacity_never_exceeds_the_byte_budget() {
    let width = 32u32;
    let row_bytes = width as usize * 4;
    let max_bytes = (row_bytes + 32) * 500 + 512 * 32 + 256 * row_bytes;
    let mut stitches = Stitcher::new(
        width,
        StitchConfig {
            max_height: 10_000,
            max_bytes,
            ..Default::default()
        },
    )
    .unwrap();
    let original = patterned_frame(width, 400, 41);
    for (start, end) in [(0, 120), (90, 210), (180, 300), (270, 390)] {
        stitches.push(&slice_rows(&original, start, end)).unwrap();
        assert!(
            stitches.pixels.capacity() <= max_bytes,
            "capacity {} exceeds budget {max_bytes}",
            stitches.pixels.capacity()
        );
    }
    assert!(stitches.finish().is_ok());
}

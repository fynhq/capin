use core_types::RgbaFrame;

use super::{PreviewAnchor, StitchConfig, Stitcher};

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

fn sparse_frame(width: u32, height: u32, band_row: u32) -> RgbaFrame {
    let mut pixels = vec![255u8; width as usize * height as usize * 4];
    for row in band_row..(band_row + 10).min(height) {
        for column in 0..width {
            let index = (row as usize * width as usize + column as usize) * 4;
            let value = ((row * 17 + column * 31) % 256) as u8;
            pixels[index] = value;
            pixels[index + 1] = 255 - value;
            pixels[index + 2] = value / 2;
        }
    }
    RgbaFrame::from_pixels(width, height, pixels).unwrap()
}

fn slice_rows(frame: &RgbaFrame, start: u32, end: u32) -> RgbaFrame {
    let row_bytes = frame.width() as usize * 4;
    RgbaFrame::from_pixels(
        frame.width(),
        end - start,
        frame.pixels()[start as usize * row_bytes..end as usize * row_bytes].to_vec(),
    )
    .unwrap()
}

fn stitcher(width: u32) -> Stitcher {
    Stitcher::new(width, StitchConfig::default()).unwrap()
}

#[test]
fn small_scroll_on_sparse_content_is_stitched() {
    let source = sparse_frame(48, 1210, 600);
    let mut stitches = stitcher(48);
    stitches.push(&slice_rows(&source, 0, 1200)).unwrap();
    let report = stitches.push(&slice_rows(&source, 10, 1210)).unwrap();
    assert_eq!(report.appended_rows, 10);
    let stitched = stitches.finish().unwrap();
    assert_eq!(stitched.height(), 1210);
    assert_eq!(stitched.pixels(), source.pixels());
}

#[test]
fn exact_duplicate_frame_appends_nothing() {
    let original = patterned_frame(48, 200, 9);
    let frame = slice_rows(&original, 0, 160);
    let mut stitches = stitcher(48);
    stitches.push(&frame).unwrap();
    let report = stitches.push(&frame).unwrap();
    assert_eq!(report.appended_rows, 0);
    assert_eq!(report.prepended_rows, 0);
    assert_eq!(stitches.height(), 160);
}

#[test]
fn anchored_preview_shows_top_or_bottom_window() {
    let original = patterned_frame(48, 800, 11);
    let mut stitches = stitcher(48);
    stitches.push(&original).unwrap();
    let top = stitches
        .thumbnail_anchored(48, 120, PreviewAnchor::Top)
        .unwrap();
    let bottom = stitches
        .thumbnail_anchored(48, 120, PreviewAnchor::Bottom)
        .unwrap();
    assert_eq!(top.width(), 48);
    assert_eq!(top.height(), 120);
    assert_eq!(bottom.width(), 48);
    assert_eq!(bottom.height(), 120);
    let row_bytes = 48 * 4;
    assert_eq!(top.pixels(), &original.pixels()[..120 * row_bytes]);
    assert_eq!(
        bottom.pixels(),
        &original.pixels()[(800 - 120) * row_bytes..]
    );
}

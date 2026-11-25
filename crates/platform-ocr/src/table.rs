use core_types::RgbaFrame;

use crate::OcrLine;

const MAX_COLUMNS: usize = 64;
const MAX_RULES: usize = 2048;
const RULE_TOLERANCE: i32 = 12;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OcrTableCell {
    pub row: u16,
    pub column: u16,
    pub lines: Vec<usize>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct OcrTable {
    pub rows: Vec<Vec<String>>,
    pub cells: Vec<OcrTableCell>,
    pub line_cells: Vec<Option<(u16, u16)>>,
}

impl OcrTable {
    pub fn tsv(&self) -> String {
        let mut text = String::new();
        for (index, row) in self.rows.iter().enumerate() {
            if index > 0 {
                text.push_str("\r\n");
            }
            for (column, cell) in row.iter().enumerate() {
                if column > 0 {
                    text.push('\t');
                }
                text.push_str(cell);
            }
        }
        text
    }

    pub fn markdown(&self) -> String {
        let columns = self.rows.iter().map(Vec::len).max().unwrap_or(0);
        if columns == 0 {
            return String::new();
        }
        let mut text = String::new();
        for (index, row) in self.rows.iter().enumerate() {
            text.push('|');
            for column in 0..columns {
                text.push(' ');
                text.push_str(row.get(column).map(String::as_str).unwrap_or(""));
                text.push_str(" |");
            }
            if index == 0 {
                text.push('\n');
                text.push('|');
                for _ in 0..columns {
                    text.push_str(" --- |");
                }
            }
            if index + 1 < self.rows.len() {
                text.push('\n');
            }
        }
        text
    }
}

pub(crate) fn detect_table(lines: &[OcrLine], frame: &RgbaFrame) -> Option<OcrTable> {
    if lines.len() < 4 {
        return None;
    }
    let text_rows = group_rows(lines);
    let typical = typical_height(lines);
    let cuts = find_cuts(lines, &text_rows, typical);
    if cuts.is_empty() {
        return None;
    }
    let (x0, x1, y0, y1) = line_bounds(lines);
    let rules = find_rules(frame, x0, x1, y0, y1);
    let bands = assign_bands(lines, &text_rows, &rules);
    let mut band_order: Vec<usize> = Vec::new();
    for band in bands.iter().flatten() {
        if !band_order.contains(band) {
            band_order.push(*band);
        }
    }
    if band_order.len() < 2 {
        return None;
    }
    let columns = cuts.len() + 1;
    let mut grid: Vec<Vec<Vec<usize>>> = vec![vec![Vec::new(); columns]; band_order.len()];
    let mut line_cells = vec![None; lines.len()];
    for (index, cell) in bands.iter().enumerate() {
        let Some(band) = cell else {
            continue;
        };
        let Some(row) = band_order.iter().position(|item| item == band) else {
            continue;
        };
        let line = &lines[index];
        if line.text.trim().is_empty() {
            continue;
        }
        let column = cuts.partition_point(|cut| *cut <= line.rect.origin.x);
        grid[row][column].push(index);
        line_cells[index] = Some((row as u16, column as u16));
    }

    let mut rows: Vec<Vec<String>> = Vec::new();
    let mut cells = Vec::new();
    let mut total_rows = 0usize;
    let mut full_rows = 0usize;
    for (row, entries) in grid.iter().enumerate() {
        let last = entries.iter().rposition(|entry| !entry.is_empty());
        let Some(last) = last else {
            continue;
        };
        let mut values = Vec::with_capacity(last + 1);
        let mut used = 0usize;
        for (column, entry) in entries.iter().enumerate().take(last + 1) {
            let text = cell_text(lines, entry);
            if !text.is_empty() {
                used += 1;
                cells.push(OcrTableCell {
                    row: row as u16,
                    column: column as u16,
                    lines: entry.clone(),
                });
            }
            values.push(text);
        }
        total_rows += 1;
        full_rows += usize::from(used >= 2);
        rows.push(values);
    }
    if total_rows < 2 || full_rows < 2 || full_rows * 5 < total_rows * 2 {
        return None;
    }
    Some(OcrTable {
        rows,
        cells,
        line_cells,
    })
}

fn group_rows(lines: &[OcrLine]) -> Vec<Vec<usize>> {
    let mut order: Vec<usize> = (0..lines.len()).collect();
    order.sort_by_key(|index| lines[*index].rect.origin.y);
    let mut rows: Vec<Vec<usize>> = Vec::new();
    for index in order {
        let line = &lines[index];
        let center = line.rect.origin.y + line.rect.size.height as i32 / 2;
        if let Some(row) = rows.last_mut() {
            let first = &lines[row[0]];
            let top = row
                .iter()
                .map(|item| lines[*item].rect.origin.y)
                .min()
                .unwrap_or(first.rect.origin.y);
            let bottom = row
                .iter()
                .map(|item| lines[*item].rect.origin.y + lines[*item].rect.size.height as i32)
                .max()
                .unwrap_or(top);
            let height = (bottom - top).max(1);
            if center >= top - height / 3 && center <= bottom + height / 3 {
                row.push(index);
                continue;
            }
        }
        rows.push(vec![index]);
    }
    for row in &mut rows {
        row.sort_by_key(|index| lines[*index].rect.origin.x);
    }
    rows
}

fn typical_height(lines: &[OcrLine]) -> i32 {
    let mut heights: Vec<i32> = lines
        .iter()
        .map(|line| line.rect.size.height as i32)
        .filter(|height| *height > 0)
        .collect();
    if heights.is_empty() {
        return 1;
    }
    heights.sort_unstable();
    heights[heights.len() / 2].max(1)
}

fn find_cuts(lines: &[OcrLine], rows: &[Vec<usize>], typical: i32) -> Vec<i32> {
    let mut spans = Vec::new();
    for row in rows {
        if row.len() > 1 {
            for index in row {
                let line = &lines[*index];
                spans.push((
                    line.rect.origin.x,
                    line.rect.origin.x + line.rect.size.width as i32,
                ));
            }
        }
    }
    spans.sort_unstable();
    let mut cuts = Vec::new();
    let mut reach = spans.first().map_or(0, |span| span.1);
    for span in spans.into_iter().skip(1) {
        if span.0 > reach {
            let gap = (reach, span.0);
            let mut vouch = 0usize;
            for row in rows {
                let left = row.iter().any(|index| {
                    let line = &lines[*index];
                    line.rect.origin.x + line.rect.size.width as i32 <= gap.0
                });
                let right = row.iter().any(|index| lines[*index].rect.origin.x >= gap.1);
                vouch += usize::from(left && right);
            }
            if vouch >= 2
                && (gap.1 - gap.0) * 10 >= typical * if vouch >= 4 { 3 } else { 10 }
                && cuts.len() < MAX_COLUMNS - 1
            {
                cuts.push((gap.0 + gap.1) / 2);
            }
        }
        reach = reach.max(span.1);
    }
    cuts
}

fn line_bounds(lines: &[OcrLine]) -> (i32, i32, i32, i32) {
    let mut x0 = i32::MAX;
    let mut x1 = i32::MIN;
    let mut y0 = i32::MAX;
    let mut y1 = i32::MIN;
    for line in lines {
        x0 = x0.min(line.rect.origin.x);
        x1 = x1.max(line.rect.origin.x + line.rect.size.width as i32);
        y0 = y0.min(line.rect.origin.y);
        y1 = y1.max(line.rect.origin.y + line.rect.size.height as i32);
    }
    (x0, x1, y0, y1)
}

fn find_rules(frame: &RgbaFrame, x0: i32, x1: i32, y0: i32, y1: i32) -> Vec<i32> {
    let width = frame.width() as i32;
    let height = frame.height() as i32;
    let x0 = x0.clamp(0, width);
    let x1 = x1.clamp(x0, width);
    let y0 = y0.clamp(1, height);
    let y1 = y1.clamp(y0, height);
    if x1 - x0 < 16 || y1 - y0 < 2 {
        return Vec::new();
    }
    let stride = frame.stride();
    let pixels = frame.pixels();
    let step = ((x1 - x0) / 384).max(1);
    let samples = ((x1 - x0) / step).max(1);
    let need = (samples * 6 / 10).max(16);
    let mut rules = Vec::new();
    for y in y0..y1 {
        let mut changed = 0i32;
        let mut x = x0;
        while x < x1 {
            let index = (y as usize) * stride + (x as usize) * 4;
            let previous = index - stride;
            let diff = (i32::from(pixels[index]) - i32::from(pixels[previous])).abs()
                + (i32::from(pixels[index + 1]) - i32::from(pixels[previous + 1])).abs()
                + (i32::from(pixels[index + 2]) - i32::from(pixels[previous + 2])).abs();
            if diff > RULE_TOLERANCE {
                changed += 1;
            }
            x += step;
        }
        if changed < need {
            continue;
        }
        if rules.len() >= MAX_RULES {
            break;
        }
        if let Some(last) = rules.last_mut()
            && y - *last <= 3
        {
            *last = y;
        } else {
            rules.push(y);
        }
    }
    rules
}

fn assign_bands(lines: &[OcrLine], rows: &[Vec<usize>], rules: &[i32]) -> Vec<Option<usize>> {
    let mut bands = vec![None; lines.len()];
    if rules.len() >= 4 {
        let mut inside = 0usize;
        let mut closed = 0usize;
        let mut last = None;
        for row in rows {
            let center = row_center(lines, row);
            let band = rules.partition_point(|rule| *rule <= center);
            if band > 0 && band < rules.len() {
                inside += 1;
                if last != Some(band) {
                    closed += 1;
                }
                last = Some(band);
            }
        }
        if closed >= 3 && closed * 2 >= inside.max(1) {
            for row in rows {
                let center = row_center(lines, row);
                let band = rules.partition_point(|rule| *rule <= center);
                if band > 0 && band < rules.len() {
                    for index in row {
                        bands[*index] = Some(band);
                    }
                }
            }
            return bands;
        }
    }
    for (row, indices) in rows.iter().enumerate() {
        for index in indices {
            bands[*index] = Some(row);
        }
    }
    bands
}

fn row_center(lines: &[OcrLine], row: &[usize]) -> i32 {
    let top = row
        .iter()
        .map(|index| lines[*index].rect.origin.y)
        .min()
        .unwrap_or(0);
    let bottom = row
        .iter()
        .map(|index| lines[*index].rect.origin.y + lines[*index].rect.size.height as i32)
        .max()
        .unwrap_or(top);
    (top + bottom) / 2
}

fn cell_text(lines: &[OcrLine], indices: &[usize]) -> String {
    let mut text = String::new();
    for index in indices {
        let value = lines[*index].text.trim();
        if value.is_empty() {
            continue;
        }
        if !text.is_empty()
            && !(text.chars().next_back().is_some_and(is_wide)
                && value.chars().next().is_some_and(is_wide))
        {
            text.push(' ');
        }
        text.push_str(value);
    }
    text
}

fn is_wide(ch: char) -> bool {
    ch as u32 >= 0x2E80
}

#[cfg(test)]
mod tests {
    use super::*;
    use core_types::PhysicalRect;

    fn line(text: &str, x: i32, y: i32, width: u32) -> OcrLine {
        OcrLine {
            text: text.to_owned(),
            rect: PhysicalRect::new(x, y, width, 10),
            chars: Vec::new(),
        }
    }

    fn blank_frame(width: u32, height: u32) -> RgbaFrame {
        RgbaFrame::from_pixels(
            width,
            height,
            vec![255; width as usize * height as usize * 4],
        )
        .unwrap()
    }

    #[test]
    fn columns_and_rows_are_detected() {
        let lines = vec![
            line("name", 0, 0, 60),
            line("count", 100, 0, 60),
            line("price", 200, 0, 60),
            line("apple", 0, 30, 60),
            line("3", 100, 30, 60),
            line("1.20", 200, 30, 60),
            line("pear", 0, 60, 60),
            line("5", 100, 60, 60),
            line("2.40", 200, 60, 60),
        ];
        let table = detect_table(&lines, &blank_frame(320, 100)).expect("table");
        assert_eq!(table.rows.len(), 3);
        assert_eq!(table.rows[0], vec!["name", "count", "price"]);
        assert_eq!(table.rows[1], vec!["apple", "3", "1.20"]);
        assert_eq!(table.rows[2], vec!["pear", "5", "2.40"]);
        assert_eq!(table.line_cells[1], Some((0, 1)));
        assert_eq!(
            table.tsv(),
            "name\tcount\tprice\r\napple\t3\t1.20\r\npear\t5\t2.40"
        );
    }

    #[test]
    fn single_column_is_not_a_table() {
        let lines = vec![
            line("one", 0, 0, 80),
            line("two", 0, 30, 80),
            line("three", 0, 60, 80),
            line("four", 0, 90, 80),
        ];
        assert!(detect_table(&lines, &blank_frame(120, 120)).is_none());
    }

    #[test]
    fn wrapped_cell_stays_in_one_row() {
        let mut pixels = vec![255u8; 320 * 120 * 4];
        for y in [1usize, 24, 54, 69] {
            for x in 0..300 {
                let index = (y * 320 + x) * 4;
                pixels[index] = 0;
                pixels[index + 1] = 0;
                pixels[index + 2] = 0;
            }
        }
        let frame = RgbaFrame::from_pixels(320, 120, pixels).unwrap();
        let lines = vec![
            line("name", 0, 0, 60),
            line("value", 100, 0, 60),
            line("wrapped", 0, 30, 60),
            line("text", 0, 40, 60),
            line("7", 100, 30, 60),
            line("last", 0, 60, 60),
            line("9", 100, 60, 60),
        ];
        let table = detect_table(&lines, &frame).expect("table");
        assert_eq!(table.rows[0], vec!["name", "value"]);
        assert_eq!(table.rows[1], vec!["wrapped text", "7"]);
        assert_eq!(table.rows[2], vec!["last", "9"]);
    }

    #[test]
    fn markdown_has_a_header_rule() {
        let table = OcrTable {
            rows: vec![
                vec!["a".to_owned(), "b".to_owned()],
                vec!["1".to_owned(), "2".to_owned()],
            ],
            cells: Vec::new(),
            line_cells: Vec::new(),
        };
        assert_eq!(table.markdown(), "| a | b |\n| --- | --- |\n| 1 | 2 |");
    }
}

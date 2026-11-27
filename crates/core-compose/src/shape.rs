use core_types::RgbaFrame;

#[derive(Clone, Copy)]
pub struct AlphaMask<'a> {
    pub alpha: &'a [u8],
    pub width: u32,
    pub height: u32,
}

#[derive(Clone, Copy)]
pub enum ShapeSource<'a> {
    Rounded,
    Mask(AlphaMask<'a>),
    WindowAlpha,
}

pub(crate) struct Coverage {
    data: Vec<u8>,
    width: u32,
    height: u32,
}

impl Coverage {
    pub(crate) fn from_mask(mask: &AlphaMask<'_>, width: u32, height: u32) -> Option<Self> {
        if width == 0 || height == 0 || mask.width == 0 || mask.height == 0 {
            return None;
        }
        let source_len = (mask.width as usize).checked_mul(mask.height as usize)?;
        if mask.alpha.len() < source_len {
            return None;
        }
        let len = (width as usize).checked_mul(height as usize)?;
        if mask.width == width && mask.height == height {
            return Some(Self {
                data: mask.alpha[..len].to_vec(),
                width,
                height,
            });
        }
        let mut data = vec![0u8; len];
        for y in 0..height {
            let fy = (y as f32 + 0.5) * mask.height as f32 / height as f32 - 0.5;
            for x in 0..width {
                let fx = (x as f32 + 0.5) * mask.width as f32 / width as f32 - 0.5;
                data[y as usize * width as usize + x as usize] =
                    sample_bilinear(mask.alpha, mask.width, mask.height, fx, fy);
            }
        }
        Some(Self {
            data,
            width,
            height,
        })
    }

    pub(crate) fn from_window(frame: &RgbaFrame) -> Self {
        let width = frame.width();
        let height = frame.height();
        let mut data = vec![0u8; width as usize * height as usize];
        for y in 0..height {
            let Some(row) = frame.row(y) else {
                continue;
            };
            let stride_start = y as usize * width as usize;
            for (index, value) in data[stride_start..stride_start + width as usize]
                .iter_mut()
                .enumerate()
            {
                *value = row[index * 4 + 3];
            }
        }
        Self {
            data,
            width,
            height,
        }
    }

    pub(crate) fn at(&self, x: u32, y: u32) -> u8 {
        if x >= self.width || y >= self.height {
            return 0;
        }
        self.data[y as usize * self.width as usize + x as usize]
    }

    pub(crate) fn width(&self) -> u32 {
        self.width
    }

    pub(crate) fn height(&self) -> u32 {
        self.height
    }

    pub(crate) fn byte(&self, x: usize, y: usize) -> u8 {
        self.data[y * self.width as usize + x]
    }
}

fn sample_bilinear(data: &[u8], width: u32, height: u32, fx: f32, fy: f32) -> u8 {
    if width == 0 || height == 0 {
        return 0;
    }
    let max_x = width.saturating_sub(1) as f32;
    let max_y = height.saturating_sub(1) as f32;
    let x = fx.clamp(0.0, max_x);
    let y = fy.clamp(0.0, max_y);
    let x0 = x.floor() as u32;
    let y0 = y.floor() as u32;
    let x1 = (x0 + 1).min(width - 1);
    let y1 = (y0 + 1).min(height - 1);
    let tx = x - x0 as f32;
    let ty = y - y0 as f32;
    let p00 = f32::from(data[y0 as usize * width as usize + x0 as usize]);
    let p10 = f32::from(data[y0 as usize * width as usize + x1 as usize]);
    let p01 = f32::from(data[y1 as usize * width as usize + x0 as usize]);
    let p11 = f32::from(data[y1 as usize * width as usize + x1 as usize]);
    let top = p00 * (1.0 - tx) + p10 * tx;
    let bottom = p01 * (1.0 - tx) + p11 * tx;
    (top * (1.0 - ty) + bottom * ty).round().clamp(0.0, 255.0) as u8
}

pub(crate) struct DistanceField {
    width: u32,
    height: u32,
    distance: Vec<f32>,
}

impl DistanceField {
    pub(crate) fn new(coverage: &Coverage) -> Self {
        const INF: i32 = 1 << 30;
        let width = coverage.width() as usize;
        let height = coverage.height() as usize;
        let mut grid = vec![(INF, INF); width * height];
        for y in 0..height {
            for x in 0..width {
                if coverage.byte(x, y) < 128 {
                    grid[y * width + x] = (0, 0);
                }
            }
        }

        let compare = |grid: &mut Vec<(i32, i32)>, x: usize, y: usize, dx: isize, dy: isize| {
            let nx = x as isize + dx;
            let ny = y as isize + dy;
            if nx < 0 || ny < 0 || nx >= width as isize || ny >= height as isize {
                return;
            }
            let neighbor = &grid[ny as usize * width + nx as usize];
            let cx = i64::from(neighbor.0) + dx as i64;
            let cy = i64::from(neighbor.1) + dy as i64;
            let current = grid[y * width + x];
            let current_sq = i64::from(current.0) * i64::from(current.0)
                + i64::from(current.1) * i64::from(current.1);
            if cx * cx + cy * cy < current_sq {
                grid[y * width + x] = (cx as i32, cy as i32);
            }
        };

        for y in 1..height {
            for x in 0..width {
                compare(&mut grid, x, y, -1, 0);
                compare(&mut grid, x, y, -1, -1);
                compare(&mut grid, x, y, 0, -1);
                compare(&mut grid, x, y, 1, -1);
            }
        }
        for y in (0..height.saturating_sub(1)).rev() {
            for x in (0..width).rev() {
                compare(&mut grid, x, y, 1, 0);
                compare(&mut grid, x, y, 1, 1);
                compare(&mut grid, x, y, 0, 1);
                compare(&mut grid, x, y, -1, 1);
            }
        }

        let mut distance = vec![0f32; width * height];
        for y in 0..height {
            for x in 0..width {
                let index = y * width + x;
                let (dx, dy) = grid[index];
                let unresolved = dx >= INF / 2 && dy >= INF / 2;
                let computed = if unresolved {
                    f32::INFINITY
                } else {
                    ((i64::from(dx) * i64::from(dx) + i64::from(dy) * i64::from(dy)) as f32).sqrt()
                };
                let frame_edge = x.min(y).min(width - 1 - x).min(height - 1 - y) as f32 + 1.0;
                distance[index] = computed.min(frame_edge);
            }
        }

        Self {
            width: coverage.width(),
            height: coverage.height(),
            distance,
        }
    }

    pub(crate) fn at(&self, x: u32, y: u32) -> f32 {
        if x >= self.width || y >= self.height {
            return 0.0;
        }
        self.distance[y as usize * self.width as usize + x as usize]
    }
}

pub(crate) fn bleed_color(frame: &RgbaFrame, x: u32, y: u32) -> [u8; 3] {
    let width = frame.width() as f32;
    let height = frame.height() as f32;
    let point_x = x as f32 + 0.5;
    let point_y = y as f32 + 0.5;
    let fallback = frame
        .pixel(x, y)
        .map(|pixel| [pixel[0], pixel[1], pixel[2]])
        .unwrap_or([0, 0, 0]);
    let dx = width * 0.5 - point_x;
    let dy = height * 0.5 - point_y;
    let length = dx.hypot(dy);
    if length <= f32::EPSILON {
        return fallback;
    }
    const BLEED_STEP: f32 = 3.0;
    let sample_x = (point_x + dx / length * BLEED_STEP).clamp(0.0, width - 1.0);
    let sample_y = (point_y + dy / length * BLEED_STEP).clamp(0.0, height - 1.0);
    frame
        .pixel(sample_x as u32, sample_y as u32)
        .map(|pixel| [pixel[0], pixel[1], pixel[2]])
        .unwrap_or(fallback)
}

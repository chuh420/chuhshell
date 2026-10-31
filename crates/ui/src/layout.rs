#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Item {
    pub width: f32,
    pub height: f32,
    pub margin_left: f32,
    pub margin_right: f32,
    pub preferred: bool,
}

impl Item {
    pub fn new(width: f32, height: f32) -> Self {
        Self {
            width,
            height,
            ..Self::default()
        }
    }

    pub fn preferred(width: f32, height: f32) -> Self {
        Self {
            width,
            height,
            preferred: true,
            ..Self::default()
        }
    }

    pub fn layout_width(&self) -> f32 {
        if self.preferred {
            self.width
        } else {
            self.width.ceil()
        }
    }

    pub fn layout_height(&self) -> f32 {
        if self.preferred {
            self.height
        } else {
            self.height.ceil()
        }
    }

    pub fn margins(mut self, left: f32, right: f32) -> Self {
        self.margin_left = left;
        self.margin_right = right;
        self
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct RowResult {
    pub positions: Vec<(f32, f32)>,
    pub width: f32,
    pub height: f32,
}

pub fn row_layout(items: &[Item], spacing: f32) -> RowResult {
    let height = items.iter().map(Item::layout_height).fold(0.0, f32::max);
    let mut x: f32 = 0.0;
    let mut positions = Vec::with_capacity(items.len());
    for (index, item) in items.iter().enumerate() {
        if index > 0 {
            x += spacing;
        }
        x += item.margin_left;
        x = x.round();
        positions.push((x, center(height, item.layout_height())));
        x += item.layout_width() + item.margin_right;
    }
    RowResult {
        positions,
        width: x.ceil(),
        height,
    }
}

pub fn row_widths(items: &[Item]) -> Vec<f32> {
    items.iter().map(Item::layout_width).collect()
}

pub fn plain_row(items: &[Item], spacing: f32) -> RowResult {
    let height = items.iter().map(|item| item.height).fold(0.0, f32::max);
    let mut x = 0.0;
    let mut positions = Vec::with_capacity(items.len());
    for (index, item) in items.iter().enumerate() {
        if index > 0 {
            x += spacing;
        }
        positions.push((x, 0.0));
        x += item.width;
    }
    RowResult {
        positions,
        width: x,
        height,
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Cell {
    pub size: f32,
    pub min: f32,
    pub max: f32,
    pub margin_start: f32,
    pub margin_end: f32,
}

impl Cell {
    pub fn implicit(size: f32) -> Self {
        Self::preferred(size.ceil())
    }

    pub fn preferred(size: f32) -> Self {
        Self {
            size,
            min: size,
            max: size,
            margin_start: 0.0,
            margin_end: 0.0,
        }
    }

    pub fn fill(size: f32) -> Self {
        Self {
            size,
            min: 0.0,
            max: f32::INFINITY,
            margin_start: 0.0,
            margin_end: 0.0,
        }
    }

    pub fn layout(size: f32) -> Self {
        Self::implicit(size)
    }

    pub fn margins(mut self, start: f32, end: f32) -> Self {
        self.margin_start = start;
        self.margin_end = end;
        self
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Slot {
    pub pos: f32,
    pub size: f32,
}

fn shrink_factor(desired: f64, available: f64, sum_desired: f64) -> f64 {
    desired * (available / sum_desired).powf(desired / sum_desired)
}

fn qround(value: f64) -> f64 {
    (value + 0.5).floor()
}

pub fn linear(cells: &[Cell], spacing: f32, available: f32) -> Vec<Slot> {
    let n = cells.len();
    if n == 0 {
        return Vec::new();
    }
    let spacing = spacing as f64;
    let boxes: Vec<(f64, f64, f64)> = cells
        .iter()
        .map(|cell| {
            let margins = (cell.margin_start + cell.margin_end) as f64;
            (
                cell.min as f64 + margins,
                cell.size as f64 + margins,
                cell.max as f64 + margins,
            )
        })
        .collect();
    let gaps = spacing * (n - 1) as f64;
    let total_min: f64 = boxes.iter().map(|b| b.0).sum::<f64>() + gaps;
    let total_pref: f64 = boxes.iter().map(|b| b.1).sum::<f64>() + gaps;
    let total_max: f64 = boxes.iter().map(|b| b.2).sum::<f64>() + gaps;
    let target = total_min.max(available as f64);
    let mut sizes: Vec<f64>;
    if target < total_pref {
        sizes = boxes.iter().map(|b| b.0).collect();
        let avail = target - total_min;
        if avail > 0.0 {
            let sum_desired = total_pref - total_min;
            let factors: Vec<f64> = boxes
                .iter()
                .map(|b| shrink_factor(b.1 - b.0, avail, sum_desired))
                .collect();
            let sum: f64 = factors.iter().sum();
            for (size, factor) in sizes.iter_mut().zip(&factors) {
                if sum > 0.0 {
                    *size += avail * factor / sum;
                }
            }
        }
    } else {
        let larger = target > total_max;
        sizes = boxes
            .iter()
            .map(|b| if larger { b.2 } else { b.1 })
            .collect();
        let mut avail = target - if larger { total_max } else { total_pref };
        if avail > 0.0 {
            let mut factors = vec![0.0; n];
            let mut grown: Vec<Option<f64>> = vec![None; n];
            for i in 0..n {
                let desired = if larger {
                    f64::INFINITY
                } else {
                    boxes[i].2 - boxes[i].1
                };
                if desired == 0.0 {
                    grown[i] = Some(sizes[i]);
                } else {
                    factors[i] = if sizes[i] == 0.0 { 1.0 } else { sizes[i] };
                }
            }
            let mut sum: f64 = factors.iter().sum();
            let mut again = true;
            while again && !larger {
                again = false;
                for i in 0..n {
                    if grown[i].is_some() || sum <= 0.0 {
                        continue;
                    }
                    let max = boxes[i].0.max(boxes[i].2.floor());
                    if sizes[i] + avail * factors[i] / sum >= max {
                        grown[i] = Some(max);
                        avail -= max - sizes[i];
                        sum -= factors[i];
                        again = avail > 0.0;
                        if !again {
                            break;
                        }
                    }
                }
            }
            for i in 0..n {
                sizes[i] = grown[i].unwrap_or_else(|| {
                    sizes[i]
                        + if sum == 0.0 {
                            0.0
                        } else {
                            avail * factors[i] / sum
                        }
                });
            }
        }
    }
    let mut positions = Vec::with_capacity(n);
    let mut offset = 0.0;
    for (i, size) in sizes.iter().enumerate() {
        if i > 0 {
            offset += spacing;
        }
        positions.push(offset);
        offset += size;
    }
    for i in 0..n {
        let old = positions[i];
        positions[i] = qround(old);
        let delta = positions[i] - old;
        sizes[i] -= delta;
        if i > 0 {
            sizes[i - 1] += delta;
        }
    }
    sizes[n - 1] = target - positions[n - 1];
    for i in 0..n {
        sizes[i] = boxes[i].0.max(qround(sizes[i]));
    }
    cells
        .iter()
        .zip(positions.iter().zip(&sizes))
        .map(|(cell, (pos, size))| Slot {
            pos: (pos + cell.margin_start as f64) as f32,
            size: (size - (cell.margin_start + cell.margin_end) as f64).max(0.0) as f32,
        })
        .collect()
}

pub fn center(outer: f32, inner: f32) -> f32 {
    ((outer - inner) / 2.0).round()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn row_with_spacing_and_margins() {
        let items = [
            Item::new(22.0, 22.0),
            Item::new(1.0, 12.0).margins(4.0, 6.0),
            Item::new(137.0, 22.0),
        ];
        let result = row_layout(&items, 3.0);
        assert_eq!(result.positions, vec![(0.0, 0.0), (29.0, 5.0), (39.0, 0.0)]);
        assert_eq!(result.width, 176.0);
        assert_eq!(row_widths(&items), vec![22.0, 1.0, 137.0]);
        assert_eq!(result.height, 22.0);
    }

    #[test]
    fn matches_qt_bar_right_row() {
        let chip = 22.59375;
        let items = [
            Item::new(135.0, 22.0).margins(2.0, 0.0),
            Item::new(49.0, 22.0),
            Item::new(chip, 22.0),
            Item::new(chip, 22.0),
            Item::new(chip, 22.0),
            Item::new(55.5625, 22.0),
            Item::new(1.0, 12.0).margins(5.0, 5.0),
            Item::new(chip, 22.0),
            Item::new(chip, 22.0),
        ];
        let result = row_layout(&items, 2.0);
        let xs: Vec<f32> = result.positions.iter().map(|p| p.0).collect();
        assert_eq!(
            xs,
            vec![2.0, 139.0, 190.0, 215.0, 240.0, 265.0, 328.0, 336.0, 361.0]
        );
        assert_eq!(result.positions[6].1, 5.0);
        assert_eq!(result.width, 384.0);
    }

    #[test]
    fn matches_qt_bar_left_row() {
        let items = [
            Item::new(22.59375, 22.0),
            Item::new(1.0, 12.0).margins(4.0, 6.0),
            Item::new(137.0, 22.0),
            Item::preferred(63.65, 16.0).margins(12.0, 0.0),
        ];
        let result = row_layout(&items, 3.0);
        let xs: Vec<f32> = result.positions.iter().map(|p| p.0).collect();
        assert_eq!(xs, vec![0.0, 30.0, 40.0, 192.0]);
        assert_eq!(result.positions[3].1, 3.0);
        assert_eq!(result.width, 256.0);
    }

    #[test]
    fn plain_row_ignores_height_centering() {
        let items = [Item::new(53.0, 13.0), Item::new(74.0, 15.0)];
        let result = plain_row(&items, 0.0);
        assert_eq!(result.positions, vec![(0.0, 0.0), (53.0, 0.0)]);
        assert_eq!(result.width, 127.0);
        assert_eq!(result.height, 15.0);
    }

    #[test]
    fn center_rounds() {
        assert_eq!(center(22.0, 13.0), 5.0);
        assert_eq!(center(28.69, 22.0), 3.0);
    }

    #[test]
    fn fixed_cells_overflow_instead_of_shrinking() {
        let slots = linear(&[Cell::preferred(30.0), Cell::preferred(32.0)], 5.0, 65.0);
        assert_eq!(
            slots[0],
            Slot {
                pos: 0.0,
                size: 30.0
            }
        );
        assert_eq!(
            slots[1],
            Slot {
                pos: 35.0,
                size: 32.0
            }
        );
    }

    #[test]
    fn fill_cells_shrink_like_qt() {
        let slots = linear(&[Cell::fill(30.0), Cell::fill(32.0)], 5.0, 65.0);
        assert_eq!(slots[0].size, 29.0);
        assert_eq!(slots[1].pos, 34.0);
    }

    #[test]
    fn extra_space_beyond_maximum_follows_sizes() {
        let slots = linear(&[Cell::layout(24.0), Cell::preferred(32.0)], 5.0, 65.0);
        assert_eq!(slots[0].size, 26.0);
        assert_eq!(
            slots[1],
            Slot {
                pos: 31.0,
                size: 34.0
            }
        );
    }

    #[test]
    fn linear_grows_fill_cells() {
        let slots = linear(&[Cell::fill(0.0), Cell::fill(0.0)], 10.0, 380.0);
        assert_eq!(slots[0].size, 185.0);
        assert_eq!(slots[1].pos, 195.0);
    }
}

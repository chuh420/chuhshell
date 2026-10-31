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
}

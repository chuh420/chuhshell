#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

impl Rect {
    pub const fn new(x: i32, y: i32, width: i32, height: i32) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.width <= 0 || self.height <= 0
    }

    pub fn right(&self) -> i32 {
        self.x + self.width
    }

    pub fn bottom(&self) -> i32 {
        self.y + self.height
    }

    pub fn touches(&self, other: &Rect) -> bool {
        self.x <= other.right()
            && other.x <= self.right()
            && self.y <= other.bottom()
            && other.y <= self.bottom()
    }

    pub fn union(&self, other: &Rect) -> Rect {
        if self.is_empty() {
            return *other;
        }
        if other.is_empty() {
            return *self;
        }
        let x = self.x.min(other.x);
        let y = self.y.min(other.y);
        Rect::new(
            x,
            y,
            self.right().max(other.right()) - x,
            self.bottom().max(other.bottom()) - y,
        )
    }

    pub fn intersect(&self, other: &Rect) -> Rect {
        let x = self.x.max(other.x);
        let y = self.y.max(other.y);
        let right = self.right().min(other.right());
        let bottom = self.bottom().min(other.bottom());
        if right <= x || bottom <= y {
            return Rect::default();
        }
        Rect::new(x, y, right - x, bottom - y)
    }
}

const MAX_RECTS: usize = 8;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Damage {
    rects: Vec<Rect>,
}

impl Damage {
    pub fn full(width: i32, height: i32) -> Self {
        let mut damage = Self::default();
        damage.add(Rect::new(0, 0, width, height));
        damage
    }

    pub fn add(&mut self, rect: Rect) {
        if rect.is_empty() {
            return;
        }
        let mut merged = rect;
        loop {
            let before = self.rects.len();
            self.rects.retain(|existing| {
                if existing.touches(&merged) {
                    merged = merged.union(existing);
                    false
                } else {
                    true
                }
            });
            if self.rects.len() == before {
                break;
            }
        }
        self.rects.push(merged);
        if self.rects.len() > MAX_RECTS {
            let bounds = self.bounds();
            self.rects.clear();
            self.rects.push(bounds);
        }
    }

    pub fn extend(&mut self, other: &Damage) {
        for rect in &other.rects {
            self.add(*rect);
        }
    }

    pub fn clip(&mut self, bounds: Rect) {
        let rects = std::mem::take(&mut self.rects);
        for rect in rects {
            self.add(rect.intersect(&bounds));
        }
    }

    pub fn bounds(&self) -> Rect {
        self.rects
            .iter()
            .fold(Rect::default(), |acc, rect| acc.union(rect))
    }

    pub fn rects(&self) -> &[Rect] {
        &self.rects
    }

    pub fn is_empty(&self) -> bool {
        self.rects.is_empty()
    }

    pub fn clear(&mut self) {
        self.rects.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn overlapping_rects_merge() {
        let mut damage = Damage::default();
        damage.add(Rect::new(0, 0, 10, 10));
        damage.add(Rect::new(5, 5, 10, 10));
        assert_eq!(damage.rects(), &[Rect::new(0, 0, 15, 15)]);
    }

    #[test]
    fn separate_rects_stay_separate() {
        let mut damage = Damage::default();
        damage.add(Rect::new(0, 0, 10, 10));
        damage.add(Rect::new(50, 0, 10, 10));
        assert_eq!(damage.rects().len(), 2);
    }

    #[test]
    fn bridging_rect_merges_chain() {
        let mut damage = Damage::default();
        damage.add(Rect::new(0, 0, 10, 10));
        damage.add(Rect::new(30, 0, 10, 10));
        damage.add(Rect::new(8, 0, 24, 10));
        assert_eq!(damage.rects(), &[Rect::new(0, 0, 40, 10)]);
    }

    #[test]
    fn too_many_rects_collapse() {
        let mut damage = Damage::default();
        for i in 0..=MAX_RECTS as i32 {
            damage.add(Rect::new(i * 20, 0, 5, 5));
        }
        assert_eq!(
            damage.rects(),
            &[Rect::new(0, 0, MAX_RECTS as i32 * 20 + 5, 5)]
        );
    }

    #[test]
    fn empty_rects_ignored() {
        let mut damage = Damage::default();
        damage.add(Rect::new(0, 0, 0, 10));
        assert!(damage.is_empty());
    }

    #[test]
    fn clip_drops_outside() {
        let mut damage = Damage::default();
        damage.add(Rect::new(-5, -5, 10, 10));
        damage.add(Rect::new(100, 100, 10, 10));
        damage.clip(Rect::new(0, 0, 50, 50));
        assert_eq!(damage.rects(), &[Rect::new(0, 0, 5, 5)]);
    }
}

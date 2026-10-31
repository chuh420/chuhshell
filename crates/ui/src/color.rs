#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Color {
    pub r: f32,
    pub g: f32,
    pub b: f32,
    pub a: f32,
}

impl Color {
    pub const TRANSPARENT: Color = Color {
        r: 0.0,
        g: 0.0,
        b: 0.0,
        a: 0.0,
    };

    pub const fn rgb(hex: u32) -> Color {
        Color {
            r: ((hex >> 16) & 0xff) as f32 / 255.0,
            g: ((hex >> 8) & 0xff) as f32 / 255.0,
            b: (hex & 0xff) as f32 / 255.0,
            a: 1.0,
        }
    }

    pub fn with_alpha(self, a: f32) -> Color {
        Color { a, ..self }
    }

    pub fn is_transparent(&self) -> bool {
        self.a <= 0.0
    }

    pub fn lerp(self, to: Color, t: f32) -> Color {
        Color {
            r: self.r + (to.r - self.r) * t,
            g: self.g + (to.g - self.g) * t,
            b: self.b + (to.b - self.b) * t,
            a: self.a + (to.a - self.a) * t,
        }
    }

    pub fn to_skia(self, opacity: f32) -> tiny_skia::Color {
        tiny_skia::Color::from_rgba(
            self.r.clamp(0.0, 1.0),
            self.g.clamp(0.0, 1.0),
            self.b.clamp(0.0, 1.0),
            (self.a * opacity).clamp(0.0, 1.0),
        )
        .unwrap_or(tiny_skia::Color::TRANSPARENT)
    }

    pub fn to_rgba8(self) -> [u8; 4] {
        let channel = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
        [
            channel(self.r),
            channel(self.g),
            channel(self.b),
            channel(self.a),
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_parses() {
        assert_eq!(Color::rgb(0x282828).to_rgba8(), [0x28, 0x28, 0x28, 0xff]);
    }

    #[test]
    fn lerp_midpoint() {
        let c = Color::rgb(0x000000).lerp(Color::rgb(0xffffff), 0.5);
        assert_eq!(c.to_rgba8(), [128, 128, 128, 255]);
    }
}

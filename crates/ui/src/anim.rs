use crate::Color;

pub type Millis = f64;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Easing {
    Linear,
    InQuad,
    OutQuad,
    InOutQuad,
    InCubic,
    OutCubic,
    InOutCubic,
    OutQuint,
    InOutSine,
    OutBack(f64),
    Bezier(f64, f64, f64, f64),
}

impl Easing {
    pub const OUT_BACK: Easing = Easing::OutBack(1.70158);
    pub const SPATIAL: Easing = Easing::Bezier(0.38, 1.21, 0.22, 1.0);
    pub const SPATIAL_FAST: Easing = Easing::Bezier(0.42, 1.67, 0.21, 0.9);
    pub const SPATIAL_SLOW: Easing = Easing::Bezier(0.39, 1.29, 0.35, 0.98);
    pub const EFFECTS: Easing = Easing::Bezier(0.34, 0.8, 0.34, 1.0);
    pub const STANDARD: Easing = Easing::Bezier(0.2, 0.0, 0.0, 1.0);
    pub const DECELERATE: Easing = Easing::Bezier(0.05, 0.7, 0.1, 1.0);
    pub const ACCELERATE: Easing = Easing::Bezier(0.3, 0.0, 0.8, 0.15);

    pub fn value(self, t: f64) -> f64 {
        let t = t.clamp(0.0, 1.0);
        match self {
            Easing::Linear => t,
            Easing::InQuad => t * t,
            Easing::OutQuad => -t * (t - 2.0),
            Easing::InOutQuad => {
                let t = t * 2.0;
                if t < 1.0 {
                    t * t / 2.0
                } else {
                    let t = t - 1.0;
                    -0.5 * (t * (t - 2.0) - 1.0)
                }
            }
            Easing::InCubic => t * t * t,
            Easing::OutCubic => {
                let t = t - 1.0;
                t * t * t + 1.0
            }
            Easing::InOutCubic => {
                let t = t * 2.0;
                if t < 1.0 {
                    0.5 * t * t * t
                } else {
                    let t = t - 2.0;
                    0.5 * (t * t * t + 2.0)
                }
            }
            Easing::OutQuint => {
                let t = t - 1.0;
                t * t * t * t * t + 1.0
            }
            Easing::InOutSine => -0.5 * ((std::f64::consts::PI * t).cos() - 1.0),
            Easing::OutBack(s) => {
                let t = t - 1.0;
                t * t * ((s + 1.0) * t + s) + 1.0
            }
            Easing::Bezier(x1, y1, x2, y2) => bezier(t, x1, y1, x2, y2),
        }
    }
}

fn bezier(x: f64, x1: f64, y1: f64, x2: f64, y2: f64) -> f64 {
    if x <= 0.0 {
        return 0.0;
    }
    if x >= 1.0 {
        return 1.0;
    }
    let curve = |t: f64, a: f64, b: f64| {
        let u = 1.0 - t;
        3.0 * u * u * t * a + 3.0 * u * t * t * b + t * t * t
    };
    let (mut low, mut high) = (0.0, 1.0);
    let mut t = x;
    for _ in 0..24 {
        let at = curve(t, x1, x2);
        if (at - x).abs() < 1e-5 {
            break;
        }
        if at < x {
            low = t;
        } else {
            high = t;
        }
        t = (low + high) / 2.0;
    }
    curve(t, y1, y2)
}

pub trait Lerp: Copy + PartialEq {
    fn lerp(from: Self, to: Self, t: f64) -> Self;
}

impl Lerp for f32 {
    fn lerp(from: Self, to: Self, t: f64) -> Self {
        (from as f64 + (to as f64 - from as f64) * t) as f32
    }
}

impl Lerp for f64 {
    fn lerp(from: Self, to: Self, t: f64) -> Self {
        from + (to - from) * t
    }
}

impl Lerp for Color {
    fn lerp(from: Self, to: Self, t: f64) -> Self {
        from.lerp(to, t as f32)
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Tween<T: Lerp> {
    value: T,
    from: T,
    to: T,
    start: Millis,
    duration: Millis,
    easing: Easing,
    running: bool,
}

impl<T: Lerp> Tween<T> {
    pub fn new(value: T, duration: Millis, easing: Easing) -> Self {
        Self {
            value,
            from: value,
            to: value,
            start: 0.0,
            duration,
            easing,
            running: false,
        }
    }

    pub fn value(&self) -> T {
        self.value
    }

    pub fn target(&self) -> T {
        self.to
    }

    pub fn is_running(&self) -> bool {
        self.running
    }

    pub fn set_duration(&mut self, duration: Millis) {
        self.duration = duration;
    }

    pub fn jump(&mut self, value: T) {
        self.value = value;
        self.from = value;
        self.to = value;
        self.running = false;
    }

    pub fn set_target(&mut self, now: Millis, to: T) {
        if self.running && self.to == to {
            return;
        }
        if !self.running && self.value == to {
            self.to = to;
            return;
        }
        self.start(now, self.value, to);
    }

    pub fn start(&mut self, now: Millis, from: T, to: T) {
        self.from = from;
        self.to = to;
        self.value = from;
        self.start = now;
        self.running = true;
        if self.duration <= 0.0 {
            self.value = to;
            self.running = false;
        }
    }

    pub fn tick(&mut self, now: Millis) -> bool {
        if !self.running {
            return false;
        }
        let t = ((now - self.start) / self.duration).clamp(0.0, 1.0);
        self.value = T::lerp(self.from, self.to, self.easing.value(t));
        if t >= 1.0 {
            self.value = self.to;
            self.running = false;
        }
        true
    }

    pub fn end_time(&self) -> Millis {
        self.start + self.duration
    }

    pub fn stop(&mut self) {
        self.running = false;
        self.to = self.value;
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Spring {
    value: f64,
    target: f64,
    velocity: f64,
    pub spring: f64,
    pub damping: f64,
    pub epsilon: f64,
    pub mass: f64,
    start: Millis,
    last: f64,
    skip: bool,
    running: bool,
}

const SPRING_STEP: f64 = 16.0;

impl Spring {
    pub fn new(value: f64, spring: f64, damping: f64) -> Self {
        Self {
            value,
            target: value,
            velocity: 0.0,
            spring,
            damping: damping.min(1.0),
            epsilon: 0.01,
            mass: 1.0,
            start: 0.0,
            last: 0.0,
            skip: false,
            running: false,
        }
    }

    pub fn value(&self) -> f64 {
        self.value
    }

    pub fn velocity(&self) -> f64 {
        self.velocity
    }

    pub fn is_running(&self) -> bool {
        self.running
    }

    pub fn jump(&mut self, value: f64) {
        self.value = value;
        self.target = value;
        self.velocity = 0.0;
        self.running = false;
    }

    pub fn start(&mut self, now: Millis, from: f64, to: f64) {
        self.skip = self.running;
        self.value = from;
        self.target = to;
        self.start = now;
        self.last = 0.0;
        self.running = true;
    }

    pub fn set_target(&mut self, now: Millis, to: f64) {
        if self.running && self.target == to {
            return;
        }
        let from = self.value;
        self.start(now, from, to);
    }

    pub fn stop(&mut self) {
        self.running = false;
    }

    pub fn set_value(&mut self, value: f64) {
        self.value = value;
    }

    pub fn tick(&mut self, now: Millis) -> bool {
        if !self.running {
            return false;
        }
        let time = (now - self.start).floor();
        if self.skip {
            self.skip = false;
            self.start = now;
            self.last = 0.0;
            return true;
        }
        let elapsed = time - self.last;
        if elapsed < SPRING_STEP {
            return true;
        }
        let count = (elapsed / SPRING_STEP).floor() as i64;
        self.last = time - (elapsed - count as f64 * SPRING_STEP);
        for _ in 0..count {
            let diff = self.target - self.value;
            if self.mass != 1.0 {
                self.velocity += (self.spring * diff - self.damping * self.velocity) / self.mass;
            } else {
                self.velocity += self.spring * diff - self.damping * self.velocity;
            }
            self.value += self.velocity * SPRING_STEP / 1000.0;
        }
        if self.velocity.abs() < self.epsilon && (self.target - self.value).abs() < self.epsilon {
            self.velocity = 0.0;
            self.value = self.target;
            self.running = false;
        }
        true
    }
}

#[derive(Clone, Copy, Debug)]
pub enum Step {
    To {
        value: f64,
        duration: Millis,
        easing: Easing,
    },
    Spring {
        value: f64,
        spring: f64,
        damping: f64,
    },
    Pause(Millis),
}

#[derive(Clone, Debug)]
pub struct Sequence {
    steps: Vec<Step>,
    index: usize,
    step_start: Millis,
    step_from: f64,
    value: f64,
    spring: Option<Spring>,
    running: bool,
}

impl Sequence {
    pub fn new(value: f64, steps: Vec<Step>) -> Self {
        Self {
            steps,
            index: 0,
            step_start: 0.0,
            step_from: value,
            value,
            spring: None,
            running: false,
        }
    }

    pub fn value(&self) -> f64 {
        self.value
    }

    pub fn is_running(&self) -> bool {
        self.running
    }

    pub fn restart(&mut self, now: Millis) {
        self.index = 0;
        self.step_start = now;
        self.step_from = self.value;
        self.running = !self.steps.is_empty();
        if let Some(spring) = self.spring.as_mut()
            && !spring.is_running()
        {
            self.spring = None;
        }
    }

    pub fn stop(&mut self) {
        self.running = false;
    }

    pub fn tick(&mut self, now: Millis) -> bool {
        if !self.running {
            return false;
        }
        while let Some(step) = self.steps.get(self.index).copied() {
            match step {
                Step::To {
                    value,
                    duration,
                    easing,
                } => {
                    let end = self.step_start + duration;
                    if now < end && duration > 0.0 {
                        let t = (now - self.step_start) / duration;
                        self.value = f64::lerp(self.step_from, value, easing.value(t));
                        return true;
                    }
                    self.value = value;
                    self.advance(end);
                }
                Step::Pause(duration) => {
                    let end = self.step_start + duration;
                    if now < end {
                        return true;
                    }
                    self.advance(end);
                }
                Step::Spring {
                    value,
                    spring,
                    damping,
                } => {
                    let start = self.step_start;
                    let from = self.value;
                    let state = self
                        .spring
                        .get_or_insert_with(|| Spring::new(from, spring, damping));
                    if !state.is_running() || state.target != value || state.start != start {
                        state.spring = spring;
                        state.damping = damping.min(1.0);
                        state.start(start, from, value);
                    }
                    state.tick(now);
                    self.value = state.value();
                    if state.is_running() {
                        return true;
                    }
                    self.advance(now);
                }
            }
        }
        self.running = false;
        true
    }

    fn advance(&mut self, at: Millis) {
        self.index += 1;
        self.step_start = at;
        self.step_from = self.value;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-9
    }

    #[test]
    fn easing_endpoints() {
        for easing in [
            Easing::Linear,
            Easing::InQuad,
            Easing::OutQuad,
            Easing::InOutQuad,
            Easing::InCubic,
            Easing::OutCubic,
            Easing::InOutCubic,
            Easing::OutQuint,
            Easing::InOutSine,
            Easing::OUT_BACK,
            Easing::SPATIAL,
            Easing::STANDARD,
            Easing::ACCELERATE,
        ] {
            assert!(close(easing.value(0.0), 0.0), "{easing:?}");
            assert!(close(easing.value(1.0), 1.0), "{easing:?}");
        }
    }

    #[test]
    fn bezier_curves_follow_their_shape() {
        let linear = Easing::Bezier(0.25, 0.25, 0.75, 0.75);
        assert!((linear.value(0.3) - 0.3).abs() < 1e-3);
        let peak = (1..100)
            .map(|step| Easing::SPATIAL_FAST.value(step as f64 / 100.0))
            .fold(0.0, f64::max);
        assert!(peak > 1.05 && peak < 1.3, "{peak}");
        assert!(Easing::ACCELERATE.value(0.5) < 0.2);
        assert!(Easing::DECELERATE.value(0.3) > 0.8);
    }

    #[test]
    fn easing_midpoints_match_qt() {
        assert!(close(Easing::OutCubic.value(0.5), 0.875));
        assert!(close(Easing::InQuad.value(0.5), 0.25));
        assert!(close(Easing::OutQuad.value(0.5), 0.75));
        assert!(close(Easing::InOutSine.value(0.5), 0.5));
        assert!(close(Easing::OUT_BACK.value(0.5), 1.0876975));
        assert!(Easing::OUT_BACK.value(0.8) > 1.0);
    }

    #[test]
    fn behavior_restarts_from_current_value() {
        let mut tween = Tween::new(0.0f64, 100.0, Easing::Linear);
        tween.set_target(0.0, 1.0);
        tween.tick(50.0);
        assert!(close(tween.value(), 0.5));
        tween.set_target(50.0, 0.0);
        tween.tick(100.0);
        assert!(close(tween.value(), 0.25));
        tween.tick(150.0);
        assert!(close(tween.value(), 0.0));
        assert!(!tween.is_running());
    }

    #[test]
    fn behavior_same_target_keeps_running() {
        let mut tween = Tween::new(0.0f64, 100.0, Easing::Linear);
        tween.set_target(0.0, 1.0);
        tween.tick(50.0);
        tween.set_target(50.0, 1.0);
        tween.tick(100.0);
        assert!(close(tween.value(), 1.0));
    }

    #[test]
    fn behavior_without_change_does_not_start() {
        let mut tween = Tween::new(1.0f64, 100.0, Easing::Linear);
        tween.set_target(0.0, 1.0);
        assert!(!tween.is_running());
    }

    #[test]
    fn spring_matches_qt_steps() {
        let mut spring = Spring::new(0.8, 5.0, 0.28);
        spring.start(0.0, 0.8, 1.0);
        spring.tick(10.0);
        assert!(close(spring.value(), 0.8));
        spring.tick(16.0);
        assert!(close(spring.value(), 0.816));
        spring.tick(33.0);
        assert!(close(spring.value(), 0.84224));
        let mut now = 33.0;
        while spring.is_running() && now < 10_000.0 {
            now += 16.0;
            spring.tick(now);
        }
        assert!(!spring.is_running());
        assert!(close(spring.value(), 1.0));
    }

    #[test]
    fn spring_carries_leftover_time() {
        let mut a = Spring::new(0.0, 4.0, 0.22);
        a.start(0.0, 1.18, 1.0);
        a.tick(40.0);
        let mut b = Spring::new(0.0, 4.0, 0.22);
        b.start(0.0, 1.18, 1.0);
        b.tick(32.0);
        assert!(close(a.value(), b.value()));
    }

    #[test]
    fn sequence_pop_runs_tween_then_spring() {
        let mut pop = Sequence::new(
            1.0,
            vec![
                Step::To {
                    value: 0.8,
                    duration: 70.0,
                    easing: Easing::OutQuad,
                },
                Step::Spring {
                    value: 1.0,
                    spring: 5.0,
                    damping: 0.28,
                },
            ],
        );
        pop.restart(0.0);
        pop.tick(35.0);
        assert!(close(pop.value(), 1.0 + (0.8 - 1.0) * 0.75));
        pop.tick(70.0);
        assert!(close(pop.value(), 0.8));
        pop.tick(86.0);
        assert!(close(pop.value(), 0.816));
        let mut now = 86.0;
        while pop.is_running() && now < 10_000.0 {
            now += 16.0;
            pop.tick(now);
        }
        assert!(close(pop.value(), 1.0));
    }

    #[test]
    fn sequence_bell_ring() {
        let steps = [
            (16.0, 70.0),
            (-12.0, 110.0),
            (7.0, 100.0),
            (-3.0, 90.0),
            (0.0, 80.0),
        ]
        .into_iter()
        .map(|(value, duration)| Step::To {
            value,
            duration,
            easing: Easing::Linear,
        })
        .collect();
        let mut ring = Sequence::new(0.0, steps);
        ring.restart(0.0);
        ring.tick(70.0);
        assert!(close(ring.value(), 16.0));
        ring.tick(125.0);
        assert!(close(ring.value(), 2.0));
        ring.tick(450.0);
        assert!(close(ring.value(), 0.0));
        assert!(!ring.is_running());
    }
}

const WHEEL_DECELERATION: f64 = 15000.0;
const MAX_VELOCITY: f64 = 2500.0;
const WHEEL_DISTANCE: f64 = 3.0 * 24.0;
const VELOCITY_SAMPLES: usize = 3;

#[derive(Clone, Debug, Default)]
pub struct Flick {
    value: f32,
    from: f32,
    target: f32,
    direction: f32,
    speed: f64,
    accel: f64,
    start: Millis,
    running: bool,
    samples: Vec<f64>,
    velocity: f64,
    last_wheel: Option<Millis>,
}

impl Flick {
    pub fn new(value: f32) -> Self {
        Self {
            value,
            from: value,
            target: value,
            ..Self::default()
        }
    }

    pub fn value(&self) -> f32 {
        self.value
    }

    pub fn target(&self) -> f32 {
        self.target
    }

    pub fn is_running(&self) -> bool {
        self.running
    }

    pub fn jump(&mut self, value: f32) {
        self.value = value;
        self.from = value;
        self.target = value;
        self.running = false;
    }

    pub fn stop(&mut self) {
        let value = self.value;
        self.jump(value);
    }

    pub fn wheel(&mut self, now: Millis, delta: i32, min: f32, max: f32) {
        if delta == 0 {
            return;
        }
        self.tick(now);
        let elapsed = self.last_wheel.map(|last| (now - last) / 1000.0);
        self.last_wheel = Some(now);
        if elapsed.is_some_and(|elapsed| elapsed <= 0.0) {
            return;
        }
        let elapsed = match elapsed {
            Some(elapsed) if self.running => elapsed,
            _ => {
                self.samples.clear();
                120.0 / (WHEEL_DECELERATION * 2.0 * WHEEL_DISTANCE).sqrt()
            }
        };
        let instant = delta as f64 / elapsed;
        if (instant < 0.0 && self.velocity > 0.0) || (instant > 0.0 && self.velocity < 0.0) {
            self.samples.clear();
        }
        self.samples
            .push(instant.clamp(-MAX_VELOCITY, MAX_VELOCITY));
        if self.samples.len() > VELOCITY_SAMPLES {
            self.samples.remove(0);
        }
        self.velocity = self.samples.iter().sum::<f64>() / self.samples.len() as f64;
        let velocity = self.velocity.clamp(-MAX_VELOCITY, MAX_VELOCITY);
        let distance = velocity * velocity / (WHEEL_DECELERATION * 2.0);
        let direction = if velocity > 0.0 { -1.0 } else { 1.0 };
        let unclamped = (self.value as f64 + direction * distance).round();
        if (unclamped - self.value as f64).abs() < 0.5 {
            return;
        }
        let accel = velocity * velocity / (2.0 * (unclamped - self.value as f64).abs());
        let target = (unclamped as f32).clamp(min, max.max(min));
        if (target - self.value).abs() < 0.01 {
            self.running = false;
            return;
        }
        self.from = self.value;
        self.target = target;
        self.direction = direction as f32;
        self.speed = velocity.abs();
        self.accel = accel;
        self.start = now;
        self.running = true;
    }

    pub fn tick(&mut self, now: Millis) -> bool {
        if !self.running {
            return false;
        }
        let t = ((now - self.start) / 1000.0).max(0.0);
        let stop = self.speed / self.accel;
        let travelled = if t >= stop {
            self.speed * stop - self.accel * stop * stop / 2.0
        } else {
            self.speed * t - self.accel * t * t / 2.0
        };
        let value = self.from + self.direction * travelled as f32;
        let reached = (value - self.target) * self.direction >= 0.0;
        if t >= stop || reached {
            self.value = self.target;
            self.running = false;
        } else {
            self.value = value;
        }
        true
    }
}

#[cfg(test)]
mod flick_tests {
    use super::*;

    #[test]
    fn single_notch_moves_three_lines() {
        let mut flick = Flick::new(0.0);
        flick.wheel(0.0, -120, 0.0, 1000.0);
        assert!(flick.is_running());
        let mut now = 0.0;
        while flick.is_running() {
            now += 8.0;
            flick.tick(now);
        }
        assert_eq!(flick.value(), 72.0);
        assert!(now < 120.0);
    }

    #[test]
    fn stops_at_bounds() {
        let mut flick = Flick::new(0.0);
        flick.wheel(0.0, 120, 0.0, 100.0);
        assert!(!flick.is_running());
        flick.wheel(500.0, -120, 0.0, 30.0);
        while flick.is_running() {
            flick.tick(1000.0);
        }
        assert_eq!(flick.value(), 30.0);
    }
}

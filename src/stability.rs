use image::{DynamicImage, GenericImageView};

const SAMPLE_STEP: usize = 4;
const PIXEL_DELTA: u8 = 18;
const CHANGE_RATIO: f32 = 0.010;
const REQUIRED_STABLE_FRAMES: u8 = 3;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrameEvent {
    None,
    Changed,
    Stable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum State {
    WaitingForInitialStability { stable_frames: u8 },
    Idle,
    Changing { stable_frames: u8 },
}

pub struct StabilityDetector {
    previous: Vec<u8>,
    state: State,
}

impl StabilityDetector {
    pub fn new() -> Self {
        Self {
            previous: Vec::new(),
            state: State::WaitingForInitialStability { stable_frames: 0 },
        }
    }

    pub fn update(&mut self, image: &DynamicImage) -> FrameEvent {
        let current = sampled_luma(image);
        if self.previous.len() != current.len() {
            self.previous = current;
            self.state = State::WaitingForInitialStability { stable_frames: 0 };
            return FrameEvent::Changed;
        }

        let difference = changed_ratio(&self.previous, &current);
        self.previous = current;

        let (next_state, event) = match self.state {
            State::Idle if difference >= CHANGE_RATIO => {
                (State::Changing { stable_frames: 0 }, FrameEvent::Changed)
            }
            State::Idle => (State::Idle, FrameEvent::None),
            State::WaitingForInitialStability { stable_frames }
            | State::Changing { stable_frames } => {
                if difference >= CHANGE_RATIO {
                    (State::Changing { stable_frames: 0 }, FrameEvent::None)
                } else {
                    let stable_frames = stable_frames + 1;
                    if stable_frames >= REQUIRED_STABLE_FRAMES {
                        (State::Idle, FrameEvent::Stable)
                    } else {
                        let state = match self.state {
                            State::WaitingForInitialStability { .. } => {
                                State::WaitingForInitialStability { stable_frames }
                            }
                            _ => State::Changing { stable_frames },
                        };
                        (state, FrameEvent::None)
                    }
                }
            }
        };
        self.state = next_state;
        event
    }

    pub fn is_active(&self) -> bool {
        !matches!(self.state, State::Idle)
    }
}

fn sampled_luma(image: &DynamicImage) -> Vec<u8> {
    let (width, height) = image.dimensions();
    let capacity = (width as usize / SAMPLE_STEP + 1) * (height as usize / SAMPLE_STEP + 1);
    let mut samples = Vec::with_capacity(capacity);
    for y in (0..height).step_by(SAMPLE_STEP) {
        for x in (0..width).step_by(SAMPLE_STEP) {
            let pixel = image.get_pixel(x, y).0;
            let luma =
                ((u16::from(pixel[0]) * 77 + u16::from(pixel[1]) * 150 + u16::from(pixel[2]) * 29)
                    >> 8) as u8;
            samples.push(luma);
        }
    }
    samples
}

fn changed_ratio(previous: &[u8], current: &[u8]) -> f32 {
    let changed = previous
        .iter()
        .zip(current)
        .filter(|(left, right)| left.abs_diff(**right) >= PIXEL_DELTA)
        .count();
    changed as f32 / current.len().max(1) as f32
}

#[cfg(test)]
mod tests {
    use image::{DynamicImage, Rgb, RgbImage};

    use super::{FrameEvent, StabilityDetector};

    fn solid(value: u8) -> DynamicImage {
        DynamicImage::ImageRgb8(RgbImage::from_pixel(64, 32, Rgb([value; 3])))
    }

    #[test]
    fn emits_once_after_initial_stability() {
        let mut detector = StabilityDetector::new();
        assert_eq!(detector.update(&solid(20)), FrameEvent::Changed);
        assert_eq!(detector.update(&solid(20)), FrameEvent::None);
        assert_eq!(detector.update(&solid(20)), FrameEvent::None);
        assert_eq!(detector.update(&solid(20)), FrameEvent::Stable);
        assert_eq!(detector.update(&solid(20)), FrameEvent::None);
    }

    #[test]
    fn waits_for_stability_after_change() {
        let mut detector = StabilityDetector::new();
        for _ in 0..4 {
            detector.update(&solid(20));
        }
        assert_eq!(detector.update(&solid(240)), FrameEvent::Changed);
        assert_eq!(detector.update(&solid(240)), FrameEvent::None);
        assert_eq!(detector.update(&solid(240)), FrameEvent::None);
        assert_eq!(detector.update(&solid(240)), FrameEvent::Stable);
    }

    #[test]
    fn tolerates_small_animated_regions() {
        let mut detector = StabilityDetector::new();
        for _ in 0..4 {
            detector.update(&solid(20));
        }
        assert_eq!(detector.update(&solid(240)), FrameEvent::Changed);
        let mut frame = solid(240).to_rgb8();
        frame.put_pixel(0, 0, Rgb([20; 3]));
        let animated = DynamicImage::ImageRgb8(frame);
        assert_eq!(detector.update(&animated), FrameEvent::None);
        assert_eq!(detector.update(&solid(240)), FrameEvent::None);
        assert_eq!(detector.update(&animated), FrameEvent::Stable);
    }
}

use std::time::{Duration, Instant};

use strsim::normalized_levenshtein;

const SAME_TEXT_THRESHOLD: f64 = 0.90;
const DUPLICATE_WINDOW: Duration = Duration::from_secs(5);

#[derive(Default)]
pub struct TextDeduplicator {
    previous: Option<(String, Instant)>,
}

impl TextDeduplicator {
    pub fn is_new(&mut self, text: &str) -> bool {
        self.is_new_at(text, Instant::now())
    }

    fn is_new_at(&mut self, text: &str, now: Instant) -> bool {
        let normalized = normalize(text);
        if normalized.is_empty() {
            return false;
        }
        if self
            .previous
            .as_ref()
            .is_some_and(|(previous, emitted_at)| {
                now.duration_since(*emitted_at) < DUPLICATE_WINDOW
                    && normalized_levenshtein(previous, &normalized) >= SAME_TEXT_THRESHOLD
            })
        {
            return false;
        }
        self.previous = Some((normalized, now));
        true
    }
}

fn normalize(text: &str) -> String {
    text.chars()
        .filter(|character| character.is_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use super::TextDeduplicator;

    #[test]
    fn suppresses_ocr_punctuation_jitter() {
        let mut dedup = TextDeduplicator::default();
        let now = Instant::now();
        assert!(dedup.is_new_at("Hello, world!", now));
        assert!(!dedup.is_new_at("Hello world.", now + Duration::from_secs(1)));
        assert!(dedup.is_new_at("A different sentence.", now + Duration::from_secs(2)));
        assert!(dedup.is_new_at("Hello, world!", now + Duration::from_secs(3)));
    }

    #[test]
    fn allows_the_same_dialogue_after_the_duplicate_window() {
        let mut dedup = TextDeduplicator::default();
        let now = Instant::now();
        assert!(dedup.is_new_at("You're a trainer, right?", now));
        assert!(dedup.is_new_at("You're a trainer, right?", now + Duration::from_secs(5)));
    }
}

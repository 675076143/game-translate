use strsim::normalized_levenshtein;

const SAME_TEXT_THRESHOLD: f64 = 0.90;

#[derive(Default)]
pub struct TextDeduplicator {
    previous: Option<String>,
}

impl TextDeduplicator {
    pub fn is_new(&mut self, text: &str) -> bool {
        let normalized = normalize(text);
        if normalized.is_empty() {
            return false;
        }
        if self.previous.as_ref().is_some_and(|previous| {
            normalized_levenshtein(previous, &normalized) >= SAME_TEXT_THRESHOLD
        }) {
            return false;
        }
        self.previous = Some(normalized);
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
    use super::TextDeduplicator;

    #[test]
    fn suppresses_ocr_punctuation_jitter() {
        let mut dedup = TextDeduplicator::default();
        assert!(dedup.is_new("Hello, world!"));
        assert!(!dedup.is_new("Hello world."));
        assert!(dedup.is_new("A different sentence."));
        assert!(dedup.is_new("Hello, world!"));
    }
}

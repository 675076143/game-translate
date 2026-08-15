use std::time::{Duration, Instant};

use strsim::normalized_levenshtein;

use crate::ocr::OcrResult;

const CONFIRM_DELAY: Duration = Duration::from_millis(250);
const SAME_THRESHOLD: f64 = 0.90;

pub struct CandidateConfirmer {
    pending: Option<(OcrResult, Instant)>,
}

impl CandidateConfirmer {
    pub fn new() -> Self {
        Self { pending: None }
    }

    pub fn propose(&mut self, result: OcrResult, now: Instant) {
        self.pending = Some((result, now + CONFIRM_DELAY));
    }

    pub fn cancel(&mut self) {
        self.pending = None;
    }

    pub fn is_due(&self, now: Instant) -> bool {
        self.pending
            .as_ref()
            .is_some_and(|pending| now >= pending.1)
    }

    pub fn remaining(&self, now: Instant) -> Option<Duration> {
        self.pending
            .as_ref()
            .map(|pending| pending.1.saturating_duration_since(now))
    }

    pub fn confirm(&mut self, result: OcrResult, now: Instant) -> Option<OcrResult> {
        let (previous, _) = self.pending.take()?;
        let similarity =
            normalized_levenshtein(&normalize(&previous.text), &normalize(&result.text));
        if similarity >= SAME_THRESHOLD {
            Some(if result.text.len() >= previous.text.len() {
                result
            } else {
                previous
            })
        } else {
            self.propose(result, now);
            None
        }
    }
}

fn normalize(text: &str) -> String {
    text.chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::CandidateConfirmer;
    use crate::ocr::OcrResult;
    use std::time::Instant;

    fn result(text: &str) -> OcrResult {
        OcrResult {
            text: text.into(),
            confidence: 90.0,
        }
    }

    #[test]
    fn requires_two_matching_results() {
        let now = Instant::now();
        let mut confirmer = CandidateConfirmer::new();
        confirmer.propose(result("The wild Rattata fainted!"), now);
        assert!(
            confirmer
                .confirm(result("The wild Rattata fainted!"), now)
                .is_some()
        );
    }

    #[test]
    fn replaces_an_incomplete_candidate() {
        let now = Instant::now();
        let mut confirmer = CandidateConfirmer::new();
        confirmer.propose(result("The wild Pidgey"), now);
        assert!(
            confirmer
                .confirm(result("The wild Pidgey used Tackle!"), now)
                .is_none()
        );
    }
}

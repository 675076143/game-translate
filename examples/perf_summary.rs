use std::{env, fs, process};

fn main() {
    let Some(path) = env::args().nth(1) else {
        eprintln!("usage: perf_summary LOG_FILE");
        process::exit(2);
    };
    let input = match fs::read_to_string(path) {
        Ok(input) => input,
        Err(error) => {
            eprintln!("cannot read log: {error}");
            process::exit(1);
        }
    };
    let mut totals = values(&input, "total_ms=");
    let mut translations = values(&input, "translate_ms=");
    let mut candidates = event_values(&input, "ocr-candidate", "elapsed_ms=");
    let mut confirmations = event_values(&input, "ocr-confirm", "elapsed_ms=");
    if totals.is_empty() {
        println!("no completed translations");
        return;
    }
    totals.sort_unstable();
    translations.sort_unstable();
    candidates.sort_unstable();
    confirmations.sort_unstable();
    println!("completed: {}", totals.len());
    print_distribution("end-to-end", &totals);
    if !translations.is_empty() {
        print_distribution("translation", &translations);
    }
    if !candidates.is_empty() {
        print_distribution("ocr-candidate", &candidates);
    }
    if !confirmations.is_empty() {
        print_distribution("ocr-confirm", &confirmations);
    }
    for source in ["template", "memory-cache", "file-cache", "model"] {
        let count = latest_session(&input)
            .lines()
            .filter(|line| {
                line.contains("\ttranslate-complete\t")
                    && line.contains(&format!("source={source}"))
            })
            .count();
        println!("source {source}: {count}");
    }
}

fn values(input: &str, key: &str) -> Vec<u64> {
    latest_session(input)
        .lines()
        .filter(|line| line.contains("\ttranslate-complete\t"))
        .filter_map(|line| value(line, key))
        .collect()
}

fn event_values(input: &str, event: &str, key: &str) -> Vec<u64> {
    latest_session(input)
        .lines()
        .filter(|line| line.contains(&format!("\t{event}\t")))
        .filter_map(|line| value(line, key))
        .collect()
}

fn latest_session(input: &str) -> &str {
    input
        .rfind("\tsession\tstarted")
        .and_then(|position| input[..position].rfind('\n').map(|start| start + 1))
        .map_or(input, |start| &input[start..])
}

fn value(line: &str, key: &str) -> Option<u64> {
    line.split_whitespace()
        .find_map(|field| field.strip_prefix(key)?.parse().ok())
}

fn print_distribution(label: &str, values: &[u64]) {
    let percentile = |percent: usize| values[(values.len() - 1) * percent / 100];
    let average = values.iter().sum::<u64>() / values.len() as u64;
    println!(
        "{label}: median={}ms p90={}ms p95={}ms average={}ms",
        percentile(50),
        percentile(90),
        percentile(95),
        average
    );
}

#[cfg(test)]
mod tests {
    use super::{event_values, values};

    #[test]
    fn extracts_completed_latency() {
        let input = "1\tsession\tstarted\n2\ttranslate-complete\tid=1 translate_ms=900 total_ms=1800\n3\tsession\tstarted\n4\ttranslate-complete\tid=2 translate_ms=200 total_ms=800\n";
        assert_eq!(values(input, "translate_ms="), vec![200]);
        assert_eq!(values(input, "total_ms="), vec![800]);
    }

    #[test]
    fn extracts_ocr_latency_from_latest_session() {
        let input = "1\tsession\tstarted\n2\tocr-candidate\telapsed_ms=900\n3\tsession\tstarted\n4\tocr-candidate\telapsed_ms=300\n5\tocr-confirm\telapsed_ms=400\n";
        assert_eq!(
            event_values(input, "ocr-candidate", "elapsed_ms="),
            vec![300]
        );
        assert_eq!(event_values(input, "ocr-confirm", "elapsed_ms="), vec![400]);
    }
}

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
    if totals.is_empty() {
        println!("no completed translations");
        return;
    }
    totals.sort_unstable();
    translations.sort_unstable();
    println!("completed: {}", totals.len());
    print_distribution("end-to-end", &totals);
    if !translations.is_empty() {
        print_distribution("translation", &translations);
    }
}

fn values(input: &str, key: &str) -> Vec<u64> {
    let latest_session = input
        .rfind("\tsession\tstarted")
        .and_then(|position| input[..position].rfind('\n').map(|start| start + 1))
        .map_or(input, |start| &input[start..]);
    latest_session
        .lines()
        .filter(|line| line.contains("\ttranslate-complete\t"))
        .filter_map(|line| {
            line.split_whitespace()
                .find_map(|field| field.strip_prefix(key)?.parse().ok())
        })
        .collect()
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
    use super::values;

    #[test]
    fn extracts_completed_latency() {
        let input = "1\tsession\tstarted\n2\ttranslate-complete\tid=1 translate_ms=900 total_ms=1800\n3\tsession\tstarted\n4\ttranslate-complete\tid=2 translate_ms=200 total_ms=800\n";
        assert_eq!(values(input, "translate_ms="), vec![200]);
        assert_eq!(values(input, "total_ms="), vec![800]);
    }
}

use std::io::{self, Write};

pub fn print_status(message: &str) {
    println!("\x1b[33m[状态] {message}\x1b[0m");
    flush();
}

pub fn print_error(message: &str) {
    eprintln!("\x1b[31m[错误] {message}\x1b[0m");
}

pub fn print_translation(original: &str, translated: &str) {
    println!("\n\x1b[37m{original}\x1b[0m");
    println!("\x1b[32m{translated}\x1b[0m");
    flush();
}

fn flush() {
    let _ = io::stdout().flush();
}

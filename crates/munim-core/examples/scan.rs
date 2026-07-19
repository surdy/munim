//! Dev smoke test: scan the real `~/.claude` and print the summary + a few records.
//! Run with `cargo run -p munim-core --example scan`. Not shipped in the app.

use munim_core::{collect, Pricing};
use std::path::PathBuf;

fn main() {
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .expect("HOME not set");
    let pricing = Pricing::embedded_default();
    let out = collect(&home, &pricing);

    println!("=== summary ===");
    println!("{}", serde_json::to_string_pretty(&out.summary).unwrap());
    println!("\nclaude session-days: {}", out.claude.len());
    println!("first 3 records:");
    for rec in out.claude.iter().take(3) {
        println!(
            "  {} {}  {:>8} in {:>8} out  ${:<8}  {}",
            rec.date, rec.time, rec.input_tokens, rec.output_tokens, rec.cost, rec.model
        );
    }
}

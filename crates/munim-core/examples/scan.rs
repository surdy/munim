//! Dev smoke test: scan the real `~/.claude` and print the summary + a few records.
//! Run with `cargo run -p munim-core --example scan`. Not shipped in the app.

use munim_core::{collect, Caches, Pricing};
use std::path::PathBuf;

fn main() {
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .expect("HOME not set");
    let pricing = Pricing::embedded_default();
    let res = collect(&home, &pricing, &Caches::empty());
    let out = &res.output;

    println!("=== summary ===");
    println!("{}", serde_json::to_string_pretty(&out.summary).unwrap());
    println!(
        "\nscan: {} parsed, {} skipped, {} preserved",
        res.stats.parsed, res.stats.skipped, res.stats.preserved
    );
    println!(
        "buckets — claude: {}, codex: {}, openclaw: {}",
        out.claude.len(),
        out.codex.len(),
        out.openclaw.len()
    );
    for rec in out.claude.iter().take(3) {
        println!(
            "  {} {}  {:>8} in {:>8} out  ${:<8}  {}",
            rec.date, rec.time, rec.input_tokens, rec.output_tokens, rec.cost, rec.model
        );
    }
}

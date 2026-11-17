//! `cargo run -p concordance-exegra --example render -- <spec.jsonc> [out.md]`: the Markdown
//! Exegra renders from the specification's sections.

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some(input) = args.first() else {
        eprintln!("usage: render <spec.jsonc> [out.md]");
        std::process::exit(2);
    };
    let text = std::fs::read_to_string(input).unwrap_or_else(|e| {
        eprintln!("{input}: {e}");
        std::process::exit(2)
    });
    let started = std::time::Instant::now();
    match concordance_exegra::render_markdown(&text) {
        Ok(md) => {
            eprintln!("rendered {} bytes in {:?}", md.len(), started.elapsed());
            match args.get(1) {
                Some(out) => std::fs::write(out, md).expect("write output"),
                None => print!("{md}"),
            }
        }
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(1);
        }
    }
}

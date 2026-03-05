use clap::Parser;

#[derive(Parser, Debug)]
#[command(name = "bop-dispatcher")]
struct Cli {
    /// Path to the .cards directory
    cards_dir: String,
}

fn main() -> anyhow::Result<()> {
    let _cli = Cli::parse();
    anyhow::bail!("not implemented")
}

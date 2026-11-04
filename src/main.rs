use clap::Parser;

#[derive(Parser, Debug)]
#[command(version, about = "Scaffold projects from templates and keep templates alive as projects evolve (Rust implementation)")]
struct Cli {
    /// Example positional argument
    input: Option<String>,

    /// Example flag
    #[arg(short, long)]
    verbose: bool,
}

fn main() {
    let cli = Cli::parse();
    if cli.verbose {
        eprintln!("{cli:?}");
    }
    println!("Hello from boilersync");
}

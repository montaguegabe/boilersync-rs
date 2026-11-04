//! `boilersync` command-line interface.

mod cli;

use std::process::ExitCode;

use boilersync_core::error::Error;
use clap::Parser;

fn main() -> ExitCode {
    let args = cli::Cli::parse();
    boilersync_core::logging::init(args.verbose);

    match cli::run(&args) {
        Ok(cli::Outcome::Success) => ExitCode::SUCCESS,
        Ok(cli::Outcome::Failure) => ExitCode::from(1),
        Err(Error::Usage(message)) => {
            eprintln!("Error: {message}");
            ExitCode::from(2)
        }
        Err(err) => {
            log::error!("{err}");
            ExitCode::from(1)
        }
    }
}

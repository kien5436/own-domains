use clap::Parser;
use own_domains_cli::{Cli, execute};

fn main() {
    let output = execute(Cli::parse());
    print!("{}", output.stdout);
    eprint!("{}", output.stderr);
    std::process::exit(output.exit_code);
}

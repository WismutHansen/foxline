mod service;

use std::process::ExitCode;

use clap::{Parser, Subcommand};

#[derive(Debug, Parser)]
#[command(name = "foxline", version, about = "Manage the Foxline voice runtime")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Manage the persistent Voice Gateway and speech runtime.
    Service(service::ServiceArgs),
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let result = match cli.command {
        Command::Service(args) => service::execute(args),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("Error: {error:#}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;

    #[test]
    fn cli_definition_is_valid() {
        Cli::command().debug_assert();
    }

    #[test]
    fn all_service_commands_parse() {
        for command in [
            "run", "start", "stop", "restart", "status", "enable", "disable",
        ] {
            Cli::try_parse_from(["foxline", "service", command])
                .unwrap_or_else(|error| panic!("failed to parse {command}: {error}"));
        }
    }
}

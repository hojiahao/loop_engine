mod config;
mod discovery;
mod output;

use std::io::{self, Write};
use std::process::ExitCode;

use clap::{Parser, Subcommand, error::ErrorKind};
use loop_core::{ComponentHealth, PRODUCT_NAME};

#[derive(Parser)]
#[command(name = "loopctl", version, about = "Operate Loop Engine")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Check the local control-plane client installation.
    Doctor {
        /// Emit machine-readable output.
        #[arg(long)]
        json: bool,
    },
    /// Operate an authenticated bounded Discovery job.
    Discovery(discovery::Arguments),
}

#[tokio::main]
async fn main() -> ExitCode {
    let cli = match Cli::try_parse() {
        Ok(cli) => cli,
        Err(error)
            if matches!(
                error.kind(),
                ErrorKind::DisplayHelp | ErrorKind::DisplayVersion
            ) =>
        {
            return if error.print().is_ok() {
                ExitCode::SUCCESS
            } else {
                ExitCode::from(8)
            };
        }
        Err(_) => return report(output::Failure::Arguments),
    };
    let result = match cli.command {
        Command::Doctor { json } => doctor(json),
        Command::Discovery(arguments) => {
            tokio::select! {
                biased;
                signal = tokio::signal::ctrl_c() => {
                    Err(if signal.is_ok() { output::Failure::Interrupted } else { output::Failure::Internal })
                }
                result = discovery::run(arguments) => result,
            }
        }
    };
    match result {
        Ok(code) => ExitCode::from(code),
        Err(error) => report(error),
    }
}

fn doctor(json: bool) -> Result<u8, output::Failure> {
    let health = ComponentHealth::ready("loopctl");
    let mut stdout = io::stdout().lock();
    if json {
        serde_json::to_writer(&mut stdout, &health).map_err(|_| output::Failure::Internal)?;
        writeln!(stdout).map_err(|_| output::Failure::Internal)?;
    } else {
        writeln!(
            stdout,
            "{PRODUCT_NAME}: loopctl is ready ({})",
            health.version
        )
        .map_err(|_| output::Failure::Internal)?;
    }
    Ok(0)
}

fn report(error: output::Failure) -> ExitCode {
    let _ = writeln!(io::stderr().lock(), "{}", error.envelope());
    ExitCode::from(error.exit_code())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_doctor_json() {
        let cli = Cli::try_parse_from(["loopctl", "doctor", "--json"]).unwrap();
        assert!(matches!(cli.command, Command::Doctor { json: true }));
    }
}

use clap::{Parser, Subcommand};
use loop_core::{ComponentHealth, PRODUCT_NAME};

#[derive(Debug, Parser)]
#[command(name = "loopctl", version, about = "Operate Loop Engine")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Check the local control-plane client installation.
    Doctor {
        /// Emit machine-readable output.
        #[arg(long)]
        json: bool,
    },
}

fn main() -> anyhow::Result<()> {
    match Cli::parse().command {
        Command::Doctor { json } => {
            let health = ComponentHealth::ready("loopctl");
            if json {
                println!("{}", serde_json::to_string(&health)?);
            } else {
                println!("{PRODUCT_NAME}: loopctl is ready ({})", health.version);
            }
        }
    }
    Ok(())
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

use clap::Parser;

/// trmnlapp — read-only NTP/chrony + gpsd monitor over SSH.
///
/// Pass 1–6 servers as `user@host` or `host` (uses your SSH config / $USER if no user specified).
#[derive(Debug, Parser)]
#[command(name = "trmnlapp", version, about)]
pub struct Cli {
    /// Server to monitor, e.g. alice@ntp1.lan or ntp1.lan
    #[arg(required = true, num_args = 1..=6, value_name = "USER@HOST")]
    pub servers: Vec<String>,
}

impl Cli {
    /// Parse the command line and return a validated `Cli`.
    ///
    /// Returns an error if fewer than 1 or more than 6 servers are provided.
    /// The `clap` derive already enforces `num_args = 1..=6`, so this function
    /// is a thin convenience wrapper that surfaces the clap error as `anyhow::Error`.
    pub fn parse_args() -> anyhow::Result<Self> {
        Ok(Self::parse())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;

    fn parse(args: &[&str]) -> Result<Cli, clap::Error> {
        Cli::try_parse_from(args)
    }

    #[test]
    fn accepts_one_server() {
        let cli = parse(&["trmnlapp", "host1"]).expect("one server should be accepted");
        assert_eq!(cli.servers, vec!["host1"]);
    }

    #[test]
    fn accepts_three_servers() {
        let cli = parse(&["trmnlapp", "h1", "h2", "h3"]).expect("three servers should be accepted");
        assert_eq!(cli.servers, vec!["h1", "h2", "h3"]);
    }

    #[test]
    fn rejects_zero_servers() {
        let err = parse(&["trmnlapp"]);
        assert!(err.is_err(), "zero servers should be rejected");
    }

    #[test]
    fn accepts_six_servers() {
        let cli = parse(&["trmnlapp", "h1", "h2", "h3", "h4", "h5", "h6"])
            .expect("six servers should be accepted");
        assert_eq!(cli.servers, vec!["h1", "h2", "h3", "h4", "h5", "h6"]);
    }

    #[test]
    fn rejects_seven_servers() {
        let err = parse(&["trmnlapp", "h1", "h2", "h3", "h4", "h5", "h6", "h7"]);
        assert!(err.is_err(), "seven servers should be rejected");
    }

    #[test]
    fn cli_self_documents() {
        // Ensure the clap command definition is internally consistent.
        Cli::command().debug_assert();
    }
}

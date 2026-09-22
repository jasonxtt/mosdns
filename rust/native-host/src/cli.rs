use std::ffi::{OsStr, OsString};
use std::path::PathBuf;

/// The only command supported by the Phase 5A native host at this boundary.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CliCommand {
    Start { config: PathBuf },
}

/// Deterministic command-line usage failure.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CliError {
    message: String,
}

impl CliError {
    fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl std::fmt::Display for CliError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "{}; usage: mosdns start -c <config>",
            self.message
        )
    }
}

impl std::error::Error for CliError {}

/// Parses `mosdns start -c <path>` and its `--config` spelling.
pub fn parse_args<I, S>(args: I) -> Result<CliCommand, CliError>
where
    I: IntoIterator<Item = S>,
    S: Into<OsString>,
{
    let mut args = args.into_iter().map(Into::into);
    let _program = args
        .next()
        .ok_or_else(|| CliError::new("missing program name"))?;
    let command = args
        .next()
        .ok_or_else(|| CliError::new("missing command"))?;
    if command != OsStr::new("start") {
        return Err(CliError::new("only the start command is supported"));
    }

    let option = args
        .next()
        .ok_or_else(|| CliError::new("missing -c/--config option"))?;
    if option != OsStr::new("-c") && option != OsStr::new("--config") {
        return Err(CliError::new("expected -c or --config"));
    }
    let config = args
        .next()
        .ok_or_else(|| CliError::new("missing config path"))?;
    if config.is_empty() {
        return Err(CliError::new("config path must not be empty"));
    }
    if args.next().is_some() {
        return Err(CliError::new("unexpected argument"));
    }

    Ok(CliCommand::Start {
        config: PathBuf::from(config),
    })
}

#[cfg(test)]
mod tests {
    use super::{CliCommand, parse_args};

    #[test]
    fn accepts_both_config_spellings() {
        assert_eq!(
            parse_args(["mosdns", "start", "-c", "forward.yaml"]),
            Ok(CliCommand::Start {
                config: "forward.yaml".into(),
            })
        );
        assert_eq!(
            parse_args(["mosdns", "start", "--config", "forward.yaml"]),
            Ok(CliCommand::Start {
                config: "forward.yaml".into(),
            })
        );
    }

    #[test]
    fn rejects_other_commands_and_extra_arguments() {
        assert!(parse_args(["mosdns", "run", "-c", "forward.yaml"]).is_err());
        assert!(parse_args(["mosdns", "start", "-c", "forward.yaml", "extra"]).is_err());
        assert!(parse_args(["mosdns", "start", "-c"]).is_err());
    }
}

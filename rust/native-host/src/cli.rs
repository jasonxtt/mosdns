use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};

/// Commands supported by the native host. `dir` is the `-d/--dir`
/// configuration root for relative plugin-argument paths.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CliCommand {
    Start {
        config: PathBuf,
        dir: Option<PathBuf>,
    },
    Check {
        config: PathBuf,
        dir: Option<PathBuf>,
    },
    Version,
    /// The update guard, started by the running host (not meant for people).
    UpdateGuard {
        transaction: PathBuf,
    },
}

/// Deterministic command-line usage failure.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CliError {
    message: String,
}

impl CliError {
    pub(crate) fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl std::fmt::Display for CliError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "{}; usage: mosdns start [-d <dir>] -c <config> | mosdns check [-d <dir>] -c <config> | mosdns version",
            self.message
        )
    }
}

impl std::error::Error for CliError {}

/// Parses `mosdns start|check [-d <dir>] -c <path>` and the `--config` /
/// `--dir` spellings, options in either order.
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
    if command == OsStr::new("version") {
        if args.next().is_some() {
            return Err(CliError::new("unexpected argument"));
        }
        return Ok(CliCommand::Version);
    }
    if command == OsStr::new("update-guard") {
        let (Some(flag), Some(path), None) = (args.next(), args.next(), args.next()) else {
            return Err(CliError::new("update-guard needs --transaction <file>"));
        };
        if flag != OsStr::new("--transaction") || path.is_empty() {
            return Err(CliError::new("update-guard needs --transaction <file>"));
        }
        return Ok(CliCommand::UpdateGuard {
            transaction: PathBuf::from(path),
        });
    }
    let check = command == OsStr::new("check");
    if command != OsStr::new("start") && !check {
        return Err(CliError::new("expected start, check or version"));
    }

    let mut config = None;
    let mut dir = None;
    while let Some(option) = args.next() {
        let (slot, name) = if option == OsStr::new("-c") || option == OsStr::new("--config") {
            (&mut config, "config")
        } else if option == OsStr::new("-d") || option == OsStr::new("--dir") {
            (&mut dir, "dir")
        } else {
            return Err(CliError::new("expected -c/--config or -d/--dir"));
        };
        let value = args
            .next()
            .ok_or_else(|| CliError::new(format!("missing {name} path")))?;
        if value.is_empty() {
            return Err(CliError::new(format!("{name} path must not be empty")));
        }
        if slot.replace(PathBuf::from(value)).is_some() {
            return Err(CliError::new(format!("duplicate {name} option")));
        }
    }
    let config = config.ok_or_else(|| CliError::new("missing -c/--config option"))?;
    Ok(if check {
        CliCommand::Check { config, dir }
    } else {
        CliCommand::Start { config, dir }
    })
}

/// Resolves the paths of a parsed command without changing the process
/// working directory. `-d` must name an existing directory; it becomes the
/// absolute configuration root, and a relative `-c` is read from inside it
/// (the reference host changes into `-d` before opening the configuration).
pub fn resolve_config_paths(
    config: &Path,
    dir: Option<&Path>,
) -> Result<(PathBuf, Option<PathBuf>), CliError> {
    let Some(dir) = dir else {
        return Ok((config.to_path_buf(), None));
    };
    if !dir.is_dir() {
        return Err(CliError::new(format!(
            "-d/--dir `{}` is not a directory",
            dir.display()
        )));
    }
    let root = if dir.is_absolute() {
        dir.to_path_buf()
    } else {
        std::env::current_dir()
            .map_err(|error| CliError::new(format!("cannot resolve -d/--dir: {error}")))?
            .join(dir)
    };
    let root = crate::special_groups::lexical_path(&root);
    let config = if config.is_absolute() {
        config.to_path_buf()
    } else {
        root.join(config)
    };
    Ok((config, Some(root)))
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
                dir: None,
            })
        );
        assert_eq!(
            parse_args(["mosdns", "start", "--config", "forward.yaml"]),
            Ok(CliCommand::Start {
                config: "forward.yaml".into(),
                dir: None,
            })
        );
    }

    #[test]
    fn check_accepts_both_config_spellings() {
        for option in ["-c", "--config"] {
            assert_eq!(
                parse_args(["mosdns", "check", option, "forward.yaml"]),
                Ok(CliCommand::Check {
                    config: "forward.yaml".into(),
                    dir: None,
                })
            );
        }
        assert!(parse_args(["mosdns", "check", "-c"]).is_err());
        assert!(parse_args(["mosdns", "check", "-c", "forward.yaml", "extra"]).is_err());
    }

    #[test]
    fn rejects_other_commands_and_extra_arguments() {
        assert!(parse_args(["mosdns", "run", "-c", "forward.yaml"]).is_err());
        assert!(parse_args(["mosdns", "start", "-c", "forward.yaml", "extra"]).is_err());
        assert!(parse_args(["mosdns", "start", "-c"]).is_err());
    }
}

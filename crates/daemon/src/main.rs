//! `protonsyncd`: the daemon. Started by systemd (see `packaging/`).
//!
//! Usage: `protonsyncd [--config FILE] [--socket PATH]`.
//! Log level via `RUST_LOG` (default `info`).
//!
//! `protonsyncd hash-password` asks for a password and prints its hash for
//! `[web] password_hash`.

use std::path::PathBuf;
use std::process::ExitCode;

use protonsync_daemon::app::{self, Options};
use tracing_subscriber::EnvFilter;

const USAGE: &str =
    "Usage: protonsyncd [--config FILE] [--socket PATH]\n       protonsyncd hash-password";
/// Minimum length of the web interface password.
const MIN_WEB_PASSWORD_LENGTH: usize = 12;

fn main() -> ExitCode {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .with_writer(std::io::stderr)
        .init();

    if std::env::args().nth(1).as_deref() == Some("hash-password") {
        return hash_password();
    }
    let options = match parse_args(std::env::args().skip(1)) {
        Ok(Some(options)) => options,
        Ok(None) => return ExitCode::SUCCESS,
        Err(message) => {
            tracing::error!("{message}\n{USAGE}");
            return ExitCode::from(2);
        }
    };
    let runtime = match tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(error) => {
            tracing::error!(%error, "cannot start the runtime");
            return ExitCode::FAILURE;
        }
    };
    match runtime.block_on(app::run(options)) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            tracing::error!("{error}");
            ExitCode::FAILURE
        }
    }
}

/// Asks twice for the web interface password and prints the hash.
/// Without a terminal, one line is read from standard input.
fn hash_password() -> ExitCode {
    use std::io::{IsTerminal, Write};

    let password = if std::io::stdin().is_terminal() {
        let first = rpassword::prompt_password(
            "Passwort für die Web-Oberfläche / web interface password: ",
        );
        let second = rpassword::prompt_password("Wiederholen / repeat: ");
        match (first, second) {
            (Ok(first), Ok(second)) if first == second => first,
            (Ok(_), Ok(_)) => {
                tracing::error!("Die Passwörter stimmen nicht überein / passwords do not match");
                return ExitCode::FAILURE;
            }
            (Err(error), _) | (_, Err(error)) => {
                tracing::error!(%error, "cannot read the password");
                return ExitCode::FAILURE;
            }
        }
    } else {
        let mut line = String::new();
        if let Err(error) = std::io::stdin().read_line(&mut line) {
            tracing::error!(%error, "cannot read the password");
            return ExitCode::FAILURE;
        }
        line.trim_end_matches(['\r', '\n']).to_owned()
    };
    if password.chars().count() < MIN_WEB_PASSWORD_LENGTH {
        tracing::error!(
            "Das Passwort braucht mindestens {MIN_WEB_PASSWORD_LENGTH} Zeichen / at least {MIN_WEB_PASSWORD_LENGTH} characters"
        );
        return ExitCode::FAILURE;
    }
    match protonsync_web::hash_password(&password) {
        Ok(hash) if writeln!(std::io::stdout(), "{hash}").is_ok() => ExitCode::SUCCESS,
        Ok(_) => ExitCode::FAILURE,
        Err(error) => {
            tracing::error!(%error, "could not create the hash");
            ExitCode::FAILURE
        }
    }
}

/// `Ok(None)` means: help or version printed, nothing else to do.
fn parse_args(mut args: impl Iterator<Item = String>) -> Result<Option<Options>, String> {
    let mut options = Options::default();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--config" => options.config = Some(path_value(&mut args, "--config")?),
            "--socket" => options.socket = Some(path_value(&mut args, "--socket")?),
            "--help" | "-h" => {
                eprintln!("{USAGE}");
                return Ok(None);
            }
            "--version" | "-V" => {
                eprintln!("protonsyncd {}", env!("CARGO_PKG_VERSION"));
                return Ok(None);
            }
            other => return Err(format!("unknown argument: {other}")),
        }
    }
    Ok(Some(options))
}

fn path_value(args: &mut impl Iterator<Item = String>, flag: &str) -> Result<PathBuf, String> {
    args.next()
        .map(PathBuf::from)
        .ok_or_else(|| format!("{flag} needs a value"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(args: &[&str]) -> Result<Option<Options>, String> {
        parse_args(args.iter().map(ToString::to_string))
    }

    #[test]
    fn parses_known_flags() {
        let options = parse(&["--config", "/etc/p.toml", "--socket", "/run/p.sock"])
            .unwrap()
            .unwrap();
        assert_eq!(options.config, Some(PathBuf::from("/etc/p.toml")));
        assert_eq!(options.socket, Some(PathBuf::from("/run/p.sock")));
        assert!(parse(&["--help"]).unwrap().is_none());
        assert!(parse(&["--config"]).is_err());
        assert!(parse(&["--unknown"]).is_err());
    }
}

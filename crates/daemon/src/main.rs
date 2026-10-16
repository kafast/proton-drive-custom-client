//! `protonsyncd`: der Daemon. Wird von systemd gestartet (siehe `packaging/`).
//!
//! Aufruf: `protonsyncd [--config DATEI] [--socket PFAD]`.
//! Protokollstufe über `RUST_LOG` (Standard `info`).

use std::path::PathBuf;
use std::process::ExitCode;

use protonsync_daemon::app::{self, Options};
use tracing_subscriber::EnvFilter;

const USAGE: &str = "Aufruf: protonsyncd [--config DATEI] [--socket PFAD]";

fn main() -> ExitCode {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .with_writer(std::io::stderr)
        .init();

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
            tracing::error!(%error, "Laufzeit nicht startbar");
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

/// `Ok(None)` heißt: Hilfe oder Version ausgegeben, nichts weiter tun.
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
            other => return Err(format!("Unbekanntes Argument: {other}")),
        }
    }
    Ok(Some(options))
}

fn path_value(args: &mut impl Iterator<Item = String>, flag: &str) -> Result<PathBuf, String> {
    args.next()
        .map(PathBuf::from)
        .ok_or_else(|| format!("{flag} braucht einen Wert"))
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
        assert!(parse(&["--unbekannt"]).is_err());
    }
}

//! `protonsync` – Bedienung des Daemons von der Kommandozeile.

use std::io::{self, BufRead, Write};
use std::process::ExitCode;

use clap::Parser;
use protonsync_api::wire::default_socket_path;
use protonsync_cli::client::SocketTransport;
use protonsync_cli::commands::{self, Cli, Command, Prompter};
use protonsync_cli::i18n::Lang;

/// Fragt im Terminal; Geheimnisse ohne Echo oder (mit `--password-stdin`)
/// zeilenweise von der Standardeingabe, damit sie nie in der Prozessliste stehen.
struct TerminalPrompter {
    secrets_from_stdin: bool,
}

fn read_line() -> io::Result<String> {
    let mut line = String::new();
    io::stdin().lock().read_line(&mut line)?;
    Ok(line.trim_end_matches(['\r', '\n']).to_owned())
}

impl Prompter for TerminalPrompter {
    fn ask(&mut self, label: &str) -> io::Result<String> {
        let mut err = io::stderr();
        write!(err, "{label}: ")?;
        err.flush()?;
        read_line()
    }

    fn ask_secret(&mut self, label: &str) -> io::Result<String> {
        if self.secrets_from_stdin {
            read_line()
        } else {
            rpassword::prompt_password(format!("{label}: "))
        }
    }
}

fn main() -> ExitCode {
    let cli = match Cli::try_parse() {
        Ok(cli) => cli,
        Err(error) => {
            let _ = error.print();
            return ExitCode::from(if error.use_stderr() {
                commands::exit::USAGE
            } else {
                commands::exit::OK
            });
        }
    };
    let secrets_from_stdin = matches!(
        cli.command,
        Command::Login {
            password_stdin: true,
            ..
        }
    );
    let mut prompter = TerminalPrompter { secrets_from_stdin };
    let socket = cli.socket.clone().unwrap_or_else(default_socket_path);
    let code = commands::run(
        &cli,
        || SocketTransport::connect(&socket),
        &mut prompter,
        &mut io::stdout(),
        &mut io::stderr(),
        Lang::from_env(),
    );
    ExitCode::from(code)
}

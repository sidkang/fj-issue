use std::io::{self, Write};
use std::process::ExitCode;

use clap::{Parser, error::ErrorKind};

use fj_issue::cli::Cli;
use fj_issue::config::Context;
use fj_issue::error::FjiError;
use fj_issue::execute;

fn install_sigpipe() {
    #[cfg(unix)]
    unsafe {
        libc::signal(libc::SIGPIPE, libc::SIG_DFL);
    }
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> ExitCode {
    install_sigpipe();
    let cli = match Cli::try_parse() {
        Ok(cli) => cli,
        Err(err) => match err.kind() {
            ErrorKind::DisplayHelp | ErrorKind::DisplayVersion => {
                let _ = err.print();
                return ExitCode::SUCCESS;
            }
            _ => {
                FjiError::usage("usage", err.to_string()).write_stderr();
                return ExitCode::from(1);
            }
        },
    };

    let ctx = Context::from_env(cli.host.clone(), cli.repo.clone(), cli.cwd.clone());
    let command = match cli.command.into_command() {
        Ok(command) => command,
        Err(err) => {
            err.write_stderr();
            return ExitCode::from(err.exit_code() as u8);
        }
    };

    match execute(command, ctx).await {
        Ok(value) => {
            let mut stdout = io::stdout().lock();
            if serde_json::to_writer(&mut stdout, &value).is_err() {
                return ExitCode::from(2);
            }
            let _ = stdout.write_all(b"\n");
            ExitCode::SUCCESS
        }
        Err(err) => {
            err.write_stderr();
            ExitCode::from(err.exit_code() as u8)
        }
    }
}

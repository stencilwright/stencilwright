//! `stencilwright` — privacy-preserving site mapping harness.

use std::path::PathBuf;

use anyhow::Result;
use clap::Parser;

mod browser;
mod cli;
mod config;
mod config_lock;
mod dispatch;
mod elements;
mod init;
mod load;
mod page_cmd;
mod place_edit;
mod places;
mod session;
mod values;

use cli::{Cli, Command, SessionOp};

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| {
                "stencilwright=info,stencil_browser=info,stencil_places=info".into()
            }),
        )
        .init();

    match Cli::parse().command {
        Command::Init { site } => init::run(&site),
        Command::Load {
            site,
            source,
            force,
        } => {
            let source = source.unwrap_or_else(|| PathBuf::from("./stencils").join(&site));
            load::run(&site, &source, force)
        }
        Command::Session { op } => match op {
            SessionOp::Start { site } => session::start(&session::site_dir(&site)).await,
            SessionOp::Stop { site } => session::stop(&session::site_dir(&site)).await,
            SessionOp::Status { site } => session::status(&session::site_dir(&site)).await,
        },
        Command::Daemon { site_dir } => stencil_browser::daemon::run(site_dir).await,
        Command::Site(args) => dispatch::site(args).await,
    }
}

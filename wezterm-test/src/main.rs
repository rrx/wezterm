use anyhow::{anyhow, Context};
use chrono::{DateTime, Utc};
use config::keyassignment::SpawnTabDomain;
use config::{UnixDomain, wezterm_version};
use mux::activity::Activity;
use mux::pane::PaneId;
use mux::tab::SplitDirection;
use mux::window::WindowId;
use mux::Mux;
use portable_pty::cmdbuilder::CommandBuilder;
use std::ffi::OsString;
use std::io::{Read, Write};
use std::rc::Rc;
use tabout::{tabulate_output, Alignment, Column};
use umask::UmaskSaver;
use wezterm_client::client::{unix_connect_with_retry, Client, AsyncReadAndWrite, ReaderMessage};
use wezterm_gui_subcommands::*;
use codec::Pdu;
use futures::FutureExt;

mod app;
use app::Application;

//    let message = "; ❤ 😍🤢\n\x1b[91;mw00t\n\x1b[37;104;m bleet\x1b[0;m.";

use termwiz::escape::osc::{
    ITermDimension, ITermFileData, ITermProprietary, OperatingSystemCommand,
};

fn terminate_with_error_message(err: &str) -> ! {
    log::error!("{}; terminating", err);
    std::process::exit(1);
}

fn terminate_with_error(err: anyhow::Error) -> ! {
    terminate_with_error_message(&format!("{:#}", err));
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    env_bootstrap::bootstrap();
    let mut a = Application::new()?;
    a.run().await?;
    Mux::shutdown();
    Ok(())
}

async fn run_cli_async2(config: config::ConfigHandle, client: &Client) -> anyhow::Result<()> {

    /*
        CliSubCommand::SendText { pane_id, text } => {
            let pane_id: PaneId = match pane_id {
                Some(p) => p,
                None => std::env::var("WEZTERM_PANE")
                    .map_err(|_| {
                        anyhow!(
                            "--pane-id was not specified and $WEZTERM_PANE \
                             is not set in the environment."
                        )
                    })?
                    .parse()?,
            };
            let data = match text {
                Some(text) => text,
                None => {
                    let mut text = String::new();
                    std::io::stdin()
                        .read_to_string(&mut text)
                        .context("reading stdin")?;
                    text
                }
            };

            client
                .send_paste(codec::SendPaste { pane_id, data })
                .await?;
        }
    */
    // server is spawned, now we read and dump
    drop(client);

    let mux = Rc::new(mux::Mux::new(None));
    Mux::set_mux(&mux);
    let unix_dom = UnixDomain::default();
    let target = unix_dom.target();
    let u_stream = unix_connect_with_retry(&target, false, None)?;
    let mut from_stream = Box::new(smol::Async::new(u_stream)?);

    // Spawn a thread to pull data from the socket and write
    // it to stdout
    //let mut from_stream = stream.try_clone()?;
    let activity = Activity::new();
    let stdout = std::io::stdout();
    let mut buf = [0u8; 8192];
    loop {
        match from_stream.readable().await {
            Ok(()) => match Pdu::decode_async(&mut from_stream).await {
                Ok(decoded) => {
                    log::info!("{:?}", decoded);
                }
                Err(err) => {
                    let reason = format!("Error while decoding response pdu: {:#}", err);
                    log::error!("{}", reason);
                }
            }
            _ =>  break
        }
    }
    std::thread::sleep(std::time::Duration::new(2, 0));
    drop(activity);
    //std::process::exit(0);
    Ok(())
}


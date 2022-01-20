use anyhow::{anyhow, Context};
use chrono::{DateTime, Utc};
use config::keyassignment::SpawnTabDomain;
use config::wezterm_version;
use mux::activity::Activity;
use mux::pane::PaneId;
use mux::tab::SplitDirection;
use mux::window::WindowId;
use mux::Mux;
use portable_pty::cmdbuilder::CommandBuilder;
use std::ffi::OsString;
use std::io::{Read, Write};
use std::rc::Rc;
use structopt::StructOpt;
use tabout::{tabulate_output, Alignment, Column};
use umask::UmaskSaver;
use wezterm_client::client::{unix_connect_with_retry, Client, AsyncReadAndWrite, ReaderMessage};
use wezterm_gui_subcommands::*;
use codec::Pdu;
use futures::FutureExt;

mod app;
use app::Application;

//    let message = "; ❤ 😍🤢\n\x1b[91;mw00t\n\x1b[37;104;m bleet\x1b[0;m.";

#[derive(Debug, StructOpt)]
#[structopt(
    about = "Wez's Terminal Emulator\nhttp://github.com/wez/wezterm",
    global_setting = structopt::clap::AppSettings::ColoredHelp,
    version = wezterm_version()
)]
struct Opt {
    /// Skip loading wezterm.lua
    #[structopt(name = "skip-config", short = "n")]
    skip_config: bool,

    /// Specify the configuration file to use, overrides the normal
    /// configuration file resolution
    #[structopt(
        long = "config-file",
        parse(from_os_str),
        conflicts_with = "skip-config"
    )]
    config_file: Option<OsString>,

    /// Override specific configuration values
    #[structopt(
        long = "config",
        name = "name=value",
        parse(try_from_str = name_equals_value),
        number_of_values = 1)]
    config_override: Vec<(String, String)>,

    /// Don't automatically start the server
    #[structopt(long = "no-auto-start")]
    no_auto_start: bool,

    /// Prefer connecting to a background mux server.
    /// The default is to prefer connecting to a running
    /// wezterm gui instance
    #[structopt(long = "prefer-mux")]
    prefer_mux: bool,

    /// When connecting to a gui instance, if you started the
    /// gui with `--class SOMETHING`, you should also pass
    /// that same value here in order for the client to find
    /// the correct gui instance.
    #[structopt(long = "class")]
    class: Option<String>
}

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

fn main() {
    config::designate_this_as_the_main_thread();
    config::assign_error_callback(mux::connui::show_configuration_error_message);
    if let Err(e) = run() {
        terminate_with_error(e);
    }
    Mux::shutdown();
}

fn run() -> anyhow::Result<()> {
    env_bootstrap::bootstrap();

    let saver = UmaskSaver::new();

    let opts = Opt::from_args();
    config::common_init(
        opts.config_file.as_ref(),
        &opts.config_override,
        opts.skip_config,
    );
    let config = config::configuration();

    let mut ui = mux::connui::ConnectionUI::new_headless();
    let initial = true;

    let client = Client::new_default_unix_domain(
        initial,
        &mut ui,
        opts.no_auto_start,
        opts.prefer_mux,
        opts.class
            .as_deref()
            .unwrap_or(wezterm_gui_subcommands::DEFAULT_WINDOW_CLASS),
    )?;

    run_cli(config, &client)
}

async fn run_cli_async(config: config::ConfigHandle, client: &Client) -> anyhow::Result<()> {
    let mut a = Application::new()?;
    let code = a.run().await?;
    //Ok(())

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
    let unix_dom = config.unix_domains.first().unwrap();
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

fn run_cli(config: config::ConfigHandle, client: &Client) -> anyhow::Result<()> {
    let executor = promise::spawn::ScopedExecutor::new();
    match promise::spawn::block_on(executor.run(async move { run_cli_async(config, client).await })) {
        Ok(_) => Ok(()),
        Err(err) => terminate_with_error(err),
    }
}

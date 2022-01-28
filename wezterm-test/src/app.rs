use anyhow::{anyhow, Context};
use log::{error, warn};
use futures::StreamExt;

use std::{
    io::{stdin, stdout, Write},
    sync::Arc,
    time::{Duration, Instant},
};

use mux::Mux;
use std::rc::Rc;

use anyhow::Error;

use crossterm::{
    event::{DisableMouseCapture, EnableMouseCapture, Event, KeyEvent, EventStream, MouseEventKind, MouseEvent,
        KeyModifiers, KeyCode,
    },
    execute, terminal,
    tty::IsTty,
};
use codec::Pdu;

use umask::UmaskSaver;
use wezterm_client::client::{unix_connect_with_retry, Client, AsyncReadAndWrite, ReaderMessage};
use config::{UnixDomain, wezterm_version};

#[cfg(not(windows))]
use {
    signal_hook::{consts::signal, low_level},
};
#[cfg(windows)]
type Signals = futures_util::stream::Empty<()>;

pub struct Application {
    client: Client,
    enable_mouse: bool
}

#[derive(Debug)]
enum Msg {
    Signal(i32),
    Input(Event),
    Quit
}

impl Application {
    pub fn new() -> Result<Self, Error> {
        let saver = UmaskSaver::new();

        let mut ui = mux::connui::ConnectionUI::new_headless();
        let initial = true;

        let client = Client::new_default_unix_domain(
            initial,
            &mut ui,
            false, //opts.no_auto_start,
            true, //opts.prefer_mux,
            wezterm_gui_subcommands::DEFAULT_WINDOW_CLASS)?;

        Ok(Self {
            client,
            enable_mouse: true
        })
    }

    pub async fn event_loop(&mut self) -> Result<(), Error> {
        use signal_hook::iterator::{Signals, SignalsInfo};
        //use signal_hook::iterator::exfiltrator::origin::WithOrigin;
        use signal_hook::low_level;

        let mut reader = EventStream::new().map(|e| {
            log::info!("key: {:?}", e);
            match e {
                // handle Ctrl-C to exit for now
                Ok(Event::Key(KeyEvent {
                    code: KeyCode::Char('c'),
                    modifiers: KeyModifiers::CONTROL,
                })) => Msg::Quit,

                Ok(Event::Key(KeyEvent {
                    code: KeyCode::Char('q'),
                    modifiers: KeyModifiers::NONE,
                })) => Msg::Quit,

                // handle suspend
                Ok(Event::Key(KeyEvent {
                    code: KeyCode::Char('z'),
                    modifiers: KeyModifiers::CONTROL,
                })) => Msg::Signal(signal::SIGTSTP),

                Ok(event) => Msg::Input(event),
                Err(e) => Msg::Quit
            }
        });
   
        let (tx, rx) = smol::channel::unbounded();
 
        // handle signals in a separate thread
        let mut sigs = Signals::new(&[signal::SIGHUP, signal::SIGTSTP, signal::SIGCONT]).unwrap();
        let signals_handle = sigs.handle();
        let _ = std::thread::spawn(move || {
            for sig in sigs.forever() {
                log::info!("signal: {:?}", sig);
                match sig {
                    signal::SIGHUP | signal::SIGCONT | signal::SIGTSTP => {
                        tx.try_send(Msg::Signal(sig)).unwrap();
                    }
                    _ => {
                        log::info!("unhandled signal: {:?}", sig);
                        break;
                    }
                }
            }
            log::info!("signals thread exit");
        });

        let mux = Rc::new(mux::Mux::new(None));
        Mux::set_mux(&mux);
        let unix_dom = UnixDomain::default();
        let target = unix_dom.target();
        let mut u_stream = unix_connect_with_retry(&target, false, None)?;
        let mut from_stream = Box::new(smol::Async::new(u_stream)?);

        let mut stream = smol::stream::race(reader, rx);

        loop {
            tokio::select! {
                result = from_stream.readable() => {
                    match result {
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

                result = stream.next() => {
                    match result {
                        Some(Msg::Input(event)) => self.handle_terminal_events(Some(Ok(event))),
                        Some(Msg::Signal(sig)) => self.handle_signals(sig).await,
                        Some(Msg::Quit) => {
                            log::info!("quit");
                            break;
                        }
                        Some(m) => log::info!("{:?}", m),
                        None => {
                            log::info!("none");
                            break;
                        }
                    }
                }
            }
        }

        log::info!("end loop");

        // cancel the signals thread
        signals_handle.close();
        Ok(())
    }

    #[cfg(windows)]
    // no signal handling available on windows
    pub async fn handle_signals(&mut self, _signal: ()) {}

    #[cfg(not(windows))]
    pub async fn handle_signals(&mut self, signal: i32) {
        match signal {
            signal::SIGTSTP => {
                log::info!("Stop");
                //self.compositor.save_cursor();
                self.restore_term().unwrap();
                low_level::emulate_default_handler(signal::SIGTSTP).unwrap();
            }
            signal::SIGCONT => {
                log::info!("Continue");
                self.claim_term().await.unwrap();
                // redraw the terminal
                //let Rect { width, height, .. } = self.compositor.size();
                //self.compositor.resize(width, height);
                //self.compositor.load_cursor();
                //self.render();
            }
            _ => unreachable!(),
        }
    }
    
    pub fn handle_terminal_events(&mut self, event: Option<Result<Event, crossterm::ErrorKind>>) {
        //let mut cx = crate::compositor::Context {
            //editor: &mut self.editor,
            //jobs: &mut self.jobs,
            //scroll: None,
        //};
        // Handle key events
        match event {
            Some(Ok(Event::Resize(w, h))) => {
                log::info!("Resize: {:?}", (w, h));
            }

            Some(Ok(Event::Mouse(MouseEvent {
                kind,
                column,
                row,
                modifiers: _,
            }))) => { 
                log::info!("Mouse: {:?}", kind);
            }
            Some(Ok(Event::Key(KeyEvent { code, modifiers }))) => {
                log::info!("Input: {:?}", (code, modifiers));
            }
            _ => log::info!("Input: {:?}", event)
        }
        //let should_redraw = match event {
            //Some(Ok(Event::Resize(width, height))) => {
                //self.compositor.resize(width, height);

                //self.compositor
                    //.handle_event(Event::Resize(width, height), &mut cx)
            //}
            //Some(Ok(event)) => self.compositor.handle_event(event, &mut cx),
            //Some(Err(x)) => panic!("{}", x),
            //None => panic!(),
        //};

        //if should_redraw && !self.editor.should_close() {
            //self.render();
        //}
    }

    async fn claim_term(&mut self) -> Result<(), Error> {
        terminal::enable_raw_mode()?;
        let mut stdout = stdout();
        execute!(stdout, terminal::EnterAlternateScreen)?;
        if self.enable_mouse {
            execute!(stdout, EnableMouseCapture)?;
        }
        Ok(())
    }

    fn restore_term(&mut self) -> Result<(), Error> {
        let mut stdout = stdout();
        // reset cursor shape
        write!(stdout, "\x1B[2 q")?;
        // Ignore errors on disabling, this might trigger on windows if we call
        // disable without calling enable previously
        let _ = execute!(stdout, DisableMouseCapture);
        execute!(stdout, terminal::LeaveAlternateScreen)?;
        terminal::disable_raw_mode()?;
        Ok(())
    }

    pub async fn run(&mut self) -> Result<i32, Error> {

        // Exit the alternate screen and disable raw mode before panicking
        let hook = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            // We can't handle errors properly inside this closure.  And it's
            // probably not a good idea to `unwrap()` inside a panic handler.
            // So we just ignore the `Result`s.
            let _ = execute!(std::io::stdout(), DisableMouseCapture);
            let _ = execute!(std::io::stdout(), terminal::LeaveAlternateScreen);
            let _ = terminal::disable_raw_mode();
            hook(info);
        }));

        self.claim_term().await?;
        self.event_loop().await;
        self.restore_term()?;

        Ok(0)//self.editor.exit_code)
    }
}


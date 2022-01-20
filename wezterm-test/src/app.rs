use log::{error, warn};
use futures::StreamExt;

use std::{
    io::{stdin, stdout, Write},
    sync::Arc,
    time::{Duration, Instant},
};


use anyhow::Error;

use crossterm::{
    event::{DisableMouseCapture, EnableMouseCapture, Event, KeyEvent, EventStream, MouseEventKind, MouseEvent,
        KeyModifiers, KeyCode,
    },
    execute, terminal,
    tty::IsTty,
};

//use smol::prelude::*;

#[cfg(not(windows))]
use {
    signal_hook::{consts::signal, low_level},
    //signal_hook_tokio::Signals,
    //signal_hook_async_std::Signals,
};
#[cfg(windows)]
type Signals = futures_util::stream::Empty<()>;

pub struct Application {
    //signals: Signals
}

#[derive(Debug)]
enum Msg {
    Continue,
    Stop,
    Signal(i32),
    Input(Event),
    Quit
}

impl Application {
    pub fn new() -> Result<Self, Error> {
        Ok(Self {
            //signals
        })
    }

    pub async fn event_loop(&mut self) {
        use signal_hook::iterator::{Signals, SignalsInfo};
        //use signal_hook::iterator::exfiltrator::origin::WithOrigin;
        use signal_hook::low_level;

        let mut reader = EventStream::new().map(|e| {
            log::info!("key: {:?}", e);
            match e {
                Ok(event) => Msg::Input(event),
                Err(e) => Msg::Quit
            }
        });
   
        let (tx, rx) = smol::channel::unbounded();

        // handle signals in a separate thread
        let handle = std::thread::spawn(move || {
            let mut sigs = Signals::new(&[signal::SIGTSTP, signal::SIGCONT, signal::SIGINT]).unwrap();
            for sig in sigs.forever() {
                log::info!("signal: {:?}", sig);
                if sig == signal::SIGINT {
                    tx.try_send(Msg::Quit).unwrap();
                    break;
                } else {
                    tx.try_send(Msg::Signal(sig)).unwrap();
                }
            }
        });

        let mut s = smol::stream::empty::<Msg>();
        let mut stream = smol::stream::race(reader, rx);
        loop {
            let result = stream.next().await;
            match result {
                // handle Ctrl-C to exit for now
                Some(Msg::Input(Event::Key(KeyEvent {
                    code: KeyCode::Char('c'),
                    modifiers: KeyModifiers::CONTROL,
                }))) => break,

                // handle suspend
                Some(Msg::Input(Event::Key(KeyEvent {
                    code: KeyCode::Char('z'),
                    modifiers: KeyModifiers::CONTROL,
                }))) => self.handle_signals(signal::SIGTSTP).await,

                Some(Msg::Input(event)) => self.handle_terminal_events(Some(Ok(event))),
                Some(Msg::Signal(sig)) => self.handle_signals(sig).await,
                Some(Msg::Quit) => break,
                Some(m) => log::info!("{:?}", m),
                None => break
            }
        }
        //handle.join().unwrap();
        log::info!("end loop");
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
        //if self.config.editor.mouse {
            execute!(stdout, EnableMouseCapture)?;
        //}
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
        std::process::exit(0);

        Ok(0)//self.editor.exit_code)
    }
}


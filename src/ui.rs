use std::io::Write;
use std::sync::Mutex;

use tokio::sync::mpsc;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Overlay {
    None,
    Help,
    Log,
    Events,
}

pub enum Key {
    Quit,
    Repaint,
}

// Terminal settings before raw mode, restored on exit.
static SAVED_TERMIOS: Mutex<Option<libc::termios>> = Mutex::new(None);

fn stdin_is_tty() -> bool {
    unsafe { libc::isatty(libc::STDIN_FILENO) == 1 }
}

// Same flags as Node's setRawMode(true): no echo/line buffering/signals, but
// output processing stays on so "\n" still returns the carriage.
fn set_raw_mode() {
    unsafe {
        let mut t: libc::termios = std::mem::zeroed();
        if libc::tcgetattr(libc::STDIN_FILENO, &mut t) != 0 {
            return;
        }
        *SAVED_TERMIOS.lock().unwrap() = Some(t);
        t.c_iflag &= !(libc::BRKINT | libc::ICRNL | libc::INPCK | libc::ISTRIP | libc::IXON);
        t.c_oflag |= libc::ONLCR;
        t.c_cflag |= libc::CS8;
        t.c_lflag &= !(libc::ECHO | libc::ICANON | libc::IEXTEN | libc::ISIG);
        t.c_cc[libc::VMIN] = 1;
        t.c_cc[libc::VTIME] = 0;
        libc::tcsetattr(libc::STDIN_FILENO, libc::TCSADRAIN, &t);
    }
}

pub fn restore_terminal() {
    if let Some(t) = SAVED_TERMIOS.lock().unwrap().take() {
        unsafe {
            libc::tcsetattr(libc::STDIN_FILENO, libc::TCSADRAIN, &t);
        }
    }
}

// (rows, cols) of the terminal, if stdout is one.
pub fn terminal_size() -> Option<(usize, usize)> {
    unsafe {
        let mut ws: libc::winsize = std::mem::zeroed();
        (libc::ioctl(libc::STDOUT_FILENO, libc::TIOCGWINSZ, &mut ws) == 0 && ws.ws_row > 0)
            .then_some((ws.ws_row as usize, ws.ws_col as usize))
    }
}

pub fn write_out(s: &str) {
    let mut out = std::io::stdout().lock();
    let _ = out.write_all(s.as_bytes());
    let _ = out.flush();
}

// Shared UI state, mutated by keypresses, read by the renderer.
pub struct Ui {
    pub overlay: Overlay,
}

impl Ui {
    pub fn new(verbose: bool) -> Ui {
        Ui { overlay: if verbose { Overlay::Help } else { Overlay::None } }
    }

    fn toggle(&mut self, o: Overlay) {
        self.overlay = if self.overlay == o { Overlay::None } else { o };
    }

    // Keys are matched by control code, by the first byte of each read.
    pub fn handle_key(&mut self, byte: u8) -> Option<Key> {
        match byte {
            3 => return Some(Key::Quit),        // ctrl+c
            15 => self.toggle(Overlay::Log),    // ctrl+o
            5 => self.toggle(Overlay::Events),  // ctrl+e
            8 => self.toggle(Overlay::Help),    // ctrl+h
            27 => self.overlay = Overlay::None, // esc
            _ => return None,
        }
        Some(Key::Repaint)
    }
}

// Puts stdin in raw mode and forwards keypresses (first byte of each read).
// Without a TTY the channel just never yields.
pub fn setup_input() -> mpsc::UnboundedReceiver<u8> {
    let (tx, rx) = mpsc::unbounded_channel();
    if !stdin_is_tty() {
        std::mem::forget(tx);
        return rx;
    }
    set_raw_mode();
    std::thread::spawn(move || {
        let mut buf = [0u8; 64];
        loop {
            let n = unsafe { libc::read(libc::STDIN_FILENO, buf.as_mut_ptr().cast(), buf.len()) };
            if n <= 0 || tx.send(buf[0]).is_err() {
                return;
            }
        }
    });
    rx
}

// Restore the terminal and exit: used by every quit path of the viewer.
pub fn quit(code: i32, tail: &str) -> ! {
    restore_terminal();
    write_out(tail);
    std::process::exit(code)
}

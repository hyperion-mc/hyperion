//! `ExecReload`: ask a running game server to reload its rules, and report what it said.
//!
//! # Why this is its own binary rather than a flag on the server
//!
//! `systemctl reload` runs a *new process*, and its exit status is the reload's exit
//! status. So the thing systemd runs has to be something that can always start: this crate
//! has no dependencies at all, which means a rules dylib built against the wrong engine
//! cannot turn "the reload was refused, and here is the reason" into "the client failed to
//! start". The refusal has to survive to be read.
//!
//! # What it says, and what it exits with
//!
//! One verb out, one line back, straight from `hyperion_hot_reload::service`:
//!
//! ```text
//! accepted <module> <revision>   -> printed, exit 0
//! refused <reason>               -> printed, exit 1
//! ```
//!
//! The reply is printed either way, because the reason is the only part of a refused
//! deploy anybody can act on and `systemctl reload` shows it to whoever asked.

// This program's entire output contract is one line on stdout -- the server's own answer,
// which a person or a deploy script reads. The workspace denies `print_stdout` because a
// library or a game server writing to stdout is a bug; for a one-shot CLI it is the API.
#![allow(clippy::print_stdout)]

use std::{
    io::{BufRead, BufReader, Write},
    os::unix::net::UnixStream,
    path::Path,
    process::ExitCode,
    time::Duration,
};

/// The only thing the server answers.
const VERB: &[u8] = b"reload";

/// How long to wait for the answer.
///
/// The server polls the socket between two ticks, so the floor is one tick -- 50ms -- plus
/// however long the `dlopen`, the schema diff and any component migration take. Thirty
/// seconds is far above every measurement in `docs/hot-reload.md` and far below systemd's
/// default `TimeoutSec`, which is what would otherwise decide this: a client that hangs
/// forever turns a wedged server into a wedged `ix apply` with no message anywhere.
const REPLY_TIMEOUT: Duration = Duration::from_secs(30);

/// Whether a reply means the reload happened.
///
/// The first word and not a substring search: "refused could not open module" contains the
/// word "accepted" in no reading, but a reason quoted back from a build could contain
/// anything, and a status derived from a `contains` would be a status that can be talked
/// into lying.
fn accepted(reply: &str) -> bool {
    reply.split_whitespace().next() == Some("accepted")
}

/// Sends the verb and returns the single line the server answered with.
fn ask(socket: &Path) -> std::io::Result<String> {
    let mut stream = UnixStream::connect(socket)?;
    stream.set_read_timeout(Some(REPLY_TIMEOUT))?;
    stream.write_all(VERB)?;
    stream.flush()?;

    let mut reply = String::new();
    BufReader::new(stream).read_line(&mut reply)?;
    Ok(reply.trim_end().to_owned())
}

fn main() -> ExitCode {
    let mut args = std::env::args_os().skip(1);
    let Some(socket) = args.next() else {
        eprintln!("usage: hyperion-reload-client <socket>");
        return ExitCode::FAILURE;
    };

    match ask(Path::new(&socket)) {
        Ok(reply) if accepted(&reply) => {
            println!("{reply}");
            ExitCode::SUCCESS
        }
        // A refusal is the server working: it looked at the build and said no. Printed the
        // same way, because the words are the gate's own and they name the component.
        Ok(reply) => {
            println!("{reply}");
            ExitCode::FAILURE
        }
        // Nothing answered. `ECONNREFUSED` on an existing path is the interesting one --
        // the socket file outlived the process that bound it -- so the path is named.
        Err(e) => {
            eprintln!(
                "could not reach the game server on {}: {e}",
                socket.display()
            );
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{io::Read, os::unix::net::UnixListener, thread};

    use super::*;

    /// A socket that answers one request with `reply` and then stops.
    fn server_saying(
        name: &str,
        reply: &'static str,
    ) -> (std::path::PathBuf, thread::JoinHandle<Vec<u8>>) {
        let path = std::env::temp_dir().join(format!("hyperion-reload-client-{name}.sock"));
        drop(std::fs::remove_file(&path));
        let listener = UnixListener::bind(&path).expect("bind");
        let handle = thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("accept");
            // One `read`, exactly like `hyperion_hot_reload::service::read_verb`. Reading
            // to EOF instead would test a server this client never talks to.
            let mut buffer = [0u8; 64];
            let read = stream.read(&mut buffer).expect("read");
            writeln!(stream, "{reply}").expect("write");
            buffer[..read].to_vec()
        });
        (path, handle)
    }

    /// The verb reaches the server and the answer comes back whole.
    #[test]
    fn the_server_hears_the_verb_and_its_answer_comes_back() {
        let (path, handle) = server_saying("round-trip", "accepted smash-rules abc1234");
        let reply = ask(&path).expect("ask");
        assert_eq!(reply, "accepted smash-rules abc1234");
        assert_eq!(handle.join().expect("join"), VERB);
        drop(std::fs::remove_file(&path));
    }

    /// A socket file with nothing behind it is what a killed server leaves. It has to read
    /// as a failed reload rather than as a hang or a success.
    #[test]
    fn a_socket_nobody_is_listening_on_is_an_error() {
        let path = std::env::temp_dir().join("hyperion-reload-client-dead.sock");
        drop(std::fs::remove_file(&path));
        drop(UnixListener::bind(&path).expect("bind"));
        assert!(ask(&path).is_err());
        drop(std::fs::remove_file(&path));
    }

    /// The exit status comes from the first word, and only from the first word.
    #[test]
    fn only_an_accepted_first_word_is_a_success() {
        assert!(accepted("accepted smash-rules abc1234"));
        assert!(accepted("accepted smash-rules unknown"));
        assert!(!accepted("refused component smash::Health changed layout"));
        assert!(!accepted(""));
        // A refusal whose reason quotes a build cannot talk its way into a zero exit.
        assert!(!accepted("refused unknown request `accepted`"));
    }
}

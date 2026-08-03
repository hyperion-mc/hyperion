//! Answering a service manager's reload request, between two ticks and never during one.
//!
//! # Why the host owns its tick loop
//!
//! A reload deletes and re-creates every system and observer a module registered, and may
//! rewrite stored component bytes. Doing that while flecs is mid-frame is not a race that
//! shows up as a wrong number; it is a system table being rebuilt underneath an iterator.
//! So the reload has to happen at a point where no frame is in progress, and the only such
//! point is between `world.progress()` calls -- which means the host has to be the thing
//! calling `progress`, rather than handing control to `ecs_app_run` and never getting it
//! back. [`run`] is that loop, and it is the whole reason this module is not simply a
//! flecs system.
//!
//! # The protocol is one verb, and the answer is one line
//!
//! `ExecReload` runs a client that connects to a unix socket in the unit's runtime
//! directory, writes `reload`, and prints whatever comes back. The reply is a single line:
//!
//! ```text
//! accepted <module> <revision>
//! refused <reason>
//! ```
//!
//! The client's exit status is taken from the first word, so a refused reload fails the
//! `systemctl reload` that asked for it instead of being a line in a log nobody reads. The
//! refusal text is the gate's own, which names the component and both layouts.
//!
//! Nothing in the request says *what* to load. The module path and the revision file are
//! fixed at startup, so `ExecReload` is a constant string and a deploy that changes the
//! rules dylib does not change the unit's `[Service]` section -- which is exactly the
//! condition under which systemd reloads rather than restarts.
//!
//! # The revision comes from a file, never from the environment
//!
//! A process's environment is fixed at `exec` time. Reading the build revision from
//! `HYPERION_BUILD_REV` would therefore report the build the process *started* as, forever,
//! and a hot reload's entire point is that the process does not restart. The revision is
//! read from a path on every accepted reload, so it says what was just loaded.
//!
//! A missing or unreadable file is reported as an unknown revision rather than as a failed
//! reload. The revision is a label for humans; refusing to load working code because a
//! label is missing would be the wrong trade.

use std::{
    io::{ErrorKind, Read, Write},
    os::unix::net::{UnixListener, UnixStream},
    path::{Path, PathBuf},
    time::Duration,
};

use flecs_ecs::core::World;

use crate::host::{Applied, HotReloader, LoadError};

/// How long a connected client is given to send its verb.
///
/// This is a stall in the tick loop, so it is bounded and short. A reload happens on
/// deploy and at no other time, and one hitch of this length on a deploy is invisible
/// beside the alternative, which is every player being disconnected by a restart. The
/// timeout exists so that a client which connects and then says nothing -- a health probe,
/// a stray `nc`, a killed `systemctl` -- cannot stop the world.
const VERB_TIMEOUT: Duration = Duration::from_millis(100);

/// The only thing a client may ask for.
const VERB: &str = "reload";

/// A reload that was applied to the running world.
#[derive(Debug, Clone)]
pub struct Reloaded {
    /// The module name the dylib declared.
    pub module: String,
    /// What the revision file said just after the load, or `None` when nothing said.
    pub revision: Option<String>,
    /// How many stored component instances a migration rewrote.
    pub migrated_instances: usize,
}

/// Where a host listens, what it loads, and what it calls the result.
pub struct ReloadService {
    listener: UnixListener,
    module: PathBuf,
    revision: PathBuf,
    reloader: HotReloader,
}

impl ReloadService {
    /// Binds the request socket and prepares to load `module`.
    ///
    /// A socket file left behind by a previous process is removed first: `bind` fails with
    /// `AddrInUse` on an existing path whether or not anything is listening, and a server
    /// that refuses to start because its predecessor was killed is a worse failure than a
    /// stale socket.
    ///
    /// # Errors
    /// Returns whatever binding the socket failed with.
    pub fn bind(socket: &Path, module: PathBuf, revision: PathBuf) -> std::io::Result<Self> {
        match std::fs::remove_file(socket) {
            Ok(()) => {}
            Err(e) if e.kind() == ErrorKind::NotFound => {}
            Err(e) => return Err(e),
        }
        let listener = UnixListener::bind(socket)?;
        // The tick loop polls; it must never wait for a client that may never arrive.
        listener.set_nonblocking(true)?;
        Ok(Self {
            listener,
            module,
            revision,
            reloader: HotReloader::new(),
        })
    }

    /// Loads the module for the first time, before any tick has run.
    ///
    /// # Errors
    /// Returns whatever the loader refused with.
    pub fn load_initial(&mut self, world: &World) -> Result<Applied, LoadError> {
        self.reloader.load(world, &self.module)
    }

    /// The module and component tree currently live.
    #[must_use]
    pub fn manifest(&self) -> crate::manifest::Manifest {
        self.reloader.manifest()
    }

    /// What the revision file says right now.
    fn revision(&self) -> Option<String> {
        let text = std::fs::read_to_string(&self.revision).ok()?;
        let trimmed = text.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_owned())
        }
    }

    /// Answers every request waiting on the socket, applying each to `world`.
    ///
    /// Returns one entry per reload that was applied. A refused reload leaves the world
    /// exactly as it was and produces no entry, because the caller's job -- telling players
    /// what they are now running -- has nothing to say about a build that was rejected.
    pub fn poll(&mut self, world: &World) -> Vec<Reloaded> {
        let mut applied = Vec::new();
        loop {
            let stream = match self.listener.accept() {
                Ok((stream, _)) => stream,
                // Nothing waiting: the normal exit from this loop, once per tick.
                Err(e) if e.kind() == ErrorKind::WouldBlock => return applied,
                Err(e) => {
                    tracing::warn!("reload socket accept failed: {e}");
                    return applied;
                }
            };
            if let Some(reloaded) = self.serve(world, stream) {
                applied.push(reloaded);
            }
        }
    }

    /// One request, start to finish.
    fn serve(&mut self, world: &World, mut stream: UnixStream) -> Option<Reloaded> {
        let reply = match read_verb(&mut stream) {
            Ok(verb) if verb == VERB => match self.reloader.load(world, &self.module) {
                Ok(applied) => {
                    let revision = self.revision();
                    let reloaded = Reloaded {
                        module: applied.module.clone(),
                        revision: revision.clone(),
                        migrated_instances: applied.migrated_instances,
                    };
                    let label = revision.as_deref().unwrap_or("unknown");
                    tracing::info!(
                        module = %applied.module,
                        revision = label,
                        migrated_instances = applied.migrated_instances,
                        "hot reload accepted"
                    );
                    let line = format!("accepted {} {label}", applied.module);
                    respond(&mut stream, &line);
                    return Some(reloaded);
                }
                // Deliberately `error`, not `warn`: a refused reload means the deploy did
                // not take, and the operator has to see it in a query filtered by severity.
                Err(e) => {
                    tracing::error!("hot reload refused: {e}");
                    format!("refused {e}")
                }
            },
            Ok(verb) => format!("refused unknown request `{verb}`"),
            Err(e) => format!("refused could not read request: {e}"),
        };
        respond(&mut stream, &reply);
        None
    }
}

/// Writes one line back, or says why it could not.
///
/// A client that hung up before reading is not a reload failure -- the reload already
/// happened -- so this reports and returns rather than propagating.
fn respond(stream: &mut UnixStream, line: &str) {
    if let Err(e) = writeln!(stream, "{line}").and_then(|()| stream.flush()) {
        tracing::warn!("could not answer a reload request: {e}");
    }
}

/// Reads the client's verb, giving up after [`VERB_TIMEOUT`].
fn read_verb(stream: &mut UnixStream) -> std::io::Result<String> {
    // The listener is non-blocking and an accepted stream inherits that on some platforms,
    // which would turn every read into `WouldBlock`. Set both explicitly.
    stream.set_nonblocking(false)?;
    stream.set_read_timeout(Some(VERB_TIMEOUT))?;
    let mut buffer = [0u8; 64];
    let read = stream.read(&mut buffer)?;
    let text = String::from_utf8_lossy(&buffer[..read]);
    Ok(text.trim().to_owned())
}

/// Ticks the world until it stops, answering reload requests between frames.
///
/// `on_reload` is the seam an event uses to tell its players what just happened; it is
/// called once per applied reload, with a world that is between frames and safe to write
/// to. It is deliberately a callback on the loop rather than a flecs observer, because a
/// reload is not a world event -- it is the thing that just replaced every observer.
pub fn run<F>(world: &World, service: &mut ReloadService, mut on_reload: F)
where
    F: FnMut(&World, &Reloaded),
{
    while world.progress() {
        for reloaded in service.poll(world) {
            on_reload(world, &reloaded);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::io::{BufRead, BufReader};

    use super::*;

    /// A socket, module and revision path under one temporary directory.
    struct Fixture {
        dir: PathBuf,
    }

    impl Fixture {
        fn new(name: &str) -> Self {
            let dir = std::env::temp_dir().join(format!("hyperion-reload-test-{name}"));
            drop(std::fs::remove_dir_all(&dir));
            std::fs::create_dir_all(&dir).expect("temp dir");
            Self { dir }
        }

        fn socket(&self) -> PathBuf {
            self.dir.join("reload.sock")
        }

        fn revision_path(&self) -> PathBuf {
            self.dir.join("build-rev")
        }

        /// A module path that exists nowhere, so `dlopen` refuses and the world is
        /// untouched -- which is what every protocol test wants.
        fn absent_module(&self) -> PathBuf {
            self.dir.join("no-such-module.so")
        }

        fn service(&self) -> ReloadService {
            ReloadService::bind(&self.socket(), self.absent_module(), self.revision_path())
                .expect("bind")
        }

        /// Sends `verb` (or nothing at all), lets the service answer, and returns the
        /// single line it wrote.
        fn ask(&self, service: &mut ReloadService, world: &World, verb: Option<&[u8]>) -> String {
            let mut client = UnixStream::connect(self.socket()).expect("connect");
            if let Some(verb) = verb {
                client.write_all(verb).expect("write");
                client.flush().expect("flush");
            }
            service.poll(world);
            let mut line = String::new();
            BufReader::new(client)
                .read_line(&mut line)
                .expect("read reply");
            line.trim_end().to_owned()
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            drop(std::fs::remove_dir_all(&self.dir));
        }
    }

    /// A killed predecessor leaves its socket file behind, and `bind` fails on an existing
    /// path whether or not anything is listening. Refusing to start for that reason would
    /// turn one crash into a server that never comes back.
    #[test]
    fn a_socket_left_by_a_dead_predecessor_does_not_stop_a_bind() {
        let fixture = Fixture::new("stale-socket");
        std::fs::write(fixture.socket(), b"not really a socket").expect("write stale file");
        drop(fixture.service());
    }

    /// The world must be untouched by anything that is not the one verb, because a stray
    /// connection is not a deploy.
    #[test]
    fn an_unrecognised_request_is_refused_without_loading_anything() {
        let fixture = Fixture::new("unknown-verb");
        let world = World::new();
        let mut service = fixture.service();
        let reply = fixture.ask(&mut service, &world, Some(b"explode"));
        assert_eq!(reply, "refused unknown request `explode`");
    }

    /// A client that connects and then says nothing must not stop the world. Without the
    /// read timeout this test hangs forever rather than failing.
    #[test]
    fn a_client_that_says_nothing_is_refused_rather_than_stopping_the_world() {
        let fixture = Fixture::new("silent-client");
        let world = World::new();
        let mut service = fixture.service();
        let reply = fixture.ask(&mut service, &world, None);
        assert!(
            reply.starts_with("refused could not read request"),
            "expected a read refusal, got `{reply}`"
        );
    }

    /// The refusal carries the loader's own words. An operator reading `systemctl reload`
    /// output needs the reason, not a generic failure.
    #[test]
    fn a_refused_load_answers_with_the_loaders_reason() {
        let fixture = Fixture::new("bad-module");
        let world = World::new();
        let mut service = fixture.service();
        let reply = fixture.ask(&mut service, &world, Some(b"reload"));
        assert!(
            reply.starts_with("refused could not open module"),
            "expected the dlopen refusal, got `{reply}`"
        );
    }

    /// Nothing is waiting on the socket for all but a handful of ticks in a process's life,
    /// so the empty poll is the only path that has to be free of surprises.
    #[test]
    fn polling_an_idle_socket_reports_nothing_and_returns() {
        let fixture = Fixture::new("idle");
        let world = World::new();
        let mut service = fixture.service();
        assert!(service.poll(&world).is_empty());
        assert!(service.poll(&world).is_empty());
    }

    /// A label for humans, so a missing file is "unknown" and never a failed reload.
    #[test]
    fn a_missing_revision_file_reads_as_unknown() {
        let fixture = Fixture::new("revision-missing");
        assert_eq!(fixture.service().revision(), None);
    }

    /// `environment.etc` writes the file with a trailing newline, and a title reading
    /// "Reloading to build abc123\n" is a rendering bug nobody would look for here.
    #[test]
    fn a_revision_is_trimmed_and_an_empty_one_is_unknown() {
        let fixture = Fixture::new("revision-trim");
        std::fs::write(fixture.revision_path(), b"  abc1234\n").expect("write revision");
        assert_eq!(fixture.service().revision().as_deref(), Some("abc1234"));

        std::fs::write(fixture.revision_path(), b"   \n").expect("write blank revision");
        assert_eq!(fixture.service().revision(), None);
    }
}

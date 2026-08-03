//! Wiring: the console entity, the tap, the web server, and one system a tick.

use std::sync::Arc;

use flecs_ecs::{macros::system, prelude::*};
use hyperion::{
    console::{ChatObserver, ChatObservers},
    egress::{ping::Ping, tab_list::Tps},
    hyperion_minecraft_proto::{
        generated::packet_id::play::clientbound::PacketId,
        packets::play::clientbound::SystemChat,
        text::{Component, NamedColor},
    },
    net::{Compose, ConnectionId, protocol::Clientbound},
    simulation::{Name, PacketState, chat::strip_formatting, event},
    storage::Events,
};
use hyperion_permission::Group;

use crate::{
    Config, Console,
    feed::Source,
    state::{PlayerRow, Request, Snapshot},
    virtual_connection::ConsoleConnection,
};

/// How the console appears in chat, and to any command that names its caller.
pub const CONSOLE_NAME: &str = "Console";

/// How often the roster and the tick rate are rebuilt, in ticks.
///
/// A second. The page is a person watching, and a person cannot read twenty
/// updates a second; rebuilding the roster every tick would cost a query per
/// tick to redraw something that has not changed.
const SNAPSHOT_EVERY: i64 = 20;

/// The console, as an ECS singleton.
///
/// A newtype rather than storing the `Arc` bare, because a bare `Arc<Console>`
/// is not a component and flecs keys on the type.
#[derive(Component)]
pub struct ConsoleHandle(pub Arc<Console>);

/// The entity commands run as.
#[derive(Component)]
pub struct ConsoleCaller(pub Entity);

/// Registration: the components this crate owns, and nothing else.
#[derive(Component)]
pub struct ConsoleComponentsModule;

impl Module for ConsoleComponentsModule {
    fn module(world: &World) {
        world
            .component::<ConsoleHandle>()
            .add_trait::<flecs::Singleton>();
        world
            .component::<ConsoleCaller>()
            .add_trait::<flecs::Singleton>();
    }
}

/// Behaviour: the web server, the chat tap, and the tick that carries out what
/// the operator asked for.
///
/// Imported explicitly by a deployment rather than by `HyperionCore`, because a
/// console is a thing an operator turns on. Import it and pass a [`Config`];
/// leave it out and nothing here costs anything.
#[derive(Component)]
pub struct ConsoleModule;

impl Module for ConsoleModule {
    fn module(world: &World) {
        world.import::<ConsoleComponentsModule>();
        // Every component this module's systems read, registered by the module
        // that owns it. `HyperionCore` is not optional and not implied by the
        // event having imported it already: flecs dedupes the import, and the
        // failure mode of leaving it out is a use-before-register that a
        // release build compiles the assert out of and a dev build aborts on
        // (ENG-11000). `Compose`, `Events`, `Name`, `ConnectionId`,
        // `ChatObservers`, `Ping` and `Tps` all come from here.
        world.import::<hyperion::HyperionCore>();
        // The registry commands are looked up in, and the `Group` a command's
        // permission check reads off the caller.
        world.import::<hyperion_command::CommandModule>();
        world.import::<hyperion_permission::PermissionModule>();
    }
}

/// Turn the console on.
///
/// Separate from [`ConsoleModule`] because a flecs module takes no arguments
/// and the address and the token are not compile-time facts. Import the module,
/// then call this.
///
/// # Errors
/// Fails if the web server cannot bind. Deliberately fatal to the caller rather
/// than logged: an operator who asked for a console and did not get one has to
/// find out at startup.
pub fn install(world: &World, config: &Config) -> std::io::Result<()> {
    let console = Arc::new(Console::new(config.token.clone()));

    // Leaked on purpose, and exactly once. The tap and the virtual connection
    // are called from the packet path and from `IoBuf`, neither of which can
    // hold a world reference, and the console lives as long as the process
    // anyway. A leak with a bounded, known size beats threading a lifetime
    // through the engine's hot path.
    let shared: &'static Console = Box::leak(Box::new(Arc::clone(&console)));

    let caller = spawn_caller(world);
    world.set(ConsoleCaller(caller));

    world.get::<&mut ChatObservers>(|observers| {
        observers.watch(Arc::new(ChatToFeed {
            console: Arc::clone(&console),
        }));
    });

    world.get::<&mut Compose>(|compose| {
        // The server's own threshold, handed over as it is: turning it into
        // what a decoder wants is `ConsoleConnection`'s job, next to the tests
        // that pin the rule.
        let threshold = compose.global().shared.compression_threshold;
        compose
            .io_buf_mut()
            .attach_virtual_connection(Arc::new(ConsoleConnection::new(&shared.feed, threshold)));
    });

    crate::spawn(Arc::clone(&console), config.address)?;
    console.feed.push(
        Source::System,
        format!("\u{a7}8console listening on http://{}/", config.address),
    );

    world.set(ConsoleHandle(console));
    install_systems(world);
    Ok(())
}

/// The entity commands run as.
///
/// It is not a player and must never be mistaken for one: no
/// [`PacketState::Play`], which is what events key "this is somebody who
/// joined" on, so no game promotes it, counts it in a lobby, or writes a
/// sidebar to it. What it does carry is exactly what a command reads --- a
/// name, a group, and a connection to answer.
fn spawn_caller(world: &World) -> Entity {
    let caller = world
        .entity_named(CONSOLE_NAME)
        .set(Name::from(std::sync::Arc::<str>::from(CONSOLE_NAME)))
        .set(Group::Admin)
        .set(ConnectionId::new(
            crate::virtual_connection::CONSOLE_STREAM,
            hyperion::net::ProxyId::new(crate::virtual_connection::CONSOLE_PROXY),
        ));

    debug_assert!(
        !caller.has_enum(PacketState::Play),
        "the console caller must not look like a joined player"
    );

    caller.id()
}

/// Player chat, on its way to the page.
struct ChatToFeed {
    console: Arc<Console>,
}

impl ChatObserver for ChatToFeed {
    fn player_said(&self, _speaker: Entity, name: &str, message: &str) {
        // Vanilla's shape, so a line reads on the page the way it reads in the
        // chat box. Not the line the game broadcast, which this cannot see:
        // see `hyperion::console` for why the tap is before the queue and what
        // that trades away.
        //
        // The message keeps its section signs and the page renders them, which
        // is a decision with a hazard attached: a player typing `\u{a7}c` paints
        // their own text here. That is the same hazard the game has and the
        // same answer -- the engine's `strip_formatting` -- applied at the one
        // place that knows this text came off a wire.
        self.console.feed.push(
            Source::Chat,
            format!("<{name}> {}", strip_formatting(message)),
        );
    }
}

fn install_systems(world: &World) {
    system!(
        "console_requests",
        world,
        &ConsoleHandle,
        &ConsoleCaller,
        &Compose,
        &Events
    )
    .each_iter(|it, _index, (handle, caller, compose, events)| {
        let world = it.world();
        for request in handle.0.take_requests() {
            match request {
                Request::Say(message) => {
                    // Built as a component with the colour as a field,
                    // rather than as a string with `\u{a7}d` inside it.
                    // Both render the same on a client, and `nix/text.nix`
                    // exists because only one of them survives contact
                    // with anything that reads the text: the legacy codes
                    // are invisible to `Component::plain`, say nothing
                    // outside the sixteen named colours, and are a
                    // formatter kept alive for 1.8 servers. That gate does
                    // not reach this crate today (ENG-10796 tracks
                    // widening it), which is a reason to hold the line
                    // here rather than to relax it.
                    //
                    // The operator's own text is a child with no style of
                    // its own, so it cannot be styled by the prefix, and
                    // the feed gets the same component rendered back down
                    // to the codes the page draws.
                    let component = Component::text(format!("[{CONSOLE_NAME}]"))
                        .color(NamedColor::LightPurple)
                        .append(Component::text(format!(" {message}")).color(NamedColor::White));
                    let line = crate::legacy::render(&component);
                    let packet = SystemChat {
                        content: component.to_tag(),
                        overlay: false,
                    };
                    match compose
                        .broadcast(Clientbound::new(PacketId::SystemChat.to_raw(), &packet))
                        .send()
                    {
                        Ok(()) => handle.0.feed.push(Source::Console, line),
                        Err(error) => {
                            // The operator pressed send and nothing
                            // happened, so the page has to say so rather
                            // than the journal alone.
                            handle.0.feed.push(
                                Source::System,
                                format!("\u{a7}cthe message was not sent: {error}"),
                            );
                        }
                    }
                }
                Request::Command(raw) => {
                    let raw = raw.strip_prefix('/').unwrap_or(&raw).to_owned();
                    handle
                        .0
                        .feed
                        .push(Source::Console, format!("\u{a7}8> /{raw}"));
                    events.push(
                        event::Command {
                            raw: raw.into(),
                            by: caller.0,
                        },
                        &world,
                    );
                }
            }
        }
    });

    system!("console_snapshot", world, &ConsoleHandle, &Compose, &Tps).each_iter(
        |it, _index, (handle, compose, tps)| {
            if compose.global().tick % SNAPSHOT_EVERY != 0 {
                return;
            }

            let mut players = Vec::new();
            it.world()
                .query::<(&Name, &Ping)>()
                .build()
                .each(|(name, ping)| {
                    players.push(PlayerRow {
                        name: name.to_string(),
                        // `latency` reports the engine's own "no reading"
                        // sentinel as a negative, which the page must not draw
                        // as a fast connection.
                        latency_ms: Some(ping.latency()).filter(|latency| *latency >= 0),
                    });
                });
            players.sort_by(|left, right| left.name.cmp(&right.name));

            handle.0.publish(Snapshot {
                players,
                tps: tps.rate,
                target_tps: hyperion::TICKS_PER_SECOND,
                build_rev: std::env::var("HYPERION_BUILD_REV").ok(),
                build_dirty: std::env::var("HYPERION_BUILD_DIRTY").as_deref() == Ok("1"),
            });
        },
    );
}

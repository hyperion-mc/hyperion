//! Hyperion

#![feature(type_alias_impl_trait)]
#![feature(io_error_more)]
#![feature(trusted_len)]
#![feature(allocator_api)]
#![feature(read_buf)]
#![feature(core_io_borrowed_buf)]
#![feature(maybe_uninit_slice)]
#![feature(duration_millis_float)]
#![feature(sync_unsafe_cell)]
#![feature(iter_array_chunks)]
#![feature(assert_matches)]
#![feature(try_trait_v2)]
#![feature(let_chains)]
#![feature(ptr_metadata)]
#![feature(stmt_expr_attributes)]
#![feature(array_try_map)]
#![feature(split_array)]
#![feature(never_type)]
#![feature(duration_constructors)]
#![feature(array_chunks)]
#![feature(portable_simd)]
#![feature(trivial_bounds)]
#![feature(pointer_is_aligned_to)]
#![feature(thread_local)]

pub const NUM_THREADS: usize = 1;
pub const CHUNK_HEIGHT_SPAN: u32 = 384; // 512; // usually 384

use std::{
    alloc::Allocator,
    cell::RefCell,
    fmt::Debug,
    io::Write,
    net::SocketAddr,
    sync::{Arc, atomic::AtomicBool},
};

use anyhow::Context;
use bevy::prelude::*;
use derive_more::{Constructor, Deref, DerefMut};
// use egress::EgressPlugin;
pub use glam;
use glam::{I16Vec2, IVec2};
// use ingress::IngressModule;
#[cfg(unix)]
use libc::{RLIMIT_NOFILE, getrlimit, setrlimit};
use libdeflater::CompressionLvl;
// use simulation::{Comms, SimModule, StreamLookup, blocks::Blocks};
// use storage::{Events, LocalDb, SkinHandler, ThreadLocal};
use tracing::{info, info_span, warn};
use util::mojang::MojangClient;
pub use uuid;
pub use valence_protocol as protocol;
// todo: slowly move more and more things to arbitrary module
// and then eventually do not re-export valence_protocol
pub use valence_protocol;
use valence_protocol::{CompressionThreshold, Encode, Packet};
pub use valence_protocol::{
    ItemKind, ItemStack, Particle,
    block::{BlockKind, BlockState},
};
pub use valence_server as server;

mod common;
pub use common::*;
// use hyperion_crafting::CraftingRegistry;
pub use valence_ident;

use crate::{
    // ingress::PendingRemove,
    net::{
        Compose, ConnectionId, IoBuf, MAX_PACKET_SIZE, PacketDecoder,
        proxy::{ReceiveState, init_proxy_comms},
    },
    // runtime::Tasks,
    // simulation::{
    //     EgressComm, EntitySize, IgnMap, PacketState, Pitch, Player, Yaw, packet::HandlerRegistry,
    // },
    // util::mojang::ApiProvider,
};

// pub mod egress;
// pub mod ingress;
pub mod net;
pub mod simulation;
// pub mod spatial;
pub mod storage;

/// Relationship for previous values
#[derive(Component)]
pub struct Prev;

pub trait PacketBundle {
    fn encode_including_ids(self, w: impl Write) -> anyhow::Result<()>;
}

impl<T: Packet + Encode> PacketBundle for &T {
    fn encode_including_ids(self, w: impl Write) -> anyhow::Result<()> {
        self.encode_with_id(w)
    }
}

/// on macOS, the soft limit for the number of open file descriptors is often 256. This is far too low
/// to test 10k players with.
/// This attempts to the specified `recommended_min` value.
#[tracing::instrument(skip_all)]
#[cfg(unix)]
pub fn adjust_file_descriptor_limits(recommended_min: u64) -> std::io::Result<()> {
    use tracing::{error, warn};

    let mut limits = libc::rlimit {
        rlim_cur: 0, // Initialize soft limit to 0
        rlim_max: 0, // Initialize hard limit to 0
    };

    if unsafe { getrlimit(RLIMIT_NOFILE, &mut limits) } == 0 {
        // Create a stack-allocated buffer...

        info!("current soft limit: {}", limits.rlim_cur);
        info!("current hard limit: {}", limits.rlim_max);
    } else {
        error!("Failed to get the current file handle limits");
        return Err(std::io::Error::last_os_error());
    }

    if limits.rlim_max < recommended_min {
        warn!(
            "Could only set file handle limit to {}. Recommended minimum is {}",
            limits.rlim_cur, recommended_min
        );
    }

    limits.rlim_cur = limits.rlim_max;

    info!("setting soft limit to: {}", limits.rlim_cur);

    if unsafe { setrlimit(RLIMIT_NOFILE, &limits) } != 0 {
        error!("Failed to set the file handle limits");
        return Err(std::io::Error::last_os_error());
    }

    Ok(())
}

#[derive(Resource, Debug, Clone, PartialEq, Eq, Hash)]
pub struct GameServerEndpoint(SocketAddr);

impl From<SocketAddr> for GameServerEndpoint {
    fn from(value: SocketAddr) -> Self {
        const DEFAULT_MINECRAFT_PORT: u16 = 25565;
        let port = value.port();

        if port == DEFAULT_MINECRAFT_PORT {
            warn!(
                "You are setting the port to the default Minecraft port \
                 ({DEFAULT_MINECRAFT_PORT}). You are likely using the wrong port as the proxy \
                 port is the port that players connect to. Therefore, if you want them to join on \
                 {DEFAULT_MINECRAFT_PORT}, you need to set the PROXY port to \
                 {DEFAULT_MINECRAFT_PORT} instead."
            );
        }

        Self(value)
    }
}

/// The central [`HyperionCore`] struct which owns and manages the entire server.
pub struct HyperionCore;

#[derive(Resource, Constructor)]
struct Shutdown {
    value: Arc<AtomicBool>,
}

impl Plugin for HyperionCore {
    /// Initialize the server.
    fn build(&self, app: &mut App) {
        // Minecraft is 20 TPS
        app.insert_resource(Time::<Fixed>::from_hz(20.0));

        // 10k players * 2 file handles / player  = 20,000. We can probably get away with 16,384 file handles
        #[cfg(unix)]
        if let Err(e) = adjust_file_descriptor_limits(32_768) {
            warn!("failed to set file limits: {e}");
        }

        rayon::ThreadPoolBuilder::new()
            .num_threads(NUM_THREADS)
            .spawn_handler(|thread| {
                std::thread::Builder::new()
                    .stack_size(1024 * 1024)
                    .spawn(move || {
                        thread.run();
                    })
                    .expect("Failed to spawn thread");
                Ok(())
            })
            .build_global()
            .expect("failed to build thread pool");

        let shared = Arc::new(Shared {
            compression_threshold: CompressionThreshold(256),
            compression_level: CompressionLvl::new(2).expect("failed to create compression level"),
        });

        // let shutdown = Arc::new(AtomicBool::new(false));
        //
        // app.insert_resource(Shutdown::new(shutdown.clone()));
        // app.add_systems(Update, run_tasks);
        //
        // info!("starting hyperion");
        // let config = config::Config::load("run/config.toml")?;
        // world.insert_resource(config);
        //
        // let (task_tx, task_rx) = kanal::bounded(32);
        // let runtime = AsyncRuntime::new(task_tx);
        //
        // #[cfg(unix)]
        // #[allow(clippy::redundant_pub_crate)]
        // runtime.spawn(async move {
        // let mut sigterm =
        // tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()).unwrap();
        // let mut sigquit =
        // tokio::signal::unix::signal(tokio::signal::unix::SignalKind::quit()).unwrap();
        //
        // tokio::select! {
        // _ = tokio::signal::ctrl_c() => {
        // warn!("SIGINT/ctrl-c received, shutting down");
        // shutdown.store(true, std::sync::atomic::Ordering::Relaxed);
        // }
        // _ = sigterm.recv() => {
        // warn!("SIGTERM received, shutting down");
        // shutdown.store(true, std::sync::atomic::Ordering::Relaxed);
        // }
        // _ = sigquit.recv() => {
        // warn!("SIGQUIT received, shutting down");
        // shutdown.store(true, std::sync::atomic::Ordering::Relaxed);
        // }
        // }
        // });
        //
        // let tasks = Tasks { tasks: task_rx };
        // app.insert_resource(tasks);
        //
        // app.insert_resource(HandlerRegistry::default());
        //
        // let db = LocalDb::new()?;
        // let skins = SkinHandler::new(&db)?;
        //
        // app.insert_resource(db);
        // app.insert_resource(skins);
        //
        // app.insert_resource(MojangClient::new(&runtime, ApiProvider::MAT_DOES_DEV));
        //
        // #[rustfmt::skip]
        // app.add_observer(set_server_endpoint);
        //
        // let global = Global::new(shared.clone());
        //
        // app.insert_resource(Compose::new(
        // Compressors::new(shared.compression_level),
        // Scratches::default(),
        // global,
        // IoBuf::default(),
        // ));
        //
        // app.insert_resource(CraftingRegistry::default());
        //
        // app.insert_resource(Comms::default());
        //
        // let events = Events::initialize(app);
        // app.insert_resource(events);
        //
        // app.insert_resource(runtime);
        // app.insert_resource(StreamLookup::default());
        //
        app.add_plugins(TaskPoolPlugin::default());
        // app.add_plugins((SimModule, EgressPlugin, IngressModule));
        //
        // app
        //     .component::<Player>()
        //     .add_trait::<(flecs::With, EntitySize)>();
        //
        // app.insert_resource(IgnMap::default());
        // app.insert_resource(Blocks::empty(app));
    }
}

// fn set_server_endpoint(trigger: Trigger<GameServerEndpoint>, mut commands: Commands<'_, '_>) {
//     let (receive_state, egress_comm) = init_proxy_comms(runtime, address);
//     commands.insert_resource(receive_state);
//
//     commands.insert_resource(egress_comm);
// }

/// A scratch buffer for intermediate operations. This will return an empty [`Vec`] when calling [`Scratch::obtain`].
#[derive(Debug)]
pub struct Scratch<A: Allocator = std::alloc::Global> {
    inner: Box<[u8], A>,
}

impl Default for Scratch<std::alloc::Global> {
    fn default() -> Self {
        std::alloc::Global.into()
    }
}

/// Nice for getting a buffer that can be used for intermediate work
pub trait ScratchBuffer: sealed::Sealed + Debug {
    /// The type of the allocator the [`Vec`] uses.
    type Allocator: Allocator;
    /// Obtains a buffer that can be used for intermediate work. The contents are unspecified.
    fn obtain(&mut self) -> &mut [u8];
}

mod sealed {
    pub trait Sealed {}
}

impl<A: Allocator + Debug> sealed::Sealed for Scratch<A> {}

impl<A: Allocator + Debug> ScratchBuffer for Scratch<A> {
    type Allocator = A;

    fn obtain(&mut self) -> &mut [u8] {
        &mut self.inner
    }
}

impl<A: Allocator> From<A> for Scratch<A> {
    fn from(allocator: A) -> Self {
        // A zeroed slice is allocated to avoid reading from uninitialized memory, which is UB.
        // Allocating zeroed memory is usually very cheap, so there are minimal performance
        // penalties from this.
        let inner = Box::new_zeroed_slice_in(MAX_PACKET_SIZE, allocator);
        // SAFETY: The box was initialized to zero, and u8 can be represented by zero
        let inner = unsafe { inner.assume_init() };
        Self { inner }
    }
}

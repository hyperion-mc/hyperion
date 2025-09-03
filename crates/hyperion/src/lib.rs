//! Hyperion

pub const CHUNK_HEIGHT_SPAN: u32 = 384; // 512; // usually 384

use std::{fmt::Debug, io::Write, net::SocketAddr, path::Path, sync::Arc, time::Duration};

use bevy::prelude::*;
use egress::EgressPlugin;
pub use glam;
#[cfg(unix)]
use libc::{RLIMIT_NOFILE, getrlimit, setrlimit};
use libdeflater::CompressionLvl;
use rustls_pki_types::{CertificateDer, PrivateKeyDer, pem::PemObject};
use storage::{LocalDb, SkinHandler};
use tracing::{info, warn};
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

mod common;
pub use common::*;
use hyperion_crafting::CraftingRegistry;
use hyperion_utils::HyperionUtilsPlugin;
pub use valence_ident;

use crate::{
    command_channel::{CommandChannel, CommandChannelPlugin},
    ingress::IngressPlugin,
    net::{Compose, ConnectionId, IoBuf, MAX_PACKET_SIZE, PacketDecoder, proxy::init_proxy_comms},
    runtime::AsyncRuntime,
    simulation::{IgnMap, SimPlugin, StreamLookup, blocks::Blocks},
    spatial::SpatialPlugin,
    util::mojang::{ApiProvider, MojangClient},
};

pub mod egress;
pub mod ingress;
pub mod net;
pub mod simulation;
pub mod spatial;
pub mod storage;

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

    if unsafe { getrlimit(RLIMIT_NOFILE, &raw mut limits) } == 0 {
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

    if unsafe { setrlimit(RLIMIT_NOFILE, &raw const limits) } != 0 {
        error!("Failed to set the file handle limits");
        return Err(std::io::Error::last_os_error());
    }

    Ok(())
}

#[derive(Resource)]
pub struct Crypto {
    /// The root certificate authority's certificate
    pub root_ca_cert: CertificateDer<'static>,

    /// The game server's certificate
    pub cert: CertificateDer<'static>,

    /// The game server's private key
    pub key: PrivateKeyDer<'static>,
}

impl Crypto {
    pub fn new(
        root_ca_cert_path: &Path,
        cert_path: &Path,
        key_path: &Path,
    ) -> Result<Self, rustls_pki_types::pem::Error> {
        Ok(Self {
            root_ca_cert: CertificateDer::from_pem_file(root_ca_cert_path)?,
            cert: CertificateDer::from_pem_file(cert_path)?,
            key: PrivateKeyDer::from_pem_file(key_path)?,
        })
    }
}

impl Clone for Crypto {
    fn clone(&self) -> Self {
        Self {
            root_ca_cert: self.root_ca_cert.clone(),
            cert: self.cert.clone(),
            key: self.key.clone_key(),
        }
    }
}

#[derive(Resource, Debug, Clone, PartialEq, Eq, Hash)]
pub struct Endpoint(SocketAddr);

impl From<SocketAddr> for Endpoint {
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

#[derive(Event, Debug, Copy, Clone, PartialEq, Eq, Hash)]
pub struct InitializePlayerPosition(pub Entity);

/// The central [`HyperionCore`] struct which owns and manages the entire server.
pub struct HyperionCore;

impl Plugin for HyperionCore {
    /// Initialize the server.
    fn build(&self, app: &mut App) {
        // 10k players * 2 file handles / player  = 20,000. We can probably get away with 16,384 file handles
        #[cfg(unix)]
        if let Err(e) = adjust_file_descriptor_limits(32_768) {
            warn!("failed to set file limits: {e}");
        }

        // Errors are ignored because they will only occur when the thread pool is initialized
        // twice, which may occur in tests that add the `HyperionCore` plugin to different apps
        let _result = rayon::ThreadPoolBuilder::new()
            .spawn_handler(|thread| {
                std::thread::Builder::new()
                    .stack_size(1024 * 1024)
                    .spawn(move || {
                        thread.run();
                    })
                    .expect("Failed to spawn thread");
                Ok(())
            })
            .build_global();

        // Initialize the compute task pool. This is done manually instead of by using
        // TaskPoolPlugin because TaskPoolPlugin also initializes AsyncComputeTaskPool and
        // IoTaskPool which are not used by Hyperion but are given 50% of the available cores.
        // Setting up ComputeTaskPool manually allows it to use 100% of the available cores.
        let mut init = false;
        bevy::tasks::ComputeTaskPool::get_or_init(|| {
            init = true;
            bevy::tasks::TaskPool::new()
        });
        if !init {
            warn!("failed to initialize ComputeTaskPool because it was already initialized");
        }

        let shared = Arc::new(Shared {
            compression_threshold: CompressionThreshold(256),
            compression_level: CompressionLvl::new(2).expect("failed to create compression level"),
        });

        info!("starting hyperion");
        let config = config::Config::load("run/config.toml").expect("failed to load config");
        app.insert_resource(config);

        let runtime = AsyncRuntime::new();

        let db = LocalDb::new().expect("failed to load database");
        let skins = SkinHandler::new(&db).expect("failed to load skin handler");

        app.insert_resource(db);
        app.insert_resource(skins);
        app.insert_resource(MojangClient::new(&runtime, ApiProvider::MAT_DOES_DEV));
        app.insert_resource(Blocks::empty(&runtime));
        app.add_event::<InitializePlayerPosition>();

        let global = Global::new(shared.clone());

        app.add_plugins(CommandChannelPlugin);

        if let Some(address) = app.world().get_resource::<Endpoint>() {
            let crypto = app.world().resource::<Crypto>();
            let command_channel = app.world().resource::<CommandChannel>();
            init_proxy_comms(&runtime, command_channel.clone(), address.0, crypto.clone());
        } else {
            warn!("Endpoint was not set while loading HyperionCore");
        }

        app.insert_resource(Compose::new(
            shared.compression_level,
            global,
            IoBuf::default(),
        ));
        app.insert_resource(runtime);
        app.insert_resource(CraftingRegistry::default());
        app.insert_resource(StreamLookup::default());

        app.add_plugins((
            bevy::time::TimePlugin,
            bevy::app::ScheduleRunnerPlugin::run_loop(Duration::from_millis(10)),
            IngressPlugin,
            EgressPlugin,
            SimPlugin,
            SpatialPlugin,
            HyperionUtilsPlugin,
        ));

        app.insert_resource(IgnMap::default());
        // Minecraft is 20 TPS
        app.insert_resource(Time::<Fixed>::from_hz(20.0));
    }
}

/// A scratch buffer for intermediate operations. This will return an empty [`Vec`] when calling [`Scratch::obtain`].
#[derive(Debug)]
pub struct Scratch {
    inner: Box<[u8]>,
}

impl Default for Scratch {
    fn default() -> Self {
        Self {
            inner: vec![0u8; MAX_PACKET_SIZE].into_boxed_slice(),
        }
    }
}

/// Nice for getting a buffer that can be used for intermediate work
pub trait ScratchBuffer: sealed::Sealed + Debug {
    /// Obtains a buffer that can be used for intermediate work. The contents are unspecified.
    fn obtain(&mut self) -> &mut [u8];
}

mod sealed {
    pub trait Sealed {}
}

impl sealed::Sealed for Scratch {}

impl ScratchBuffer for Scratch {
    fn obtain(&mut self) -> &mut [u8] {
        &mut self.inner
    }
}

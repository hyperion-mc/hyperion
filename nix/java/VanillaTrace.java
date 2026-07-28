// Records vanilla's own per-tick answer for a scenario, so hyperion's
// simulation can be diffed against the server rather than against a reading of
// the server.
//
//   VanillaTrace <scenario.json> <trace.json> [seed]
//
// Why this is a server subclass and not the published dedicated server: an
// entity only ticks inside a `ServerLevel`, a `ServerLevel` only exists inside
// a `MinecraftServer`, and `MinecraftServer`'s dedicated subclass binds a
// listening socket during startup. A nix build sandbox denies bind, so the
// dedicated server cannot run in a derivation at all. `GameTestServer` is
// Mojang's own answer to the same problem -- a `MinecraftServer` that never
// opens a port, used to run their game tests in CI -- and everything in
// `create` below is that recipe with the test runner replaced by a recorder.
//
// What makes the recording reproducible:
//
//   - No randomness is consumed on the path being measured. `AbstractArrow`'s
//     movement, `Projectile.shoot` at zero inaccuracy and
//     `LivingEntity.knockback` for a non-degenerate direction are all
//     branch-free arithmetic on doubles. The claim is not taken on faith: the
//     recorder derivation replays every scenario under several world seeds and
//     refuses to emit a trace unless all of them agree byte for byte, so a
//     scenario that does reach the level's random source fails the build
//     instead of quietly producing noise.
//   - Mob spawning, daylight, weather and random block ticks are off, so
//     nothing else in the level can move or generate.
//   - The world is the flat preset with a fixed seed, which has no structures
//     and no terrain variation.
//
// The sample at index 0 is the state immediately after the entity is added to
// the level and before any tick has run; sample k is the state after k server
// ticks. `MinecraftServer.tickServer` is what advances the level, so the
// sampling phase is exactly "end of tick", the same phase hyperion's own test
// reads after `world.progress()`.

import com.google.gson.Gson;
import com.google.gson.GsonBuilder;
import com.google.gson.JsonArray;
import com.google.gson.JsonElement;
import com.google.gson.JsonObject;
import com.google.gson.JsonParser;

import com.mojang.authlib.GameProfile;
import com.mojang.authlib.yggdrasil.ServicesKeySet;
import com.mojang.logging.LogUtils;
import com.mojang.serialization.Lifecycle;

import java.io.IOException;
import java.io.UncheckedIOException;
import java.net.Proxy;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.ArrayList;
import java.util.LinkedHashMap;
import java.util.LinkedHashSet;
import java.util.List;
import java.util.Locale;
import java.util.Map;
import java.util.Objects;
import java.util.Optional;
import java.util.Set;
import java.util.UUID;
import java.util.function.BooleanSupplier;

import net.minecraft.SharedConstants;
import net.minecraft.SystemReport;
import net.minecraft.commands.Commands;
import net.minecraft.core.BlockPos;
import net.minecraft.core.MappedRegistry;
import net.minecraft.core.Registry;
import net.minecraft.core.registries.BuiltInRegistries;
import net.minecraft.core.registries.Registries;
import net.minecraft.gizmos.GizmoCollector;
import net.minecraft.gizmos.Gizmos;
import net.minecraft.resources.Identifier;
import net.minecraft.server.Bootstrap;
import net.minecraft.server.MinecraftServer;
import net.minecraft.server.Services;
import net.minecraft.server.WorldLoader;
import net.minecraft.server.WorldStem;
import net.minecraft.server.level.ServerLevel;
import net.minecraft.server.level.progress.LoggingLevelLoadListener;
import net.minecraft.server.notifications.EmptyNotificationService;
import net.minecraft.server.notifications.NotificationManager;
import net.minecraft.server.packs.repository.PackRepository;
import net.minecraft.server.packs.repository.ServerPacksSource;
import net.minecraft.server.permissions.LevelBasedPermissionSet;
import net.minecraft.server.permissions.PermissionSet;
import net.minecraft.server.players.NameAndId;
import net.minecraft.server.players.PlayerList;
import net.minecraft.server.players.ProfileResolver;
import net.minecraft.server.players.UserNameToIdResolver;
import net.minecraft.util.Mth;
import net.minecraft.util.Util;
import net.minecraft.util.debugchart.LocalSampleLogger;
import net.minecraft.util.debugchart.SampleLogger;
import net.minecraft.util.datafix.DataFixers;
import net.minecraft.world.damagesource.DamageSource;
import net.minecraft.world.entity.Entity;
import net.minecraft.world.entity.EntitySpawnReason;
import net.minecraft.world.entity.EntityType;
import net.minecraft.world.entity.LivingEntity;
import net.minecraft.world.entity.projectile.Projectile;
import net.minecraft.world.flag.FeatureFlags;
import net.minecraft.world.level.ChunkPos;
import net.minecraft.world.level.DataPackConfig;
import net.minecraft.world.level.GameType;
import net.minecraft.world.level.LevelSettings;
import net.minecraft.world.level.WorldDataConfiguration;
import net.minecraft.world.level.dimension.LevelStem;
import net.minecraft.world.level.gamerules.GameRuleMap;
import net.minecraft.world.level.gamerules.GameRules;
import net.minecraft.world.level.levelgen.WorldDimensions;
import net.minecraft.world.level.levelgen.WorldGenSettings;
import net.minecraft.world.level.levelgen.WorldOptions;
import net.minecraft.world.level.levelgen.presets.WorldPresets;
import net.minecraft.world.level.storage.LevelDataAndDimensions;
import net.minecraft.world.level.storage.LevelStorageSource;
import net.minecraft.world.level.storage.PrimaryLevelData;
import net.minecraft.world.phys.Vec3;

import org.slf4j.Logger;

public final class VanillaTrace extends MinecraftServer {
    private static final Logger LOGGER = LogUtils.getLogger();

    /// Everything the level can do on its own is turned off here. Without this
    /// a passive mob can generate in a forced chunk and collide with the
    /// projectile being measured, which reads as a physics difference.
    ///
    /// A method rather than a constant because touching `GameRules` pulls in
    /// `BuiltInRegistries`, and a static field would do that during class
    /// initialisation, which is before `main` has had the chance to call
    /// `Bootstrap.bootStrap`.
    private static GameRules quietRules() {
        return new GameRules(
            FeatureFlags.DEFAULT_FLAGS,
            new GameRuleMap.Builder()
                    .set(GameRules.SPAWN_MOBS, false)
                    .set(GameRules.SPAWN_MONSTERS, false)
                    .set(GameRules.SPAWN_PATROLS, false)
                    .set(GameRules.SPAWN_PHANTOMS, false)
                    .set(GameRules.SPAWN_WANDERING_TRADERS, false)
                    .set(GameRules.ADVANCE_TIME, false)
                    .set(GameRules.ADVANCE_WEATHER, false)
                    .set(GameRules.RANDOM_TICK_SPEED, 0)
                    .set(GameRules.MOB_GRIEFING, false)
                    .set(GameRules.PROJECTILES_CAN_BREAK_BLOCKS, false)
                    .build());
    }

    private final Scenario scenario;
    private final long seed;
    private final Path output;
    private final Map<String, Entity> tracked = new LinkedHashMap<>();
    private final List<JsonObject> samples = new ArrayList<>();
    private final LocalSampleLogger tickTimes = new LocalSampleLogger(4);
    private final Set<ChunkPos> forced = new LinkedHashSet<>();
    private int warmup;

    private VanillaTrace(
            Thread serverThread,
            LevelStorageSource.LevelStorageAccess storage,
            PackRepository packs,
            WorldStem stem,
            Scenario scenario,
            long seed,
            Path output) {
        super(
                serverThread,
                storage,
                packs,
                stem,
                Optional.of(quietRules()),
                Proxy.NO_PROXY,
                DataFixers.getDataFixer(),
                offlineServices(),
                LoggingLevelLoadListener.forDedicatedServer(),
                false,
                new NotificationManager());
        this.scenario = scenario;
        this.seed = seed;
        this.output = output;
    }

    public static void main(String[] args) throws Exception {
        if (args.length < 2) {
            System.err.println("usage: VanillaTrace <scenario.json> <trace.json> [seed]");
            System.exit(2);
        }

        Path scenarioPath = Path.of(args[0]);
        Path outputPath = Path.of(args[1]);
        Scenario scenario = Scenario.read(scenarioPath);
        long seed = args.length > 2 ? Long.parseLong(args[2]) : scenario.seed;

        // Without these the block and entity registries are empty and
        // `WorldLoader.load` throws before it reaches the level.
        SharedConstants.tryDetectVersion();
        Bootstrap.bootStrap();

        // A throwaway level directory: the trace is the artefact, the save is
        // not, and writing into the build directory keeps the derivation from
        // needing a writable HOME.
        Path worldRoot = Files.createTempDirectory("vanilla-trace");
        LevelStorageSource storageSource = LevelStorageSource.createDefault(worldRoot);
        LevelStorageSource.LevelStorageAccess storage = storageSource.createAccess("trace");
        PackRepository packs = ServerPacksSource.createPackRepository(storage);
        WorldStem stem = loadFlatWorld(packs, seed);

        // `spin` starts the server thread and returns; `runServer` on that
        // thread calls `initServer`, ticks, and ends in `onServerExit`, which
        // is where the trace is written and the process ends.
        MinecraftServer.spin(
                thread -> new VanillaTrace(thread, storage, packs, stem, scenario, seed, outputPath));
    }

    /// The flat preset at a fixed seed, loaded the way `GameTestServer` loads it.
    ///
    /// Flat rather than the default overworld because it has no structures, no
    /// caves and no surface variation, so the only thing in the level is the
    /// entity under test and the slab of ground far below it.
    private static WorldStem loadFlatWorld(PackRepository packs, long seed) throws Exception {
        packs.reload();
        List<String> enabled = new ArrayList<>(packs.getAvailableIds());
        enabled.remove("vanilla");
        enabled.addFirst("vanilla");

        WorldDataConfiguration dataConfig =
                new WorldDataConfiguration(new DataPackConfig(enabled, List.of()), FeatureFlags.DEFAULT_FLAGS);
        LevelSettings settings = new LevelSettings(
                "vanilla-trace",
                GameType.CREATIVE,
                LevelSettings.DifficultySettings.DEFAULT,
                true,
                dataConfig);
        WorldLoader.PackConfig packConfig = new WorldLoader.PackConfig(packs, dataConfig, false, true);
        WorldLoader.InitConfig initConfig =
                new WorldLoader.InitConfig(packConfig, Commands.CommandSelection.DEDICATED, LevelBasedPermissionSet.OWNER);
        WorldOptions options = new WorldOptions(seed, false, false);

        return Util.<WorldStem>blockUntilDone(executor -> WorldLoader.load(
                        initConfig,
                        context -> {
                            Registry<LevelStem> empty =
                                    new MappedRegistry<LevelStem>(Registries.LEVEL_STEM, Lifecycle.stable()).freeze();
                            WorldDimensions dimensions = context.datapackWorldgen()
                                    .lookupOrThrow(Registries.WORLD_PRESET)
                                    .getOrThrow(WorldPresets.FLAT)
                                    .value()
                                    .createWorldDimensions();
                            WorldDimensions.Complete baked = dimensions.bake(empty);
                            PrimaryLevelData data = new PrimaryLevelData(
                                    settings, baked.specialWorldProperty(), baked.lifecycle());
                            return new WorldLoader.DataLoadOutput<LevelDataAndDimensions.WorldDataAndGenSettings>(
                                    new LevelDataAndDimensions.WorldDataAndGenSettings(
                                            data, new WorldGenSettings(options, dimensions)),
                                    baked.dimensionsRegistryAccess());
                        },
                        WorldStem::new,
                        Util.backgroundExecutor(),
                        executor))
                .get();
    }

    /// No session service and no name lookups: nothing ever authenticates,
    /// because nothing ever connects.
    private static Services offlineServices() {
        UserNameToIdResolver names = new UserNameToIdResolver() {
            @Override
            public void add(NameAndId nameAndId) {}

            @Override
            public Optional<NameAndId> get(String name) {
                return Optional.empty();
            }

            @Override
            public Optional<NameAndId> get(UUID id) {
                return Optional.empty();
            }

            @Override
            public void resolveOfflineUsers(boolean resolve) {}

            @Override
            public void save() {}
        };
        ProfileResolver profiles = new ProfileResolver() {
            @Override
            public Optional<GameProfile> fetchByName(String name) {
                return Optional.empty();
            }

            @Override
            public Optional<GameProfile> fetchById(UUID id) {
                return Optional.empty();
            }
        };
        return new Services(null, ServicesKeySet.EMPTY, null, names, profiles);
    }

    // `MinecraftServer` leaves fourteen methods abstract for its two real
    // subclasses to answer, and every one of them describes a facility this
    // harness does not have: no console, no rcon, no chat, no network. The
    // answers below are the "none of that exists" ones.

    @Override
    public boolean isSingleplayerOwner(NameAndId nameAndId) {
        return false;
    }

    @Override
    public boolean shouldInformAdmins() {
        return false;
    }

    @Override
    public boolean isPublished() {
        return false;
    }

    @Override
    public boolean isDedicatedServer() {
        return false;
    }

    @Override
    public boolean shouldRconBroadcast() {
        return false;
    }

    @Override
    public boolean useNativeTransport() {
        return false;
    }

    @Override
    public int getRateLimitPacketsPerSecond() {
        return 0;
    }

    @Override
    public int getCommandSpamThresholdSeconds() {
        return 0;
    }

    @Override
    public int getChatSpamThresholdSeconds() {
        return 0;
    }

    @Override
    public LevelBasedPermissionSet operatorUserPermissions() {
        return LevelBasedPermissionSet.OWNER;
    }

    @Override
    public PermissionSet getFunctionCompilationPermissions() {
        return LevelBasedPermissionSet.OWNER;
    }

    @Override
    protected SampleLogger getTickTimeLogger() {
        return tickTimes;
    }

    @Override
    public int getMaxPlayers() {
        return 0;
    }

    @Override
    public SystemReport fillServerSystemReport(SystemReport report) {
        report.setDetail("Type", "Vanilla trace recorder");
        return report;
    }

    @Override
    protected boolean initServer() {
        // `PlayerList` is abstract but declares no abstract members; the empty
        // subclass is what `GameTestServer` uses for the same reason.
        setPlayerList(new PlayerList(this, registries(), this.playerDataStorage, new EmptyNotificationService()) {});
        Gizmos.withCollector(GizmoCollector.NOOP);
        loadLevel();
        forceChunks(overworld());
        LOGGER.info("recording {} for {} ticks at seed {}", scenario.name, scenario.ticks, seed);
        return true;
    }

    /// Claims every chunk the run could possibly reach.
    ///
    /// A level only ticks entities inside chunks that have climbed all the way
    /// to entity ticking, and nothing here loads chunks the way a nearby
    /// player would. The radius is a bound rather than a guess: `shoot`
    /// normalises before scaling and `knockback` scales a unit vector, so an
    /// entity's initial speed is exactly the declared power, and no vanilla
    /// drag term ever increases speed. `ticks * speed` therefore cannot be
    /// exceeded, whatever the trajectory does vertically.
    ///
    /// Claiming them is not the same as having them. Promotion to entity
    /// ticking runs over several ticks inside the chunk system, which is why
    /// `tickServer` waits for `chunksReady` before it spawns anything: an
    /// entity that spends its first thirty ticks in a chunk that is merely
    /// loaded does not move, and the trace records a stationary arrow with no
    /// hint that anything went wrong.
    private void forceChunks(ServerLevel level) {
        double speed = 0.0;
        for (Scenario.EntitySpec spec : scenario.entities) {
            speed = Math.max(speed, spec.initialSpeed());
        }
        int radius = (int) Math.ceil(scenario.ticks * speed / 16.0) + 2;

        for (Scenario.EntitySpec spec : scenario.entities) {
            int cx = Mth.floor(spec.position[0]) >> 4;
            int cz = Mth.floor(spec.position[2]) >> 4;
            for (int dx = -radius; dx <= radius; dx++) {
                for (int dz = -radius; dz <= radius; dz++) {
                    forced.add(new ChunkPos(cx + dx, cz + dz));
                }
            }
        }
        for (ChunkPos pos : forced) {
            level.setChunkForced(pos.x(), pos.z(), true);
        }
        LOGGER.info("forced {} chunks (radius {})", forced.size(), radius);
    }

    private boolean chunksReady(ServerLevel level) {
        for (ChunkPos pos : forced) {
            if (!level.isPositionEntityTicking(new BlockPos(pos.getMiddleBlockX(), 64, pos.getMiddleBlockZ()))) {
                return false;
            }
        }
        return true;
    }

    private Entity spawn(ServerLevel level, Scenario.EntitySpec spec) {
        EntityType<?> type = BuiltInRegistries.ENTITY_TYPE.getValue(Identifier.parse(spec.type));
        if (type == null) {
            throw new IllegalArgumentException("no such entity type: " + spec.type);
        }
        Entity entity = type.create(level, EntitySpawnReason.COMMAND);
        if (entity == null) {
            throw new IllegalArgumentException("entity type refused to spawn: " + spec.type);
        }
        entity.setPos(spec.position[0], spec.position[1], spec.position[2]);

        if (spec.launch != null) {
            if (!(entity instanceof Projectile projectile)) {
                throw new IllegalArgumentException(spec.type + " is not a projectile, so it cannot be launched");
            }
            // `Projectile.shootFromRotation` with a stationary shooter and a
            // zero pitch offset, inlined because it wants an `Entity` source
            // only to read that shooter's own movement. The three components
            // are vanilla's, sine table and all; `shoot` then normalises and
            // scales them, which is where the initial speed actually comes
            // from. Inaccuracy is fixed at zero, so the one random term on
            // this path -- the bow's spread -- is not exercised.
            float yaw = spec.launch.yaw * ((float) Math.PI / 180);
            float pitch = spec.launch.pitch * ((float) Math.PI / 180);
            float xd = -Mth.sin(yaw) * Mth.cos(pitch);
            float yd = -Mth.sin(pitch);
            float zd = Mth.cos(yaw) * Mth.cos(pitch);
            projectile.shoot(xd, yd, zd, spec.launch.power, 0.0f);
        } else if (spec.motion != null) {
            entity.setDeltaMovement(spec.motion[0], spec.motion[1], spec.motion[2]);
        }

        if (spec.knockback != null) {
            if (!(entity instanceof LivingEntity living)) {
                throw new IllegalArgumentException(spec.type + " is not a living entity, so it cannot be knocked back");
            }
            // `knockback` reads `onGround`, which decides whether the vertical
            // term is applied at all, so the scenario states it rather than
            // letting it depend on where the entity happens to have landed.
            entity.setOnGround(spec.knockback.onGround);
            DamageSource source = level.damageSources().generic();
            living.knockback(
                    spec.knockback.power,
                    spec.knockback.fromX,
                    spec.knockback.fromZ,
                    source,
                    spec.knockback.damage);
        }

        if (!level.addFreshEntity(entity)) {
            throw new IllegalStateException("level refused the entity: " + spec.type);
        }
        return entity;
    }

    /// A run that spends this many ticks waiting for chunks has hit something
    /// other than ordinary chunk loading, and saying so beats emitting a trace
    /// of an entity that never moved.
    private static final int WARMUP_LIMIT = 600;

    @Override
    protected void tickServer(BooleanSupplier haveTime) {
        super.tickServer(haveTime);
        ServerLevel level = overworld();

        if (tracked.isEmpty()) {
            warmup++;
            if (chunksReady(level)) {
                for (Scenario.EntitySpec spec : scenario.entities) {
                    tracked.put(spec.id, spawn(level, spec));
                }
                LOGGER.info("chunks ready after {} ticks; spawned {} entities", warmup, tracked.size());
                sample();
            } else if (warmup >= WARMUP_LIMIT) {
                throw new IllegalStateException(
                        "chunks were still not entity ticking after " + WARMUP_LIMIT + " ticks");
            }
            return;
        }

        sample();
        // ticks + 1 because index 0 is the state before the first tick.
        if (samples.size() > scenario.ticks) {
            halt(false);
        }
    }

    private void sample() {
        JsonObject entities = new JsonObject();
        for (Map.Entry<String, Entity> entry : tracked.entrySet()) {
            Entity entity = entry.getValue();
            JsonObject state = new JsonObject();
            state.add("position", vec(entity.position()));
            state.add("velocity", vec(entity.getDeltaMovement()));
            // The client-facing orientation, which is the whole of the "wrong
            // heading" this trace exists to pin down. A projectile's yRot is
            // `atan2(dx, dz)` and its xRot `atan2(dy, horizontalDistance)`, both
            // derived from the velocity every tick in `AbstractArrow.tick`, and
            // both in the projectile-entity sign convention rather than the
            // look-direction one a shooter's own yaw uses. Recorded [yaw, pitch]
            // to match the order hyperion stores Yaw before Pitch.
            JsonArray rotation = new JsonArray();
            rotation.add(entity.getYRot());
            rotation.add(entity.getXRot());
            state.add("rotation", rotation);
            state.addProperty("removed", entity.isRemoved());
            entities.add(entry.getKey(), state);
        }
        JsonObject sample = new JsonObject();
        sample.addProperty("tick", samples.size());
        sample.add("entities", entities);
        samples.add(sample);
    }

    private static JsonArray vec(Vec3 value) {
        JsonArray array = new JsonArray();
        array.add(value.x);
        array.add(value.y);
        array.add(value.z);
        return array;
    }

    /// Ticks as fast as the machine allows rather than at twenty a second.
    /// The default sleeps until the next tick boundary, which would make a
    /// sixty tick recording take three seconds of wall clock for no reason.
    @Override
    protected void waitUntilNextTick() {
        runAllTasks();
    }

    @Override
    public boolean isTickTimeLoggingEnabled() {
        return false;
    }

    @Override
    protected void onServerExit() {
        super.onServerExit();
        try {
            write();
        } catch (IOException e) {
            throw new UncheckedIOException(e);
        }
        System.exit(0);
    }

    private void write() throws IOException {
        JsonObject trace = new JsonObject();
        trace.addProperty("scenario", scenario.name);
        trace.addProperty("minecraftVersion", SharedConstants.getCurrentVersion().name());
        trace.addProperty("seed", seed);
        trace.addProperty("ticks", scenario.ticks);
        JsonArray array = new JsonArray();
        samples.forEach(array::add);
        trace.add("samples", array);

        Gson gson = new GsonBuilder().setPrettyPrinting().create();
        Path parent = output.toAbsolutePath().getParent();
        if (parent != null) {
            Files.createDirectories(parent);
        }
        Files.writeString(output, gson.toJson(trace) + "\n", StandardCharsets.UTF_8);
        LOGGER.info("wrote {} samples to {}", samples.size(), output);
    }

    /// A recorded scenario, mirroring `docs/differential-testing.md`.
    private static final class Scenario {
        private String name;
        private String description;
        private int ticks;
        private long seed;
        private List<EntitySpec> entities = List.of();

        private static final class EntitySpec {
            private String id;
            private String type;
            private double[] position;
            private double[] motion;
            private Launch launch;
            private Knockback knockback;

            /// The speed this entity starts with, in blocks per tick.
            ///
            /// Exact rather than approximate for the two impulse forms:
            /// `Projectile.shoot` normalises its direction before scaling by
            /// the power, and `LivingEntity.knockback` scales a unit vector,
            /// so in both cases the power is the magnitude.
            private double initialSpeed() {
                double speed = 0.0;
                if (motion != null) {
                    speed = Math.sqrt(motion[0] * motion[0] + motion[1] * motion[1] + motion[2] * motion[2]);
                }
                if (launch != null) {
                    speed = Math.max(speed, launch.power);
                }
                if (knockback != null) {
                    speed = Math.max(speed, knockback.power);
                }
                return speed;
            }
        }

        private static final class Launch {
            private float yaw;
            private float pitch;
            private float power;
        }

        private static final class Knockback {
            private double power;
            private double fromX;
            private double fromZ;
            private float damage;
            private boolean onGround;
        }

        private static Scenario read(Path path) throws IOException {
            JsonElement root = JsonParser.parseString(Files.readString(path, StandardCharsets.UTF_8));
            Scenario scenario = new Gson().fromJson(root, Scenario.class);
            Objects.requireNonNull(scenario.name, "scenario is missing a name");
            if (scenario.ticks <= 0) {
                throw new IllegalArgumentException(scenario.name + ": ticks must be positive");
            }
            if (scenario.entities.isEmpty()) {
                throw new IllegalArgumentException(scenario.name + ": no entities to record");
            }
            for (EntitySpec spec : scenario.entities) {
                Objects.requireNonNull(spec.id, "entity is missing an id");
                Objects.requireNonNull(spec.type, spec.id + ": entity is missing a type");
                if (spec.position == null || spec.position.length != 3) {
                    throw new IllegalArgumentException(spec.id + ": position must be three numbers");
                }
                if (spec.launch != null && spec.motion != null) {
                    throw new IllegalArgumentException(
                            String.format(Locale.ROOT, "%s: launch and motion are alternatives", spec.id));
                }
            }
            return scenario;
        }
    }
}

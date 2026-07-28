// Prints bytes produced by Mojang's own encoders, so a Rust codec can be
// checked against the server rather than against a reading of the server.
//
// Everything here runs against the pinned server jar and nothing else. Two
// commands, for two different consumers:
//
//   fixtures <file>     writes JSON: named hex strings that `tests/` compares
//                       its own encoder's output against. A file rather than
//                       stdout because `Bootstrap.bootStrap` redirects stdout
//                       through the logger, which would prefix every line.
//   packets             prints `name -> hex` for the play packets alone, for
//                       reading rather than for diffing. The same builders
//                       feed `fixtures`, so the two can never disagree.
//   registries <dir>    writes the network NBT of every synchronised registry
//                       element, which `nix/generate-rust.py` turns into the
//                       tables under `src/generated`.
//   tags <dir>          writes the whole tag map as `ClientboundUpdateTagsPacket`
//                       puts it on the wire, which `nix/generate-tag-data.py`
//                       turns into `src/tag_data`.
//   verify-tags <dir>   binds only the tags in that dump and then loads the
//                       registries the way a joining client does, so a tag set
//                       too small to join fails here instead of on a player's
//                       screen.
//
// Adding a fixture means adding a `put` call below and a matching assertion in
// Rust. Both halves live in version control, which is the point: issue #970 is
// that this used to be retyped from scratch every time somebody needed it.

import com.google.common.collect.ImmutableMultimap;

import com.google.gson.JsonArray;
import com.google.gson.JsonElement;
import com.google.gson.JsonObject;
import com.google.gson.JsonParser;

import com.mojang.authlib.GameProfile;
import com.mojang.authlib.properties.Property;
import com.mojang.authlib.properties.PropertyMap;
import com.mojang.datafixers.util.Pair;
import com.mojang.serialization.DynamicOps;

import io.netty.buffer.ByteBuf;
import io.netty.buffer.Unpooled;
import io.netty.channel.embedded.EmbeddedChannel;

import it.unimi.dsi.fastutil.ints.IntList;

import it.unimi.dsi.fastutil.shorts.ShortArraySet;
import it.unimi.dsi.fastutil.shorts.ShortSet;

import java.io.IOException;
import java.io.PrintStream;
import java.lang.reflect.Field;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.ArrayList;
import java.util.BitSet;
import java.util.Comparator;
import java.util.EnumMap;
import java.util.EnumSet;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;
import java.util.Optional;
import java.util.UUID;
import java.util.stream.Stream;

import javax.crypto.Cipher;
import javax.crypto.spec.IvParameterSpec;
import javax.crypto.spec.SecretKeySpec;

import net.minecraft.SharedConstants;
import net.minecraft.core.BlockPos;
import net.minecraft.core.Holder;

import net.minecraft.core.HolderLookup;
import net.minecraft.core.IdMap;
import net.minecraft.core.particles.BlockParticleOption;
import net.minecraft.core.particles.ColorParticleOption;
import net.minecraft.core.particles.DustColorTransitionOptions;
import net.minecraft.core.particles.DustParticleOptions;
import net.minecraft.core.particles.GeyserBaseParticleOptions;
import net.minecraft.core.particles.GeyserParticleOptions;
import net.minecraft.core.particles.ItemParticleOption;
import net.minecraft.core.particles.ParticleOptions;
import net.minecraft.core.particles.ParticleTypes;
import net.minecraft.core.particles.PowerParticleOption;
import net.minecraft.core.particles.SculkChargeParticleOptions;
import net.minecraft.core.particles.ShriekParticleOption;
import net.minecraft.core.particles.SpellParticleOption;
import net.minecraft.core.particles.TrailParticleOption;
import net.minecraft.core.particles.VibrationParticleOption;

import net.minecraft.core.LayeredRegistryAccess;
import net.minecraft.core.Registry;
import net.minecraft.core.RegistryAccess;
import net.minecraft.core.SectionPos;
import net.minecraft.core.component.DataComponentInitializers;
import net.minecraft.core.registries.BuiltInRegistries;
import net.minecraft.core.registries.Registries;
import net.minecraft.nbt.CompoundTag;
import net.minecraft.nbt.ListTag;
import net.minecraft.nbt.NbtOps;
import net.minecraft.nbt.Tag;
import net.minecraft.network.CipherEncoder;
import net.minecraft.network.CompressionEncoder;
import net.minecraft.network.FriendlyByteBuf;
import net.minecraft.network.RegistryFriendlyByteBuf;
import net.minecraft.network.Varint21LengthFieldPrepender;
import net.minecraft.network.chat.Component;
import net.minecraft.network.codec.ByteBufCodecs;
import net.minecraft.network.codec.StreamCodec;
import net.minecraft.network.protocol.game.ClientboundAddEntityPacket;
import net.minecraft.network.protocol.game.ClientboundContainerSetContentPacket;
import net.minecraft.network.protocol.game.ClientboundContainerSetSlotPacket;
import net.minecraft.network.protocol.game.ClientboundDamageEventPacket;
import net.minecraft.network.protocol.game.ClientboundHurtAnimationPacket;
import net.minecraft.network.protocol.game.ClientboundLevelParticlesPacket;
import net.minecraft.network.protocol.game.ClientboundRemoveMobEffectPacket;
import net.minecraft.network.protocol.game.ClientboundUpdateMobEffectPacket;
import net.minecraft.network.protocol.game.ClientboundPlayerAbilitiesPacket;
import net.minecraft.network.protocol.game.ClientboundPlayerCombatKillPacket;
import net.minecraft.network.protocol.game.ClientboundPlayerInfoUpdatePacket;
import net.minecraft.network.protocol.game.ClientboundRemoveEntitiesPacket;
import net.minecraft.network.protocol.game.ClientboundSectionBlocksUpdatePacket;
import net.minecraft.network.protocol.game.ClientboundSetEntityDataPacket;
import net.minecraft.network.protocol.game.ClientboundSetEntityMotionPacket;
import net.minecraft.network.protocol.game.ClientboundSetEquipmentPacket;
import net.minecraft.network.protocol.game.ClientboundSetExperiencePacket;
import net.minecraft.network.protocol.game.ClientboundSetHealthPacket;
import net.minecraft.network.protocol.game.ClientboundUpdateAttributesPacket;
import net.minecraft.network.syncher.EntityDataSerializer;
import net.minecraft.network.syncher.EntityDataSerializers;
import net.minecraft.network.syncher.SynchedEntityData;
import net.minecraft.resources.Identifier;
import net.minecraft.resources.RegistryDataLoader;
import net.minecraft.resources.ResourceKey;
import net.minecraft.server.Bootstrap;
import net.minecraft.server.RegistryLayer;
import net.minecraft.server.packs.PackType;
import net.minecraft.server.packs.repository.ServerPacksSource;
import net.minecraft.server.packs.resources.CloseableResourceManager;
import net.minecraft.server.packs.resources.MultiPackResourceManager;
import net.minecraft.tags.TagLoader;
import net.minecraft.tags.TagNetworkSerialization;
import net.minecraft.world.damagesource.DamageType;
import net.minecraft.world.damagesource.DamageTypes;
import net.minecraft.world.entity.EntityType;
import net.minecraft.world.entity.EntityTypes;
import net.minecraft.world.entity.EquipmentSlot;
import net.minecraft.world.entity.ai.attributes.AttributeInstance;
import net.minecraft.world.entity.ai.attributes.AttributeModifier;
import net.minecraft.world.entity.ai.attributes.Attributes;
import net.minecraft.world.entity.player.Abilities;
import net.minecraft.world.effect.MobEffect;
import net.minecraft.world.effect.MobEffectInstance;
import net.minecraft.world.item.ItemStack;
import net.minecraft.world.item.ItemStackTemplate;
import net.minecraft.world.item.Items;
import net.minecraft.world.level.GameType;
import net.minecraft.world.level.biome.Biome;
import net.minecraft.world.level.biome.Biomes;
import net.minecraft.world.level.block.Block;
import net.minecraft.world.level.block.Blocks;
import net.minecraft.world.level.gameevent.BlockPositionSource;
import net.minecraft.world.level.block.state.BlockState;
import net.minecraft.world.level.chunk.LevelChunkSection;
import net.minecraft.world.level.chunk.PalettedContainer;
import net.minecraft.world.level.chunk.PalettedContainerFactory;
import net.minecraft.world.level.chunk.Strategy;
import net.minecraft.world.level.levelgen.Heightmap;
import net.minecraft.world.phys.Vec3;

public final class VanillaEncoder {
    /// Fixture ordering is part of the output, so a map that keeps insertion
    /// order rather than one that sorts by hash.
    private final Map<String, String> fixtures = new LinkedHashMap<>();

    public static void main(String[] args) throws Exception {
        if (args.length < 1 || (args.length < 2 && !args[0].equals("packets"))) {
            System.err.println(
                    "usage: VanillaEncoder (fixtures <file> | packets | registries <dir>"
                            + " | tags <dir> | verify-tags <dir>)");
            System.exit(2);
        }

        // `Bootstrap.bootStrap` replaces both standard streams with ones that
        // funnel through log4j, so the handle for the `packets` listing is
        // taken before that happens rather than after.
        PrintStream stdout = System.out;

        // Without these the block state registry is empty and every codec
        // below either throws or silently encodes nothing.
        SharedConstants.tryDetectVersion();
        Bootstrap.bootStrap();

        VanillaEncoder encoder = new VanillaEncoder();
        // `loadServerLayers` is per command rather than hoisted: it ends by
        // applying the data pack's tags to the built-in registries, and
        // `verify-tags` exists precisely to load with no tags but the dumped
        // ones bound.
        switch (args[0]) {
            case "fixtures" -> encoder.fixtures(Path.of(args[1]), loadServerLayers().compositeAccess());
            case "packets" -> encoder.printPackets(stdout, loadServerLayers().compositeAccess());
            case "registries" -> encoder.registries(Path.of(args[1]), loadServerLayers().compositeAccess());
            case "tags" -> encoder.tags(Path.of(args[1]), loadServerLayers());
            case "verify-tags" -> verifyTags(Path.of(args[1]));
            default -> {
                System.err.println("unknown command: " + args[0]);
                System.exit(2);
            }
        }
    }

    /// The registry layers a running server has, not the built-in ones alone.
    ///
    /// `Bootstrap.bootStrap` only fills `BuiltInRegistries`. Three things a
    /// server does afterwards are load bearing here, and each one is its own
    /// crash when skipped:
    ///
    ///   - the worldgen registries come out of the vanilla data pack, so
    ///     without loading it `Registries.BIOME` is absent and
    ///     `PalettedContainerFactory.create` throws;
    ///   - tags are bound into the built-in registries only once that pack's
    ///     tag files have been read;
    ///   - item data component prototypes are attached by
    ///     `DataComponentInitializers`, and until that has run every
    ///     `new ItemStack(...)` throws "Components not bound yet".
    ///
    /// `WorldLoader.load` is the method that does all three, but it wants a
    /// level directory and a `WorldDataSupplier` this harness has no reason to
    /// invent, so the same steps run here in the same order. The last two
    /// mirror `ReloadableServerResources.updateComponentsAndStaticRegistryTags`,
    /// which is the jar's only caller of `PendingComponents.apply`.
    ///
    /// The layers rather than the composite access they flatten into, because
    /// `TagNetworkSerialization.serializeTagsToNetwork` takes the layered form
    /// and `RegistrySynchronization.networkSafeRegistries` reads the layer a
    /// registry came from to decide whether a client tracks it at all.
    private static LayeredRegistryAccess<RegistryLayer> loadServerLayers() {
        // Closing the manager is safe: `TagLoader` and `RegistryDataLoader`
        // both read eagerly, so nothing below holds a resource open.
        try (CloseableResourceManager resources = vanillaResources()) {
            LayeredRegistryAccess<RegistryLayer> layers = RegistryLayer.createRegistryAccess();
            List<Registry.PendingTags<?>> staticTags = TagLoader.loadTagsForExistingRegistries(
                    resources, layers.getLayer(RegistryLayer.STATIC));
            List<HolderLookup.RegistryLookup<?>> taggedStatic = TagLoader.buildUpdatedLookups(
                    layers.getAccessForLoading(RegistryLayer.WORLDGEN), staticTags);

            // `Runnable::run` rather than a pool: this load is the only work in
            // flight, and a same-thread executor keeps a failure on this stack
            // instead of wrapped in a `CompletionException`.
            RegistryAccess.Frozen worldgen = RegistryDataLoader
                    .load(resources, taggedStatic, RegistryDataLoader.WORLDGEN_REGISTRIES, Runnable::run)
                    .join();

            staticTags.forEach(Registry.PendingTags::apply);
            HolderLookup.Provider full = HolderLookup.Provider.create(
                    Stream.concat(taggedStatic.stream(), worldgen.listRegistries()));
            BuiltInRegistries.DATA_COMPONENT_INITIALIZERS
                    .build(full)
                    .forEach(DataComponentInitializers.PendingComponents::apply);

            return layers.replaceFrom(RegistryLayer.WORLDGEN, worldgen);
        }
    }

    private static CloseableResourceManager vanillaResources() {
        return new MultiPackResourceManager(
                PackType.SERVER_DATA, List.of(ServerPacksSource.createVanillaPackSource()));
    }

    // --- fixtures ---------------------------------------------------------

    private void fixtures(Path out, RegistryAccess.Frozen registries) throws IOException {
        framing();
        palettedContainers(registries);
        heightmaps();
        lightData();
        playPackets(registries);
        Files.writeString(out, toJson(), StandardCharsets.UTF_8);
    }

    /// The `packets` command: the same builders `fixtures` uses, listed for
    /// reading. Anything named here is also in `vanilla.json`, so a value read
    /// off this listing is a value some Rust test is already asserting.
    private void printPackets(PrintStream out, RegistryAccess.Frozen registries) {
        playPackets(registries);
        for (Map.Entry<String, String> entry : fixtures.entrySet()) {
            out.println(entry.getKey() + " -> " + entry.getValue());
        }
        out.flush();
    }

    /// Frames driven through the real netty handlers rather than a
    /// transcription of them, so the threshold comparison and the
    /// uncompressed-length-zero convention come from `CompressionEncoder`
    /// itself.
    private void framing() {
        byte[] small = repeat((byte) 0x11, 8);
        byte[] large = repeat((byte) 0x22, 512);

        put("frame.plain.empty_body", hex(prepend(new byte[] { 0x00 })));
        put("frame.plain.small", hex(prepend(concat(new byte[] { 0x2A }, small))));

        // 256 is the vanilla default (`ServerProperties.networkCompressionThreshold`),
        // and 8 and 512 straddle it.
        put("frame.compressed_256.below", hex(prepend(compress(256, concat(new byte[] { 0x2A }, small)))));
        put("frame.compressed_256.above", hex(prepend(compress(256, concat(new byte[] { 0x2A }, large)))));

        // A body of exactly the threshold: `CompressionEncoder` compares the
        // id-plus-body length with `<`, so this one is compressed. The wiki
        // has long described the boundary the other way round.
        byte[] exact = repeat((byte) 0x33, 64 - 1);
        put("frame.compressed_64.exact", hex(prepend(compress(64, concat(new byte[] { 0x2A }, exact)))));
        byte[] justUnder = repeat((byte) 0x33, 64 - 2);
        put("frame.compressed_64.just_under", hex(prepend(compress(64, concat(new byte[] { 0x2A }, justUnder)))));

        // Encryption covers the length prefix, so the cipher runs over the
        // already-framed bytes. The secret doubles as the IV, per Crypt.
        byte[] secret = new byte[16];
        for (int i = 0; i < secret.length; i++) {
            secret[i] = (byte) (i * 17);
        }
        put("secret", hex(secret));
        put("frame.encrypted.plain_small", encrypt(secret, prepend(concat(new byte[] { 0x2A }, small))));

        byte[] framed = prepend(compress(256, concat(new byte[] { 0x2A }, large)));
        put("frame.encrypted.compressed_256_above", encrypt(secret, framed));

        // Two frames through one cipher: CFB8 carries its shift register
        // across them, so the second is not what enciphering it alone gives.
        EmbeddedChannel channel = new EmbeddedChannel(new CipherEncoder(cipher(Cipher.ENCRYPT_MODE, secret)));
        byte[] first = prepend(concat(new byte[] { 0x01 }, small));
        byte[] second = prepend(concat(new byte[] { 0x02 }, small));
        put("frame.encrypted.stream_first", hex(runOutbound(channel, first)));
        put("frame.encrypted.stream_second", hex(runOutbound(channel, second)));
    }

    /// Paletted containers at each palette kind the block-state strategy can
    /// pick, since the byte before the palette is the in-memory bit width and
    /// not the number of entries.
    private void palettedContainers(RegistryAccess registries) {
        Strategy<BlockState> strategy = Strategy.createForBlockStates(Block.BLOCK_STATE_REGISTRY);

        BlockState air = Blocks.AIR.defaultBlockState();
        BlockState stone = Blocks.STONE.defaultBlockState();
        BlockState dirt = Blocks.DIRT.defaultBlockState();

        put("block_state_id.air", Integer.toString(Block.BLOCK_STATE_REGISTRY.getId(air)));
        put("block_state_id.stone", Integer.toString(Block.BLOCK_STATE_REGISTRY.getId(stone)));
        put("block_state_id.dirt", Integer.toString(Block.BLOCK_STATE_REGISTRY.getId(dirt)));

        PalettedContainer<BlockState> single = new PalettedContainer<>(air, strategy);
        put("palette.single_air", hex(writeContainer(single)));

        PalettedContainer<BlockState> allStone = new PalettedContainer<>(stone, strategy);
        put("palette.single_stone", hex(writeContainer(allStone)));

        // One stone block in an air section: two palette entries, so a linear
        // palette, which the strategy pads to four bits in memory.
        PalettedContainer<BlockState> twoEntries = new PalettedContainer<>(air, strategy);
        twoEntries.set(0, 0, 0, stone);
        put("palette.linear_two", hex(writeContainer(twoEntries)));

        PalettedContainer<BlockState> threeEntries = new PalettedContainer<>(air, strategy);
        threeEntries.set(0, 0, 0, stone);
        threeEntries.set(1, 0, 0, dirt);
        threeEntries.set(15, 15, 15, dirt);
        put("palette.linear_three", hex(writeContainer(threeEntries)));

        // Biomes use a 2-bit axis, so 64 entries rather than 4096, and a
        // different bits-to-palette table. The ids are the datapack-loaded
        // registry's own `asHolderIdMap`, which is what a server sends; the
        // `listElements` order of a `VanillaRegistries.createLookup` provider
        // used to stand in for it here and disagreed, because that provider
        // patches the built-in registries rather than reproducing the order a
        // `MappedRegistry` assigns while reading the data pack.
        Registry<Biome> biomes = registries.lookupOrThrow(Registries.BIOME);
        IdMap<Holder<Biome>> biomeIds = biomes.asHolderIdMap();
        Strategy<Holder<Biome>> biomeStrategy = Strategy.createForBiomes(biomeIds);

        Holder<Biome> plains = biomes.getOrThrow(Biomes.PLAINS);
        Holder<Biome> desert = biomes.getOrThrow(Biomes.DESERT);
        put("biome_id.plains", Integer.toString(biomeIds.getId(plains)));
        put("biome_id.desert", Integer.toString(biomeIds.getId(desert)));

        PalettedContainer<Holder<Biome>> singleBiome = new PalettedContainer<>(plains, biomeStrategy);
        put("palette.biome_single", hex(writeContainer(singleBiome)));

        PalettedContainer<Holder<Biome>> twoBiomes = new PalettedContainer<>(plains, biomeStrategy);
        twoBiomes.set(0, 0, 0, desert);
        put("palette.biome_linear_two", hex(writeContainer(twoBiomes)));
    }

    /// The heightmap map codec, which in 26.2 is a `VarInt`-keyed map of
    /// `long[]` rather than the NBT compound older versions sent.
    private void heightmaps() {
        StreamCodec<ByteBuf, Map<Heightmap.Types, long[]>> codec = ByteBufCodecs.map(
                size -> new EnumMap<>(Heightmap.Types.class),
                Heightmap.Types.STREAM_CODEC,
                ByteBufCodecs.LONG_ARRAY);

        for (Heightmap.Types type : Heightmap.Types.values()) {
            put("heightmap_id." + type.getSerializationKey(), Integer.toString(type.ordinal()));
            put("heightmap_client." + type.getSerializationKey(), Boolean.toString(type.sendToClient()));
        }

        Map<Heightmap.Types, long[]> empty = new EnumMap<>(Heightmap.Types.class);
        ByteBuf buffer = Unpooled.buffer();
        codec.encode(buffer, empty);
        put("heightmaps.empty", hex(drain(buffer)));

        // A world 384 blocks tall needs 9 bits per column and 256 columns, so
        // 37 longs at 7 columns each. The values are arbitrary; the framing
        // around them is what is being pinned.
        Map<Heightmap.Types, long[]> filled = new EnumMap<>(Heightmap.Types.class);
        long[] data = new long[37];
        for (int i = 0; i < data.length; i++) {
            data[i] = 0x0123456789ABCDEFL + i;
        }
        filled.put(Heightmap.Types.MOTION_BLOCKING, data);
        filled.put(Heightmap.Types.WORLD_SURFACE, new long[] { 1L, 2L });
        buffer = Unpooled.buffer();
        codec.encode(buffer, filled);
        put("heightmaps.two", hex(drain(buffer)));
    }

    /// The light section of `level_chunk_with_light`, built from its parts:
    /// `ClientboundLightUpdatePacketData` needs a live light engine, but its
    /// `write` is only these six calls.
    private void lightData() {
        StreamCodec<ByteBuf, byte[]> dataLayer = ByteBufCodecs.byteArray(2048);

        BitSet sky = new BitSet();
        sky.set(0);
        sky.set(3);
        sky.set(25);
        FriendlyByteBuf buffer = new FriendlyByteBuf(Unpooled.buffer());
        buffer.writeBitSet(sky);
        put("bitset.0_3_25", hex(drain(buffer)));

        buffer = new FriendlyByteBuf(Unpooled.buffer());
        buffer.writeBitSet(new BitSet());
        put("bitset.empty", hex(drain(buffer)));

        byte[] layer = new byte[2048];
        for (int i = 0; i < layer.length; i++) {
            layer[i] = (byte) (i & 0xFF);
        }
        buffer = new FriendlyByteBuf(Unpooled.buffer());
        buffer.writeCollection(List.of(layer), dataLayer);
        put("light.one_layer", hex(drain(buffer)));

        buffer = new FriendlyByteBuf(Unpooled.buffer());
        buffer.writeCollection(List.of(), dataLayer);
        put("light.no_layers", hex(drain(buffer)));
    }

    // --- play packets -----------------------------------------------------

    /// A `UUID` with every byte distinct, so a transposed half shows up.
    private static final UUID PROFILE_ID = UUID.fromString("00112233-4455-6677-8899-aabbccddeeff");

    /// Clientbound play packets the extractor could not follow, encoded
    /// through their own `StreamCodec`.
    ///
    /// Each one is built with values that are wrong in a visible way if a
    /// field is dropped, reordered or sized differently: no zeroes, no
    /// defaults, and asymmetric coordinates so an x/z swap is not a fixpoint.
    private void playPackets(RegistryAccess.Frozen registries) {
        addEntity(registries);
        setEntityData(registries);
        setEntityMotion(registries);
        setEquipment(registries);
        playerInfoUpdate(registries);
        playerAbilities(registries);
        sectionBlocksUpdate(registries);
        levelParticles(registries);
        mobEffects(registries);
        containerPackets(registries);
        combatPackets(registries);
    }

    private void addEntity(RegistryAccess registries) {
        put("entity_type_id.pig", Integer.toString(BuiltInRegistries.ENTITY_TYPE.getId(EntityTypes.PIG)));

        // The three rotations are `Mth.packDegrees` bytes on the wire, not
        // floats, so the packing is pinned separately from the packet: a Rust
        // helper that rounds the other way would otherwise only show up as one
        // wrong byte in a 40-byte blob.
        put("packed_degrees.0", Integer.toString(net.minecraft.util.Mth.packDegrees(0.0f)));
        put("packed_degrees.90", Integer.toString(net.minecraft.util.Mth.packDegrees(90.0f)));
        put("packed_degrees.-45.5", Integer.toString(net.minecraft.util.Mth.packDegrees(-45.5f)));
        put("packed_degrees.179.9", Integer.toString(net.minecraft.util.Mth.packDegrees(179.9f)));
        put("packed_degrees.-179.9", Integer.toString(net.minecraft.util.Mth.packDegrees(-179.9f)));

        ClientboundAddEntityPacket packet = new ClientboundAddEntityPacket(
                0x2A,
                PROFILE_ID,
                1.5,
                64.0625,
                -2.25,
                12.5f,
                -45.5f,
                EntityTypes.PIG,
                7,
                new Vec3(0.25, -0.5, 0.125),
                179.9);
        put("packet.add_entity", encode(registries, ClientboundAddEntityPacket.STREAM_CODEC, packet));
    }

    private void setEntityData(RegistryAccess registries) {
        // The serializer id is an index into a registration-ordered bimap, so
        // it moves whenever Mojang inserts one. Pinning the ids the Rust
        // helpers hard-code is what makes that a test failure rather than a
        // silently mistyped field.
        putSerializerId("byte", EntityDataSerializers.BYTE);
        putSerializerId("int", EntityDataSerializers.INT);
        putSerializerId("long", EntityDataSerializers.LONG);
        putSerializerId("float", EntityDataSerializers.FLOAT);
        putSerializerId("string", EntityDataSerializers.STRING);
        putSerializerId("component", EntityDataSerializers.COMPONENT);
        putSerializerId("optional_component", EntityDataSerializers.OPTIONAL_COMPONENT);
        putSerializerId("item_stack", EntityDataSerializers.ITEM_STACK);
        putSerializerId("boolean", EntityDataSerializers.BOOLEAN);
        putSerializerId("block_pos", EntityDataSerializers.BLOCK_POS);

        List<SynchedEntityData.DataValue<?>> values = List.of(
                new SynchedEntityData.DataValue<>(0, EntityDataSerializers.BYTE, (byte) 0x21),
                new SynchedEntityData.DataValue<>(1, EntityDataSerializers.INT, -1234),
                new SynchedEntityData.DataValue<>(2, EntityDataSerializers.LONG, 1234567890123L),
                new SynchedEntityData.DataValue<>(3, EntityDataSerializers.FLOAT, 12.5f),
                new SynchedEntityData.DataValue<>(4, EntityDataSerializers.STRING, "hello"),
                new SynchedEntityData.DataValue<>(5, EntityDataSerializers.BOOLEAN, true),
                new SynchedEntityData.DataValue<>(6, EntityDataSerializers.COMPONENT, Component.literal("hi")),
                new SynchedEntityData.DataValue<>(
                        7, EntityDataSerializers.OPTIONAL_COMPONENT, Optional.of(Component.literal("hi"))),
                new SynchedEntityData.DataValue<>(
                        8, EntityDataSerializers.OPTIONAL_COMPONENT, Optional.<Component>empty()),
                new SynchedEntityData.DataValue<>(
                        9, EntityDataSerializers.ITEM_STACK, new ItemStack(Items.DIAMOND_SWORD, 3)),
                new SynchedEntityData.DataValue<>(10, EntityDataSerializers.ITEM_STACK, ItemStack.EMPTY),
                new SynchedEntityData.DataValue<>(
                        11, EntityDataSerializers.BLOCK_POS, new net.minecraft.core.BlockPos(1, -2, 3)));

        put("packet.set_entity_data", encode(
                registries,
                ClientboundSetEntityDataPacket.STREAM_CODEC,
                new ClientboundSetEntityDataPacket(0x2A, values)));

        // The terminator is the whole body when there is nothing to send, and
        // a codec that wrote a count instead would still look plausible.
        put("packet.set_entity_data.empty", encode(
                registries,
                ClientboundSetEntityDataPacket.STREAM_CODEC,
                new ClientboundSetEntityDataPacket(1, List.of())));
    }

    /// `Vec3.LP_STREAM_CODEC`, which is where the interesting cases are: the
    /// encoding quantises against a per-vector scale and only writes a
    /// continuation `VarInt` when that scale needs more than two bits.
    private void setEntityMotion(RegistryAccess registries) {
        Map<String, Vec3> vectors = new LinkedHashMap<>();
        // Below `ABS_MIN_VALUE`: one zero byte and nothing else.
        vectors.put("zero", Vec3.ZERO);
        vectors.put("subnormal", new Vec3(1.0e-6, -1.0e-6, 0.0));
        // Scale 1..3 fits the two marker bits, so no continuation.
        vectors.put("small", new Vec3(0.25, -0.5, 0.125));
        vectors.put("scale_one_exact", new Vec3(1.0, -1.0, 0.5));
        vectors.put("scale_three", new Vec3(2.0, -1.0, 3.0));
        // 3.25 ceils to 4, the first scale that needs the continuation.
        vectors.put("scale_four", new Vec3(1.5, -3.25, 2.0));
        vectors.put("scale_large", new Vec3(100.5, -0.5, 20.0));
        // Past `ABS_MAX_VALUE`, where `sanitize` clamps.
        vectors.put("clamped", new Vec3(1.0e12, -1.0e12, 0.0));
        vectors.put("nan", new Vec3(Double.NaN, 1.0, Double.NaN));

        for (Map.Entry<String, Vec3> entry : vectors.entrySet()) {
            ByteBuf buffer = Unpooled.buffer();
            Vec3.LP_STREAM_CODEC.encode(buffer, entry.getValue());
            put("lp_vec3." + entry.getKey(), hex(drain(buffer)));

            put("packet.set_entity_motion." + entry.getKey(), encode(
                    registries,
                    ClientboundSetEntityMotionPacket.STREAM_CODEC,
                    new ClientboundSetEntityMotionPacket(0x2A, entry.getValue())));
        }
    }

    /// Every option shape a particle can carry, driven through the game's own
    /// dispatch rather than through a transcription of it.
    ///
    /// `nix/generate-particles.py` reads these shapes out of the decompiled
    /// Java, which means the Rust it emits is only as right as that reading.
    /// These fixtures are what checks the reading: Mojang's encoder and the
    /// generated one have to produce the same bytes, so a codec parsed wrong
    /// fails here rather than on a client.
    ///
    /// The type id is also pinned separately for two of them, because a
    /// registry insertion moves every id after it and one wrong id inside a
    /// forty-byte blob is hard to read off a diff.
    private void levelParticles(RegistryAccess.Frozen registries) {
        put("particle_id.flame", Integer.toString(BuiltInRegistries.PARTICLE_TYPE.getId(ParticleTypes.FLAME)));
        put("particle_id.dust", Integer.toString(BuiltInRegistries.PARTICLE_TYPE.getId(ParticleTypes.DUST)));
        // `block_state_id.stone` is already pinned by `sectionBlocksUpdate`.

        Map<String, ParticleOptions> options = new LinkedHashMap<>();
        // A `SimpleParticleType`: the id and not one byte more, which is what
        // makes the count before it the last field a reader can find unaided.
        options.put("flame", ParticleTypes.FLAME);
        // The six the effects layer names, one per body shape.
        options.put("block", new BlockParticleOption(ParticleTypes.BLOCK, Blocks.STONE.defaultBlockState()));
        options.put("item", new ItemParticleOption(ParticleTypes.ITEM, new ItemStackTemplate(Items.DIAMOND_SWORD, 3)));
        options.put("dust", new DustParticleOptions(0xFFFF0000, 2.0f));
        options.put("dust_color_transition", new DustColorTransitionOptions(0xFFFF0000, 0xFF0000FF, 1.5f));
        options.put("entity_effect", ColorParticleOption.create(ParticleTypes.ENTITY_EFFECT, 0x80336699));
        options.put(
                "vibration",
                new VibrationParticleOption(new BlockPositionSource(new BlockPos(1, -2, 3)), 40));
        // The remaining shapes, so a codec misread anywhere fails here too.
        options.put("dragon_breath", PowerParticleOption.create(ParticleTypes.DRAGON_BREATH, 1.0f));
        options.put("effect", SpellParticleOption.create(ParticleTypes.EFFECT, 0xFF112233, 0.75f));
        options.put("sculk_charge", new SculkChargeParticleOptions(0.5f));
        options.put("shriek", new ShriekParticleOption(17));
        options.put("trail", new TrailParticleOption(new Vec3(1.5, -2.25, 3.75), 0xFF00FF00, 30));
        options.put("geyser", new GeyserParticleOptions(ParticleTypes.GEYSER, 5));
        options.put("geyser_base", new GeyserBaseParticleOptions(ParticleTypes.GEYSER_BASE, 5, 0.25f));

        for (Map.Entry<String, ParticleOptions> entry : options.entrySet()) {
            ByteBuf buffer = Unpooled.buffer();
            ParticleTypes.STREAM_CODEC.encode(
                    new RegistryFriendlyByteBuf(buffer, registries), entry.getValue());
            put("particle." + entry.getKey(), hex(drain(buffer)));

            put("packet.level_particles." + entry.getKey(), encode(
                    registries,
                    ClientboundLevelParticlesPacket.STREAM_CODEC,
                    new ClientboundLevelParticlesPacket(
                            entry.getValue(),
                            true,
                            false,
                            1.5,
                            64.0625,
                            -2.25,
                            0.5f,
                            0.25f,
                            0.125f,
                            0.75f,
                            100)));
        }
    }

    /// The two mob-effect packets, through the game's own `holderRegistry`
    /// codec.
    ///
    /// The effect id is what matters: `MobEffect.STREAM_CODEC` is
    /// `holderRegistry`, and this crate types the field as a plain
    /// `RegistryId`, so if `holderRegistry` biased the id the way the inline
    /// `holder` codec does, these bytes would be off by one and a slow would
    /// arrive as the wrong effect. The registry ids are pinned separately for
    /// the two the tooltips reach for.
    private void mobEffects(RegistryAccess.Frozen registries) {
        Registry<MobEffect> effects = registries.lookupOrThrow(Registries.MOB_EFFECT);
        // By name rather than through the `MobEffects` constants, which are not
        // in the decompiled subset this harness is otherwise written against.
        Holder<MobEffect> slowness = effects.getOrThrow(
                ResourceKey.create(Registries.MOB_EFFECT, Identifier.withDefaultNamespace("slowness")));
        Holder<MobEffect> speed = effects.getOrThrow(
                ResourceKey.create(Registries.MOB_EFFECT, Identifier.withDefaultNamespace("speed")));

        put("mob_effect_id.slowness", Integer.toString(BuiltInRegistries.MOB_EFFECT.getId(slowness.value())));
        put("mob_effect_id.speed", Integer.toString(BuiltInRegistries.MOB_EFFECT.getId(speed.value())));

        // Slowness IV for a second and a half, particles and icon on: the shape
        // an immobilise or a web-slow takes. Amplifier is zero-based, so IV is
        // 3, and 30 ticks is 1.5 s.
        MobEffectInstance slow = new MobEffectInstance(slowness, 30, 3, false, true, true);
        put("packet.update_mob_effect.slowness", encode(
                registries,
                ClientboundUpdateMobEffectPacket.STREAM_CODEC,
                new ClientboundUpdateMobEffectPacket(0x2A, slow, false)));

        // Speed II, indefinite (-1), which is the caster-buff shape.
        MobEffectInstance fast = new MobEffectInstance(speed, -1, 1, false, true, true);
        put("packet.update_mob_effect.speed", encode(
                registries,
                ClientboundUpdateMobEffectPacket.STREAM_CODEC,
                new ClientboundUpdateMobEffectPacket(0x2A, fast, false)));

        put("packet.remove_mob_effect.slowness", encode(
                registries,
                ClientboundRemoveMobEffectPacket.STREAM_CODEC,
                new ClientboundRemoveMobEffectPacket(0x2A, slowness)));
    }

    private void setEquipment(RegistryAccess registries) {
        put("item_id.diamond_sword", Integer.toString(BuiltInRegistries.ITEM.getId(Items.DIAMOND_SWORD)));
        put("item_id.stone", Integer.toString(BuiltInRegistries.ITEM.getId(Items.STONE)));

        for (EquipmentSlot slot : EquipmentSlot.values()) {
            put("equipment_slot." + slot.getSerializedName(), Integer.toString(slot.ordinal()));
        }

        // Three entries so both the continuation bit and its absence on the
        // last one are exercised, and an empty stack in the middle because
        // that is the case `OPTIONAL_STREAM_CODEC` shortens to a single byte.
        List<Pair<EquipmentSlot, ItemStack>> slots = List.of(
                Pair.of(EquipmentSlot.MAINHAND, new ItemStack(Items.DIAMOND_SWORD, 1)),
                Pair.of(EquipmentSlot.HEAD, ItemStack.EMPTY),
                Pair.of(EquipmentSlot.SADDLE, new ItemStack(Items.STONE, 64)));
        put("packet.set_equipment", encode(
                registries,
                ClientboundSetEquipmentPacket.STREAM_CODEC,
                new ClientboundSetEquipmentPacket(0x2A, slots)));

        put("packet.set_equipment.single", encode(
                registries,
                ClientboundSetEquipmentPacket.STREAM_CODEC,
                new ClientboundSetEquipmentPacket(
                        1, List.of(Pair.of(EquipmentSlot.OFFHAND, new ItemStack(Items.STONE, 2))))));
    }

    private void playerInfoUpdate(RegistryAccess registries) {
        for (ClientboundPlayerInfoUpdatePacket.Action action : ClientboundPlayerInfoUpdatePacket.Action.values()) {
            put("player_info_action." + action.name().toLowerCase(java.util.Locale.ROOT),
                    Integer.toString(action.ordinal()));
        }

        PropertyMap properties = new PropertyMap(ImmutableMultimap.of(
                "textures", new Property("textures", "dGV4dHVyZQ==", "c2lnbmF0dXJl")));
        GameProfile profile = new GameProfile(PROFILE_ID, "Notch", properties);

        ClientboundPlayerInfoUpdatePacket.Entry entry = new ClientboundPlayerInfoUpdatePacket.Entry(
                PROFILE_ID,
                profile,
                true,
                42,
                GameType.CREATIVE,
                Component.literal("Notch"),
                true,
                7,
                null);

        // Every action but INITIALIZE_CHAT, whose payload is a signed profile
        // key this harness has no way to build.
        EnumSet<ClientboundPlayerInfoUpdatePacket.Action> all = EnumSet.of(
                ClientboundPlayerInfoUpdatePacket.Action.ADD_PLAYER,
                ClientboundPlayerInfoUpdatePacket.Action.UPDATE_GAME_MODE,
                ClientboundPlayerInfoUpdatePacket.Action.UPDATE_LISTED,
                ClientboundPlayerInfoUpdatePacket.Action.UPDATE_LATENCY,
                ClientboundPlayerInfoUpdatePacket.Action.UPDATE_DISPLAY_NAME,
                ClientboundPlayerInfoUpdatePacket.Action.UPDATE_LIST_ORDER,
                ClientboundPlayerInfoUpdatePacket.Action.UPDATE_HAT);
        put("packet.player_info_update", encode(
                registries,
                ClientboundPlayerInfoUpdatePacket.STREAM_CODEC,
                playerInfoPacket(all, List.of(entry))));

        // A null display name and a profile with no properties: the two
        // absent-value shapes ADD_PLAYER and UPDATE_DISPLAY_NAME each have.
        ClientboundPlayerInfoUpdatePacket.Entry bare = new ClientboundPlayerInfoUpdatePacket.Entry(
                PROFILE_ID,
                new GameProfile(PROFILE_ID, "Bare", new PropertyMap(ImmutableMultimap.of())),
                false,
                0,
                GameType.SURVIVAL,
                null,
                false,
                0,
                null);
        EnumSet<ClientboundPlayerInfoUpdatePacket.Action> minimal = EnumSet.of(
                ClientboundPlayerInfoUpdatePacket.Action.ADD_PLAYER,
                ClientboundPlayerInfoUpdatePacket.Action.UPDATE_LISTED,
                ClientboundPlayerInfoUpdatePacket.Action.UPDATE_LATENCY,
                ClientboundPlayerInfoUpdatePacket.Action.UPDATE_DISPLAY_NAME);
        put("packet.player_info_update.minimal", encode(
                registries,
                ClientboundPlayerInfoUpdatePacket.STREAM_CODEC,
                playerInfoPacket(minimal, List.of(bare, entry))));
    }

    /// The packet with a chosen entry list.
    ///
    /// Its public constructors all take live `ServerPlayer`s, which need a
    /// running server; the field the codec reads is set directly instead. The
    /// bytes still come from the real `write`, which is the point of the
    /// fixture.
    private static ClientboundPlayerInfoUpdatePacket playerInfoPacket(
            EnumSet<ClientboundPlayerInfoUpdatePacket.Action> actions,
            List<ClientboundPlayerInfoUpdatePacket.Entry> entries) {
        ClientboundPlayerInfoUpdatePacket packet = new ClientboundPlayerInfoUpdatePacket(actions, List.of());
        try {
            Field field = ClientboundPlayerInfoUpdatePacket.class.getDeclaredField("entries");
            field.setAccessible(true);
            field.set(packet, entries);
        } catch (ReflectiveOperationException e) {
            throw new IllegalStateException("ClientboundPlayerInfoUpdatePacket.entries is no longer settable", e);
        }
        return packet;
    }

    private void playerAbilities(RegistryAccess registries) {
        Abilities abilities = new Abilities();
        abilities.invulnerable = true;
        abilities.flying = false;
        abilities.mayfly = true;
        abilities.instabuild = true;
        abilities.setFlyingSpeed(0.05f);
        abilities.setWalkingSpeed(0.1f);
        put("packet.player_abilities", encode(
                registries,
                ClientboundPlayerAbilitiesPacket.STREAM_CODEC,
                new ClientboundPlayerAbilitiesPacket(abilities)));

        Abilities none = new Abilities();
        none.setFlyingSpeed(0.0f);
        none.setWalkingSpeed(0.0f);
        put("packet.player_abilities.none", encode(
                registries,
                ClientboundPlayerAbilitiesPacket.STREAM_CODEC,
                new ClientboundPlayerAbilitiesPacket(none)));
    }

    private void sectionBlocksUpdate(RegistryAccess registries) {
        LevelChunkSection section = new LevelChunkSection(PalettedContainerFactory.create(registries));
        // `setBlockState`'s locking overload asserts a chunk lock is held, and
        // there is no chunk here.
        section.setBlockState(1, 3, 2, Blocks.STONE.defaultBlockState(), false);
        section.setBlockState(15, 15, 0, Blocks.DIRT.defaultBlockState(), false);

        // `ShortArraySet` rather than a hash set: the packet writes in
        // iteration order, and only an insertion-ordered set makes that
        // reproducible enough to commit.
        ShortSet changes = new ShortArraySet();
        changes.add(packSectionRelative(1, 3, 2));
        changes.add(packSectionRelative(15, 15, 0));

        put("section_relative.1_3_2", Integer.toString(packSectionRelative(1, 3, 2)));
        put("section_relative.15_15_0", Integer.toString(packSectionRelative(15, 15, 0)));
        put("section_pos.3_-1_7", Long.toString(SectionPos.of(3, -1, 7).asLong()));

        put("packet.section_blocks_update", encode(
                registries,
                ClientboundSectionBlocksUpdatePacket.STREAM_CODEC,
                new ClientboundSectionBlocksUpdatePacket(SectionPos.of(3, -1, 7), changes, section)));
    }

    /// `SectionPos`'s relative packing: x in bits 8..12, z in 4..8, y in 0..4.
    private static short packSectionRelative(int x, int y, int z) {
        return (short) (x << 8 | z << 4 | y);
    }

    private void containerPackets(RegistryAccess registries) {
        List<ItemStack> items = List.of(
                new ItemStack(Items.STONE, 64),
                ItemStack.EMPTY,
                new ItemStack(Items.DIAMOND_SWORD, 1));
        put("packet.container_set_content", encode(
                registries,
                ClientboundContainerSetContentPacket.STREAM_CODEC,
                new ClientboundContainerSetContentPacket(3, 17, items, new ItemStack(Items.STONE, 2))));

        put("packet.container_set_slot", encode(
                registries,
                ClientboundContainerSetSlotPacket.STREAM_CODEC,
                new ClientboundContainerSetSlotPacket(3, 17, 9, new ItemStack(Items.DIAMOND_SWORD, 1))));

        // Slot -1 with the player's own container id, which is how a server
        // sets the cursor stack; a `short` field written as a `VarInt` would
        // pass every positive case and fail this one.
        put("packet.container_set_slot.cursor", encode(
                registries,
                ClientboundContainerSetSlotPacket.STREAM_CODEC,
                new ClientboundContainerSetSlotPacket(0, 1, -1, ItemStack.EMPTY)));
    }

    /// The combat and player-status packets.
    ///
    /// `ClientboundEntityEventPacket` is missing on purpose: its only public
    /// constructor takes a live `Entity`, which needs a `Level`, which needs a
    /// server. Its vector stays hand-derived in `tests/play_combat.rs`.
    private void combatPackets(RegistryAccess.Frozen registries) {
        Registry<DamageType> damageTypes = registries.lookupOrThrow(Registries.DAMAGE_TYPE);
        Holder<DamageType> arrow = damageTypes.getOrThrow(DamageTypes.ARROW);
        Holder<DamageType> generic = damageTypes.getOrThrow(DamageTypes.GENERIC);
        put("damage_type_id.arrow", Integer.toString(damageTypes.getId(arrow.value())));
        put("damage_type_id.generic", Integer.toString(damageTypes.getId(generic.value())));

        // `writeOptionalEntityId` sends `id + 1`, so a present cause of zero
        // and an absent one differ by exactly one byte. -1 is what the
        // `DamageSource` constructor passes for "no entity".
        put("packet.damage_event", encode(
                registries,
                ClientboundDamageEventPacket.STREAM_CODEC,
                new ClientboundDamageEventPacket(0x2A, arrow, 0, -1, Optional.empty())));
        put("packet.damage_event.full", encode(
                registries,
                ClientboundDamageEventPacket.STREAM_CODEC,
                new ClientboundDamageEventPacket(
                        1, generic, 7, 9, Optional.of(new Vec3(1.5, -2.0, 0.25)))));

        put("packet.hurt_animation", encode(
                registries,
                ClientboundHurtAnimationPacket.STREAM_CODEC,
                new ClientboundHurtAnimationPacket(42, 90.0f)));

        put("packet.player_combat_kill", encode(
                registries,
                ClientboundPlayerCombatKillPacket.STREAM_CODEC,
                new ClientboundPlayerCombatKillPacket(42, Component.literal("hi"))));

        // 300 is the two-byte `VarInt` that catches a list written as
        // fixed-width ints, and the empty case pins that the count is written
        // even when there is nothing after it.
        put("packet.remove_entities", encode(
                registries,
                ClientboundRemoveEntitiesPacket.STREAM_CODEC,
                new ClientboundRemoveEntitiesPacket(1, 300)));
        put("packet.remove_entities.empty", encode(
                registries,
                ClientboundRemoveEntitiesPacket.STREAM_CODEC,
                new ClientboundRemoveEntitiesPacket()));

        put("packet.set_health", encode(
                registries,
                ClientboundSetHealthPacket.STREAM_CODEC,
                new ClientboundSetHealthPacket(20.0f, 20, 5.0f)));

        // The constructor is (progress, total, level) while the wire order is
        // progress, level, total, so a level that differs from the total is
        // the only value that catches a transposition.
        put("packet.set_experience", encode(
                registries,
                ClientboundSetExperiencePacket.STREAM_CODEC,
                new ClientboundSetExperiencePacket(0.5f, 0, 30)));

        put("attribute_id.armor",
                Integer.toString(BuiltInRegistries.ATTRIBUTE.getId(Attributes.ARMOR.value())));

        AttributeInstance armor = new AttributeInstance(Attributes.ARMOR, instance -> {
        });
        armor.setBaseValue(3.0);
        put("packet.update_attributes", encode(
                registries,
                ClientboundUpdateAttributesPacket.STREAM_CODEC,
                new ClientboundUpdateAttributesPacket(42, List.of(armor))));

        AttributeInstance modified = new AttributeInstance(Attributes.ARMOR, instance -> {
        });
        modified.setBaseValue(3.0);
        modified.addPermanentModifier(new AttributeModifier(
                Identifier.parse("hi"), 0.5, AttributeModifier.Operation.ADD_MULTIPLIED_BASE));
        put("packet.update_attributes.modifier", encode(
                registries,
                ClientboundUpdateAttributesPacket.STREAM_CODEC,
                new ClientboundUpdateAttributesPacket(42, List.of(modified))));
    }

    private void putSerializerId(String name, EntityDataSerializer<?> serializer) {
        put("entity_data_serializer." + name, Integer.toString(EntityDataSerializers.getSerializedId(serializer)));
    }

    /// Drive one packet through its own codec.
    ///
    /// The bound is `? super RegistryFriendlyByteBuf` because the three buffer
    /// types form a chain: a packet declaring `ByteBuf` or `FriendlyByteBuf`
    /// still accepts the registry-carrying one.
    private static <T> String encode(
            RegistryAccess registries, StreamCodec<? super RegistryFriendlyByteBuf, T> codec, T packet) {
        RegistryFriendlyByteBuf buffer = new RegistryFriendlyByteBuf(Unpooled.buffer(), registries);
        codec.encode(buffer, packet);
        return hex(drain(buffer));
    }

    // --- registry dump ----------------------------------------------------

    /// Writes the network NBT of every synchronised registry element.
    ///
    /// This is `RegistrySynchronization.packRegistry` with an empty set of
    /// client-known packs, which is the case where the server sends contents
    /// rather than a bare name. The bytes are what `ByteBufCodecs.TAG` puts on
    /// the wire: a type byte and then the payload, with no root name.
    ///
    /// The provider is the datapack-loaded one rather than
    /// `VanillaRegistries.createLookup`. That is a correctness fix, not a
    /// tidy-up: the lookups a `RegistrySetBuilder` hands back iterate in the
    /// order of a `HashMap` keyed on freshly allocated holders, so two runs of
    /// this command listed the same registry's elements in different orders
    /// and the committed tables under `src/registry_data` only matched
    /// whichever run happened to produce them. A `MappedRegistry` iterates by
    /// id, which is both stable and the order the server assigns.
    private void registries(Path outDir, RegistryAccess.Frozen provider) throws IOException {
        // `SynchronizeRegistriesTask` encodes with
        // `registries.createSerializationContext(NbtOps.INSTANCE)`, not with
        // bare `NbtOps`. The difference matters: a `RegistryOps` writes a
        // `HolderSet` as the tag's own name, where plain `NbtOps` tries to
        // dereference the tag and throws.
        DynamicOps<Tag> ops = provider.createSerializationContext(NbtOps.INSTANCE);
        Files.createDirectories(outDir);

        StringBuilder index = new StringBuilder("[\n");
        boolean firstRegistry = true;

        for (RegistryDataLoader.RegistryData<?> registryData : RegistryDataLoader.SYNCHRONIZED_REGISTRIES) {
            List<String> ids = new ArrayList<>();
            List<byte[]> payloads = new ArrayList<>();
            // An element codec that throws used to be caught and the registry
            // listed in `skipped.json`, because tags were unbound under
            // `VanillaRegistries.createLookup` and several codecs dereference
            // one. The datapack load binds them, so nothing is expected to
            // throw here any more, and a throw that does happen is a bug
            // rather than a gap: letting it out kills the run instead of
            // writing a table that is silently short.
            if (!dumpRegistry(provider, ops, registryData, ids, payloads)) {
                continue;
            }

            String registryName = registryData.key().identifier().toString();
            Path file = outDir.resolve(fileNameFor(registryName, "nbt"));
            ByteBuf blob = Unpooled.buffer();
            for (byte[] payload : payloads) {
                blob.writeBytes(payload);
            }
            Files.write(file, drain(blob));

            if (!firstRegistry) {
                index.append(",\n");
            }
            firstRegistry = false;
            index.append("  {\"registry\": ").append(quote(registryName));
            index.append(", \"file\": ").append(quote(file.getFileName().toString()));
            index.append(", \"entries\": [");
            for (int i = 0; i < ids.size(); i++) {
                if (i > 0) {
                    index.append(", ");
                }
                index.append("{\"id\": ").append(quote(ids.get(i)));
                index.append(", \"length\": ").append(payloads.get(i).length).append("}");
            }
            index.append("]}");
        }
        index.append("\n]\n");
        Files.writeString(outDir.resolve("index.json"), index.toString(), StandardCharsets.UTF_8);
    }

    /// The same tag with every compound rebuilt in sorted key order, so the
    /// bytes depend on the data and not on the run that produced them.
    ///
    /// Two things combine into unstable output without this.
    /// `DimensionType`'s `EnvironmentAttributeMap` is keyed on registry
    /// objects whose hash codes are identity hash codes, so the order its
    /// entries reach the encoder in changes on every JVM. And a `CompoundTag`
    /// writes its entries in its backing `HashMap`'s iteration order, which
    /// keeps insertion order within a bucket, so that changing order reaches
    /// the wire. `dimension_type.nbt` and `timeline.nbt` therefore differed
    /// between two runs of this command, and the copies committed under
    /// `src/registry_data` only ever matched the run that wrote them.
    ///
    /// Sorting the keys before inserting them fixes the insertion order,
    /// which fixes the iteration order of any map keyed on those strings.
    ///
    /// Nothing is lost: an NBT compound is unordered, a client reads the same
    /// element either way, and a vanilla server's own order is no more
    /// canonical than this one, being whatever its last restart produced.
    private static Tag canonicalize(Tag tag) {
        if (tag instanceof CompoundTag compound) {
            CompoundTag rebuilt = new CompoundTag();
            for (String key : compound.keySet().stream().sorted().toList()) {
                rebuilt.put(key, canonicalize(compound.get(key)));
            }
            return rebuilt;
        }
        if (tag instanceof ListTag list) {
            ListTag rebuilt = new ListTag();
            for (Tag element : list) {
                rebuilt.add(rebuilt.size(), canonicalize(element));
            }
            return rebuilt;
        }
        return tag;
    }

    /// Returns false when this build carries no registry under that key, which
    /// happens for registries a data pack would have to supply.
    private <T> boolean dumpRegistry(
            HolderLookup.Provider provider,
            DynamicOps<Tag> ops,
            RegistryDataLoader.RegistryData<T> registryData,
            List<String> ids,
            List<byte[]> payloads) {
        Optional<? extends HolderLookup.RegistryLookup<T>> lookup = provider.lookup(registryData.key());
        if (lookup.isEmpty()) {
            return false;
        }
        lookup.get().listElements().forEach(element -> {
            Tag encoded = canonicalize(registryData.elementCodec()
                    .encodeStart(ops, element.value())
                    .getOrThrow(message -> new IllegalStateException(
                            "failed to encode " + element.key() + ": " + message)));
            ByteBuf buffer = Unpooled.buffer();
            ByteBufCodecs.TAG.encode(buffer, encoded);
            ids.add(element.key().identifier().toString());
            payloads.add(drain(buffer));
        });
        return true;
    }

    // --- tag dump ---------------------------------------------------------

    /// Writes the whole tag map, laid out as `ClientboundUpdateTagsPacket`
    /// puts it on the wire.
    ///
    /// One `.bin` per registry holding, for each of its tags, the bytes
    /// `FriendlyByteBuf.writeIntIdList` produces: a `VarInt` count then the
    /// network id of every element. An `index.json` names the tags and how many
    /// bytes each one occupies, so the Rust generator never parses the blob.
    ///
    /// The ids are positions in the registry the tag indexes into, so this dump
    /// is only usable by a server that sends `registry_data` in exactly the
    /// element order `registries` dumped. Both come from the same
    /// `MappedRegistry` iteration, which is by id.
    private void tags(Path outDir, LayeredRegistryAccess<RegistryLayer> layers) throws IOException {
        Files.createDirectories(outDir);

        Map<ResourceKey<? extends Registry<?>>, TagNetworkSerialization.NetworkPayload> payloads =
                TagNetworkSerialization.serializeTagsToNetwork(layers);

        // `serializeTagsToNetwork` collects into a `HashMap` keyed on
        // `ResourceKey`, which does not override `hashCode`, and each payload
        // holds a second `HashMap`. Both iteration orders therefore change from
        // one JVM to the next, so registries and tag names are sorted for the
        // same reason `canonicalize` sorts NBT keys: a map is unordered on the
        // wire, and the committed tables have to be reproducible.
        List<ResourceKey<? extends Registry<?>>> registryKeys = new ArrayList<>(payloads.keySet());
        registryKeys.sort(Comparator.comparing(key -> key.identifier().toString()));

        StringBuilder index = new StringBuilder("[\n");
        boolean firstRegistry = true;
        for (ResourceKey<? extends Registry<?>> registryKey : registryKeys) {
            // The map inside a `NetworkPayload` is private, so the payload is
            // written and read straight back rather than reached into. That
            // also proves the bytes below are the ones vanilla would send.
            FriendlyByteBuf written = new FriendlyByteBuf(Unpooled.buffer());
            payloads.get(registryKey).write(written);
            Map<Identifier, IntList> tags =
                    written.readMap(FriendlyByteBuf::readIdentifier, FriendlyByteBuf::readIntIdList);

            List<Identifier> names = new ArrayList<>(tags.keySet());
            names.sort(Comparator.comparing(Identifier::toString));

            String registryName = registryKey.identifier().toString();
            Path file = outDir.resolve(fileNameFor(registryName, "bin"));
            ByteBuf blob = Unpooled.buffer();

            if (!firstRegistry) {
                index.append(",\n");
            }
            firstRegistry = false;
            index.append("  {\"registry\": ").append(quote(registryName));
            index.append(", \"file\": ").append(quote(file.getFileName().toString()));
            index.append(", \"tags\": [");
            for (int i = 0; i < names.size(); i++) {
                Identifier name = names.get(i);
                IntList ids = tags.get(name);
                FriendlyByteBuf encoded = new FriendlyByteBuf(Unpooled.buffer());
                encoded.writeIntIdList(ids);
                byte[] payload = drain(encoded);
                blob.writeBytes(payload);

                if (i > 0) {
                    index.append(", ");
                }
                index.append("{\"name\": ").append(quote(name.toString()));
                index.append(", \"entries\": ").append(ids.size());
                index.append(", \"length\": ").append(payload.length).append("}");
            }
            index.append("]}");

            Files.write(file, drain(blob));
        }
        index.append("\n]\n");
        Files.writeString(outDir.resolve("index.json"), index.toString(), StandardCharsets.UTF_8);
    }

    // --- tag verification -------------------------------------------------

    /// Loads the registries the way a joining client does, with only the tags
    /// in `dumpDir` bound.
    ///
    /// This is the check the disconnect that motivated this command would have
    /// failed. A client does not keep the tags it read from its own packs:
    /// `update_tags` replaces the whole map, and only then does it parse the
    /// registry elements from its vanilla pack. An element naming a tag that is
    /// not bound throws "Missing tag", one throw fails the whole registry load,
    /// and that fails `finish_configuration` with "Network Protocol Error".
    ///
    /// So the tags come from the dump rather than from `TagLoader`, and
    /// `RegistryDataLoader` then parses the same files the client parses with
    /// the same codecs. Running it against an empty dump reproduces the
    /// disconnect; running it against the full one is what the check asserts.
    ///
    /// Not a rendering client: this proves the data a client is sent loads, not
    /// that the socket dance around it works. `nix run .#e2e` covers the wire.
    private static void verifyTags(Path dumpDir) throws IOException {
        Map<String, TagNetworkSerialization.NetworkPayload> dumped = readTagDump(dumpDir);

        try (CloseableResourceManager resources = vanillaResources()) {
            LayeredRegistryAccess<RegistryLayer> layers = RegistryLayer.createRegistryAccess();

            List<Registry.PendingTags<?>> pending = new ArrayList<>();
            layers.getLayer(RegistryLayer.STATIC).registries().forEach(entry -> {
                TagNetworkSerialization.NetworkPayload payload =
                        dumped.get(entry.key().identifier().toString());
                if (payload != null) {
                    pending.add(prepareTags(entry.value(), payload));
                }
            });

            List<HolderLookup.RegistryLookup<?>> tagged = TagLoader.buildUpdatedLookups(
                    layers.getAccessForLoading(RegistryLayer.WORLDGEN), pending);
            RegistryAccess.Frozen worldgen = RegistryDataLoader
                    .load(resources, tagged, RegistryDataLoader.WORLDGEN_REGISTRIES, Runnable::run)
                    .join();

            long elements = worldgen.registries()
                    .mapToLong(entry -> entry.value().size())
                    .sum();
            System.err.printf(
                    "loaded %d registries (%d elements) against %d tagged registries%n",
                    worldgen.registries().count(), elements, pending.size());
        }
    }

    /// Splitting this out is what gives the wildcard in `registries()` a name
    /// to bind to, so `resolve` and `prepareTagReload` agree on one `T`.
    private static <T> Registry.PendingTags<T> prepareTags(
            Registry<T> registry, TagNetworkSerialization.NetworkPayload payload) {
        return registry.prepareTagReload(payload.resolve(registry));
    }

    /// The dump read back into the payloads a client would have decoded from
    /// `update_tags`. The `.bin` holds each tag's id list back to back, and
    /// `index.json` says which tag each one belongs to and how long it is.
    private static Map<String, TagNetworkSerialization.NetworkPayload> readTagDump(Path dumpDir)
            throws IOException {
        Map<String, TagNetworkSerialization.NetworkPayload> payloads = new LinkedHashMap<>();
        JsonArray index = JsonParser
                .parseString(Files.readString(dumpDir.resolve("index.json"), StandardCharsets.UTF_8))
                .getAsJsonArray();

        for (JsonElement element : index) {
            JsonObject registry = element.getAsJsonObject();
            byte[] blob = Files.readAllBytes(dumpDir.resolve(registry.get("file").getAsString()));
            FriendlyByteBuf buffer = new FriendlyByteBuf(Unpooled.wrappedBuffer(blob));

            Map<Identifier, IntList> tags = new LinkedHashMap<>();
            for (JsonElement tag : registry.getAsJsonArray("tags")) {
                String name = tag.getAsJsonObject().get("name").getAsString();
                tags.put(Identifier.parse(name), buffer.readIntIdList());
            }
            if (buffer.isReadable()) {
                throw new IllegalStateException(
                        registry.get("file").getAsString() + " has "
                                + buffer.readableBytes() + " bytes index.json does not account for");
            }
            payloads.put(
                    registry.get("registry").getAsString(),
                    new TagNetworkSerialization.NetworkPayload(tags));
        }
        return payloads;
    }

    // --- plumbing ---------------------------------------------------------

    private void put(String name, String value) {
        if (fixtures.put(name, value) != null) {
            throw new IllegalStateException("duplicate fixture: " + name);
        }
    }

    private byte[] prepend(byte[] frameBody) {
        return runOutbound(new EmbeddedChannel(new Varint21LengthFieldPrepender()), frameBody);
    }

    private byte[] compress(int threshold, byte[] body) {
        return runOutbound(new EmbeddedChannel(new CompressionEncoder(threshold)), body);
    }

    private String encrypt(byte[] secret, byte[] framed) {
        EmbeddedChannel channel = new EmbeddedChannel(new CipherEncoder(cipher(Cipher.ENCRYPT_MODE, secret)));
        return hex(runOutbound(channel, framed));
    }

    private static Cipher cipher(int mode, byte[] secret) {
        try {
            Cipher cipher = Cipher.getInstance("AES/CFB8/NoPadding");
            SecretKeySpec key = new SecretKeySpec(secret, "AES");
            cipher.init(mode, key, new IvParameterSpec(secret));
            return cipher;
        } catch (Exception e) {
            throw new IllegalStateException(e);
        }
    }

    private static byte[] runOutbound(EmbeddedChannel channel, byte[] input) {
        channel.writeOutbound(Unpooled.wrappedBuffer(input));
        ByteBuf out = Unpooled.buffer();
        ByteBuf part;
        while ((part = channel.readOutbound()) != null) {
            out.writeBytes(part);
            part.release();
        }
        return drain(out);
    }

    private static <T> byte[] writeContainer(PalettedContainer<T> container) {
        FriendlyByteBuf buffer = new FriendlyByteBuf(Unpooled.buffer());
        container.write(buffer);
        return drain(buffer);
    }

    private static byte[] drain(ByteBuf buffer) {
        byte[] bytes = new byte[buffer.readableBytes()];
        buffer.readBytes(bytes);
        return bytes;
    }

    private static byte[] repeat(byte value, int count) {
        byte[] out = new byte[count];
        java.util.Arrays.fill(out, value);
        return out;
    }

    private static byte[] concat(byte[] left, byte[] right) {
        byte[] out = new byte[left.length + right.length];
        System.arraycopy(left, 0, out, 0, left.length);
        System.arraycopy(right, 0, out, left.length, right.length);
        return out;
    }

    private static String hex(byte[] bytes) {
        StringBuilder builder = new StringBuilder(bytes.length * 2);
        for (byte b : bytes) {
            builder.append(String.format("%02x", b));
        }
        return builder.toString();
    }

    private static String fileNameFor(String registryName, String extension) {
        return registryName.replace(':', '.').replace('/', '.') + "." + extension;
    }

    private static String quote(String value) {
        StringBuilder builder = new StringBuilder("\"");
        for (int i = 0; i < value.length(); i++) {
            char c = value.charAt(i);
            switch (c) {
                case '"' -> builder.append("\\\"");
                case '\\' -> builder.append("\\\\");
                default -> builder.append(c);
            }
        }
        return builder.append('"').toString();
    }

    private String toJson() {
        StringBuilder builder = new StringBuilder("{\n");
        boolean first = true;
        for (Map.Entry<String, String> entry : fixtures.entrySet()) {
            if (!first) {
                builder.append(",\n");
            }
            first = false;
            builder.append("  ").append(quote(entry.getKey())).append(": ").append(quote(entry.getValue()));
        }
        return builder.append("\n}\n").toString();
    }

    private VanillaEncoder() {
    }
}

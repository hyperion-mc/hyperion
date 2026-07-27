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
//   registries <dir>    writes the network NBT of every synchronised registry
//                       element, which `nix/generate-rust.py` turns into the
//                       tables under `src/generated`.
//
// Adding a fixture means adding a `put` call below and a matching assertion in
// Rust. Both halves live in version control, which is the point: issue #970 is
// that this used to be retyped from scratch every time somebody needed it.

import com.mojang.serialization.DynamicOps;

import io.netty.buffer.ByteBuf;
import io.netty.buffer.Unpooled;
import io.netty.channel.embedded.EmbeddedChannel;

import java.io.IOException;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.ArrayList;
import java.util.BitSet;
import java.util.EnumMap;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;
import java.util.Optional;

import javax.crypto.Cipher;
import javax.crypto.spec.IvParameterSpec;
import javax.crypto.spec.SecretKeySpec;

import net.minecraft.SharedConstants;
import net.minecraft.core.Holder;
import net.minecraft.core.HolderLookup;
import net.minecraft.core.IdMap;
import net.minecraft.core.registries.Registries;
import net.minecraft.data.registries.VanillaRegistries;
import net.minecraft.nbt.NbtOps;
import net.minecraft.nbt.Tag;
import net.minecraft.network.CipherEncoder;
import net.minecraft.network.CompressionEncoder;
import net.minecraft.network.FriendlyByteBuf;
import net.minecraft.network.Varint21LengthFieldPrepender;
import net.minecraft.network.codec.ByteBufCodecs;
import net.minecraft.network.codec.StreamCodec;
import net.minecraft.resources.RegistryDataLoader;
import net.minecraft.server.Bootstrap;
import net.minecraft.world.level.biome.Biome;
import net.minecraft.world.level.biome.Biomes;
import net.minecraft.world.level.block.Block;
import net.minecraft.world.level.block.Blocks;
import net.minecraft.world.level.block.state.BlockState;
import net.minecraft.world.level.chunk.PalettedContainer;
import net.minecraft.world.level.chunk.Strategy;
import net.minecraft.world.level.levelgen.Heightmap;

public final class VanillaEncoder {
    /// Fixture ordering is part of the output, so a map that keeps insertion
    /// order rather than one that sorts by hash.
    private final Map<String, String> fixtures = new LinkedHashMap<>();

    public static void main(String[] args) throws Exception {
        if (args.length < 2) {
            System.err.println("usage: VanillaEncoder (fixtures <file> | registries <dir>)");
            System.exit(2);
        }

        // Without these the block state registry is empty and every codec
        // below either throws or silently encodes nothing.
        SharedConstants.tryDetectVersion();
        Bootstrap.bootStrap();

        VanillaEncoder encoder = new VanillaEncoder();
        switch (args[0]) {
            case "fixtures" -> encoder.fixtures(Path.of(args[1]));
            case "registries" -> encoder.registries(Path.of(args[1]));
            default -> {
                System.err.println("unknown command: " + args[0]);
                System.exit(2);
            }
        }
    }

    // --- fixtures ---------------------------------------------------------

    private void fixtures(Path out) throws IOException {
        framing();
        palettedContainers();
        heightmaps();
        lightData();
        Files.writeString(out, toJson(), StandardCharsets.UTF_8);
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
    private void palettedContainers() {
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
        // different bits-to-palette table. `Registry.asHolderIdMap` is what
        // supplies the id map in game; a `HolderLookup.Provider` has no such
        // method, so the ids here come from `listElements` order, which is
        // the order a `MappedRegistry` assigns them in.
        HolderLookup.Provider provider = VanillaRegistries.createLookup();
        HolderLookup.RegistryLookup<Biome> biomes = provider.lookupOrThrow(Registries.BIOME);
        List<Holder<Biome>> ordered = biomes.listElements().map(h -> (Holder<Biome>) h).toList();
        IdMap<Holder<Biome>> biomeIds = new ListIdMap<>(ordered);
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

    // --- registry dump ----------------------------------------------------

    /// Writes the network NBT of every synchronised registry element.
    ///
    /// This is `RegistrySynchronization.packRegistry` with an empty set of
    /// client-known packs, which is the case where the server sends contents
    /// rather than a bare name. The bytes are what `ByteBufCodecs.TAG` puts on
    /// the wire: a type byte and then the payload, with no root name.
    private void registries(Path outDir) throws IOException {
        HolderLookup.Provider provider = VanillaRegistries.createLookup();
        // `SynchronizeRegistriesTask` encodes with
        // `registries.createSerializationContext(NbtOps.INSTANCE)`, not with
        // bare `NbtOps`. The difference matters: a `RegistryOps` writes a
        // `HolderSet` as the tag's own name, where plain `NbtOps` tries to
        // dereference the tag and throws.
        DynamicOps<Tag> ops = provider.createSerializationContext(NbtOps.INSTANCE);
        Files.createDirectories(outDir);

        StringBuilder index = new StringBuilder("[\n");
        boolean firstRegistry = true;

        List<String> skipped = new ArrayList<>();
        for (RegistryDataLoader.RegistryData<?> registryData : RegistryDataLoader.SYNCHRONIZED_REGISTRIES) {
            List<String> ids = new ArrayList<>();
            List<byte[]> payloads = new ArrayList<>();
            try {
                if (!dumpRegistry(provider, ops, registryData, ids, payloads)) {
                    continue;
                }
            } catch (RuntimeException e) {
                // Some element codecs dereference a tag, and tags are not
                // bound in a `HolderLookup.Provider` built by
                // `VanillaRegistries.createLookup`: they come from the
                // datapack loader, which needs a resource manager and a
                // server. Those registries are reported rather than dropped,
                // so a consumer sees the gap instead of a short list.
                skipped.add(registryData.key().identifier() + ": " + e);
                System.err.println("skipping " + registryData.key().identifier() + ": " + e);
                continue;
            }

            String registryName = registryData.key().identifier().toString();
            Path file = outDir.resolve(fileNameFor(registryName));
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

        StringBuilder skippedJson = new StringBuilder("[");
        for (int i = 0; i < skipped.size(); i++) {
            if (i > 0) {
                skippedJson.append(", ");
            }
            skippedJson.append(quote(skipped.get(i)));
        }
        skippedJson.append("]\n");
        Files.writeString(outDir.resolve("skipped.json"), skippedJson.toString(), StandardCharsets.UTF_8);
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
            Tag encoded = registryData.elementCodec()
                    .encodeStart(ops, element.value())
                    .getOrThrow(message -> new IllegalStateException(
                            "failed to encode " + element.key() + ": " + message));
            ByteBuf buffer = Unpooled.buffer();
            ByteBufCodecs.TAG.encode(buffer, encoded);
            ids.add(element.key().identifier().toString());
            payloads.add(drain(buffer));
        });
        return true;
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

    private static String fileNameFor(String registryName) {
        return registryName.replace(':', '.').replace('/', '.') + ".nbt";
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

    /// An `IdMap` over a fixed list, which is all `Strategy` asks of one.
    ///
    /// The real one is `MappedRegistry`, reachable only from a
    /// `RegistryAccess`; building one of those means running the datapack
    /// loader. Ids match because both assign them in registration order.
    private record ListIdMap<T>(List<T> values) implements IdMap<T> {
        @Override
        public int getId(T value) {
            return values.indexOf(value);
        }

        @Override
        public T byId(int id) {
            return id >= 0 && id < values.size() ? values.get(id) : null;
        }

        @Override
        public int size() {
            return values.size();
        }

        @Override
        public java.util.Iterator<T> iterator() {
            return values.iterator();
        }
    }
}

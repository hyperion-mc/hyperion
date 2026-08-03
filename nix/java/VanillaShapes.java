// Dumps every block state's collision shape, straight out of the server's own
// code.
//
// This exists because the shapes are not data anywhere else. Mojang's data
// generator (`--reports`) describes a block state as an id and a property map
// and stops there -- 1196 blocks in 26.2 and not one mention of collision --
// because a collision shape is a `VoxelShape` constant compiled into each
// block class, not an entry in a table. The only way to read it is to ask the
// game, which is what this does.
//
// The alternative it replaces was `valence_generated`'s checked-in
// `extracted/blocks.json`. That file is Minecraft 1.20.1: 1003 blocks and
// 24135 states against this jar's 1196 and 32366, so an arrow was clipped
// against 1.20.1's shapes while the client watching it ran 26.2.
//
// Measured, that cost less than it sounds. Of the 24135 states a 1.20.1 world
// can hold, exactly one has different geometry in 26.2, and it is one no world
// contains -- the `shapes_changed_since_1_20_1` test in hyperion's
// `simulation/blocks/translate.rs` names it and fails if that stops being
// true. What this buys is not a fixed collision but a source that moves with
// the jar under a check, and a shape for all 32366 of this version's states
// rather than for the 24135 the old table described.
//
// No `MinecraftServer` subclass here, unlike `VanillaTrace`. A collision shape
// is a pure function of the state: `getCollisionShape` takes a `BlockGetter`
// only so that a block *could* look at its neighbours, and the ones whose
// shape depends on a neighbour encode that dependency in their own properties
// (`fence[north=true]`) rather than by reading the level. So
// `EmptyBlockGetter.INSTANCE` is the honest argument, `Bootstrap.bootStrap()`
// is all the setup needed, and the whole thing runs in seconds without a world.

import com.google.gson.GsonBuilder;
import com.google.gson.JsonArray;
import com.google.gson.JsonObject;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.ArrayList;
import java.util.HashMap;
import java.util.List;
import java.util.Map;
import net.minecraft.SharedConstants;
import net.minecraft.core.BlockPos;
import net.minecraft.server.Bootstrap;
import net.minecraft.world.level.EmptyBlockGetter;
import net.minecraft.world.level.block.Block;
import net.minecraft.world.level.block.state.BlockState;
import net.minecraft.world.phys.AABB;
import net.minecraft.world.phys.shapes.VoxelShape;

public final class VanillaShapes {
    private VanillaShapes() {}

    public static void main(String[] args) throws Exception {
        if (args.length != 1) {
            System.err.println("usage: VanillaShapes <output.json>");
            System.exit(2);
        }

        // Same two lines every harness here opens with. Without them the block
        // registry is empty and every lookup below returns air.
        SharedConstants.tryDetectVersion();
        Bootstrap.bootStrap();

        // One entry per distinct box list, and an index per state. The same
        // compression valence's table uses, and for the same reason: 32366
        // states share a few hundred shapes, since every one of the sixty-odd
        // stone-cut full cubes is the same unit box.
        List<List<AABB>> shapes = new ArrayList<>();
        Map<String, Integer> shapeIndex = new HashMap<>();

        // The registry is indexed by the protocol state id -- the same number
        // `hyperion_minecraft_proto::block_state` computes and the same one the
        // wire carries -- so the table this writes is indexable by it directly
        // with no name round trip.
        int maxId = -1;
        Map<Integer, Integer> stateToShape = new HashMap<>();
        for (BlockState state : Block.BLOCK_STATE_REGISTRY) {
            int id = Block.BLOCK_STATE_REGISTRY.getId(state);
            if (id < 0) {
                throw new IllegalStateException("state " + state + " is not in the registry");
            }
            maxId = Math.max(maxId, id);

            VoxelShape shape = state.getCollisionShape(EmptyBlockGetter.INSTANCE, BlockPos.ZERO);
            List<AABB> boxes = shape.toAabbs();

            String key = key(boxes);
            Integer index = shapeIndex.get(key);
            if (index == null) {
                index = shapes.size();
                shapes.add(boxes);
                shapeIndex.put(key, index);
            }
            stateToShape.put(id, index);
        }

        if (maxId < 0) {
            throw new IllegalStateException("the block state registry is empty");
        }
        // Dense by construction: the registry numbers states contiguously from
        // zero, and a hole would mean a state id that indexes into nothing.
        // Checked rather than assumed, because the failure downstream is an
        // arrow passing through one specific block rather than a crash.
        if (stateToShape.size() != maxId + 1) {
            throw new IllegalStateException(
                    "the state ids are not dense: " + stateToShape.size() + " states with a maximum id of " + maxId);
        }

        JsonArray shapeArray = new JsonArray();
        for (List<AABB> boxes : shapes) {
            JsonArray boxArray = new JsonArray();
            for (AABB box : boxes) {
                JsonArray corners = new JsonArray();
                corners.add(box.minX);
                corners.add(box.minY);
                corners.add(box.minZ);
                corners.add(box.maxX);
                corners.add(box.maxY);
                corners.add(box.maxZ);
                boxArray.add(corners);
            }
            shapeArray.add(boxArray);
        }

        JsonArray perState = new JsonArray();
        for (int id = 0; id <= maxId; id++) {
            perState.add(stateToShape.get(id).intValue());
        }

        JsonObject out = new JsonObject();
        out.addProperty("minecraftVersion", SharedConstants.getCurrentVersion().name());
        out.addProperty("stateCount", maxId + 1);
        out.add("shapes", shapeArray);
        out.add("stateShapes", perState);

        Path output = Path.of(args[0]);
        Files.createDirectories(output.toAbsolutePath().getParent());
        Files.writeString(
                output,
                new GsonBuilder().create().toJson(out) + "\n",
                StandardCharsets.UTF_8);
        System.err.printf(
                "wrote %d states over %d distinct shapes to %s%n", maxId + 1, shapes.size(), output);
    }

    /// A box list's identity, for deduplication.
    ///
    /// The doubles are formatted rather than rounded: two shapes that differ in
    /// the last bit are two shapes, and collapsing them would silently move a
    /// block's surface. `AABB` has no `hashCode` worth relying on across a
    /// list, so this is the key.
    private static String key(List<AABB> boxes) {
        StringBuilder builder = new StringBuilder(boxes.size() * 48);
        for (AABB box : boxes) {
            builder.append(box.minX).append(',')
                    .append(box.minY).append(',')
                    .append(box.minZ).append(',')
                    .append(box.maxX).append(',')
                    .append(box.maxY).append(',')
                    .append(box.maxZ).append(';');
        }
        return builder.toString();
    }
}

package dev.hyperion.pilot;

import com.google.gson.JsonArray;
import com.google.gson.JsonObject;
import java.io.File;
import java.util.concurrent.CompletableFuture;
import net.minecraft.client.Minecraft;
import net.minecraft.client.Screenshot;
import net.minecraft.core.registries.BuiltInRegistries;
import net.minecraft.network.protocol.game.ServerboundSetCarriedItemPacket;
import net.minecraft.world.entity.Entity;
import net.minecraft.world.entity.projectile.arrow.AbstractArrow;
import net.minecraft.world.item.ItemStack;
import net.minecraft.world.phys.Vec3;

/**
 * Everything that touches game state. Each method hops onto the client thread
 * via {@link Minecraft#execute} and, where the caller needs an answer, returns a
 * CompletableFuture the socket thread can block on. The state readback is
 * deliberately rich: an agent's whole feedback loop is drive -> read state /
 * screenshot -> decide, and arrows-in-flight plus bow-draw ticks are exactly
 * what the two bugs this mod exists to test turn on.
 */
public final class ClientActions {
    private ClientActions() {}

    private static Minecraft mc() {
        return Minecraft.getInstance();
    }

    /** Grab the framebuffer to a PNG under ~/.hyperion-pilot/screenshots and return its path. */
    public static CompletableFuture<String> screenshot() {
        Minecraft mc = mc();
        CompletableFuture<String> fut = new CompletableFuture<>();
        mc.execute(() -> {
            try {
                String name = "shot-" + System.currentTimeMillis() + ".png";
                File workDir = PilotHome.ROOT.toFile(); // Screenshot.grab writes into <workDir>/screenshots
                Screenshot.grab(workDir, name, mc.gameRenderer.mainRenderTarget(), 1,
                        component -> fut.complete(PilotHome.SCREENSHOT_DIR.resolve(name).toString()));
            } catch (Throwable t) {
                fut.completeExceptionally(t);
            }
        });
        return fut;
    }

    public static void setSlot(int slot) {
        Minecraft mc = mc();
        mc.execute(() -> {
            if (mc.player == null) return;
            int s = Math.max(0, Math.min(8, slot));
            mc.player.getInventory().setSelectedSlot(s);
            mc.player.connection.getConnection().send(new ServerboundSetCarriedItemPacket(s));
        });
    }

    public static void drop(boolean all) {
        Minecraft mc = mc();
        mc.execute(() -> {
            if (mc.player != null) mc.player.drop(all);
        });
    }

    public static void chat(String message) {
        Minecraft mc = mc();
        mc.execute(() -> {
            if (mc.player != null) mc.player.connection.sendChat(message);
        });
    }

    public static void command(String command) {
        Minecraft mc = mc();
        mc.execute(() -> {
            if (mc.player != null) {
                String c = command.startsWith("/") ? command.substring(1) : command;
                mc.player.connection.sendCommand(c);
            }
        });
    }

    /** A full snapshot of the world from the character's point of view. */
    public static CompletableFuture<JsonObject> state(double radius) {
        Minecraft mc = mc();
        CompletableFuture<JsonObject> fut = new CompletableFuture<>();
        mc.execute(() -> {
            try {
                fut.complete(gather(mc, radius));
            } catch (Throwable t) {
                fut.completeExceptionally(t);
            }
        });
        return fut;
    }

    private static JsonObject gather(Minecraft mc, double radius) {
        JsonObject o = new JsonObject();
        var player = mc.player;
        o.addProperty("hasPlayer", player != null);
        o.addProperty("hasLevel", mc.level != null);
        if (player == null) return o;

        JsonObject self = new JsonObject();
        self.addProperty("name", player.getName().getString());
        self.addProperty("uuid", player.getUUID().toString());
        self.add("pos", vec(player.getX(), player.getY(), player.getZ()));
        self.add("velocity", vecOf(player.getDeltaMovement()));
        self.addProperty("yaw", player.getYRot());
        self.addProperty("pitch", player.getXRot());
        self.addProperty("onGround", player.onGround());
        self.addProperty("health", player.getHealth());
        self.addProperty("food", player.getFoodData().getFoodLevel());
        self.addProperty("sprinting", player.isSprinting());
        self.addProperty("sneaking", player.isShiftKeyDown());
        self.addProperty("usingItem", player.isUsingItem());
        self.addProperty("useItemRemainingTicks", player.getUseItemRemainingTicks());
        self.addProperty("selectedSlot", player.getInventory().getSelectedSlot());
        self.add("mainHand", itemJson(player.getMainHandItem()));
        self.add("offHand", itemJson(player.getOffhandItem()));
        o.add("player", self);

        JsonArray entities = new JsonArray();
        JsonArray arrows = new JsonArray();
        if (mc.level != null) {
            Vec3 eye = player.position();
            double r2 = radius * radius;
            for (Entity e : mc.level.entitiesForRendering()) {
                if (e == player) continue;
                if (e.position().distanceToSqr(eye) > r2) continue;
                JsonObject ej = entityJson(e);
                entities.add(ej);
                if (e instanceof AbstractArrow) arrows.add(ej);
                if (entities.size() >= 128) break;
            }
        }
        o.add("nearbyEntities", entities);
        o.add("arrows", arrows);
        return o;
    }

    private static JsonObject entityJson(Entity e) {
        JsonObject j = new JsonObject();
        j.addProperty("id", e.getId());
        j.addProperty("type", BuiltInRegistries.ENTITY_TYPE.getKey(e.getType()).toString());
        j.add("pos", vec(e.getX(), e.getY(), e.getZ()));
        j.add("velocity", vecOf(e.getDeltaMovement()));
        j.addProperty("yaw", e.getYRot());
        j.addProperty("pitch", e.getXRot());
        j.addProperty("onGround", e.onGround());
        return j;
    }

    private static JsonObject itemJson(ItemStack stack) {
        JsonObject j = new JsonObject();
        if (stack.isEmpty()) {
            j.addProperty("empty", true);
            return j;
        }
        j.addProperty("empty", false);
        j.addProperty("id", BuiltInRegistries.ITEM.getKey(stack.getItem()).toString());
        j.addProperty("count", stack.getCount());
        return j;
    }

    private static JsonObject vec(double x, double y, double z) {
        JsonObject j = new JsonObject();
        j.addProperty("x", x);
        j.addProperty("y", y);
        j.addProperty("z", z);
        return j;
    }

    private static JsonObject vecOf(Vec3 v) {
        return vec(v.x, v.y, v.z);
    }
}

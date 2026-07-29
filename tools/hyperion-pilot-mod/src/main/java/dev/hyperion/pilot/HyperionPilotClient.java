package dev.hyperion.pilot;

import com.mojang.blaze3d.platform.InputConstants;
import dev.hyperion.pilot.control.ControlServer;
import dev.hyperion.pilot.mixin.KeyMappingAccessor;
import java.util.concurrent.atomic.AtomicInteger;
import net.fabricmc.api.ClientModInitializer;
import net.fabricmc.fabric.api.client.event.lifecycle.v1.ClientTickEvents;
import net.fabricmc.fabric.api.client.keymapping.v1.KeyMappingHelper;
import net.fabricmc.fabric.api.client.rendering.v1.hud.HudElementRegistry;
import net.fabricmc.fabric.api.client.rendering.v1.hud.VanillaHudElements;
import net.minecraft.client.KeyMapping;
import net.minecraft.client.Minecraft;
import net.minecraft.client.Options;
import net.minecraft.resources.Identifier;
import org.lwjgl.glfw.GLFW;
import org.slf4j.Logger;
import org.slf4j.LoggerFactory;

/**
 * Wires the three pieces together: the packet log (via the Connection mixin,
 * which touches PacketLog directly), the in-client overlay with a toggle
 * keybind, and the control socket. The per-tick handler is the heart of the
 * pilot: it folds held mouse buttons onto the real key mappings, fires queued
 * single clicks, and turns the camera toward the commanded look target.
 */
public final class HyperionPilotClient implements ClientModInitializer {
    private static final Logger LOGGER = LoggerFactory.getLogger("hyperion-pilot");

    public static final PilotState STATE = new PilotState();

    private static final AtomicInteger PENDING_ATTACK = new AtomicInteger();
    private static final AtomicInteger PENDING_USE = new AtomicInteger();

    private KeyMapping overlayKey;

    public static void pulseAttack() { PENDING_ATTACK.incrementAndGet(); }
    public static void pulseUse() { PENDING_USE.incrementAndGet(); }

    @Override
    public void onInitializeClient() {
        // Force the packet log open now so captures from the netty thread have a sink.
        PacketLog.get();

        overlayKey = KeyMappingHelper.registerKeyMapping(new KeyMapping(
                "key.hyperion-pilot.overlay",
                InputConstants.Type.KEYSYM,
                GLFW.GLFW_KEY_F6,
                KeyMapping.Category.MISC));

        HudElementRegistry.attachElementBefore(
                VanillaHudElements.CHAT,
                Identifier.fromNamespaceAndPath("hyperion-pilot", "packet-overlay"),
                (graphics, deltaTracker) -> PacketOverlay.render(graphics));

        ClientTickEvents.END_CLIENT_TICK.register(this::onClientTick);

        new ControlServer(STATE).start();
        LOGGER.info("hyperion-pilot: initialised. Control home at {}", PilotHome.ROOT);
    }

    private void onClientTick(Minecraft mc) {
        while (overlayKey.consumeClick()) {
            PacketOverlay.toggle();
        }

        STATE.runQueued();

        Options opt = mc.options;
        if (opt != null) {
            // Held mouse buttons: hold use == drawing a bow, release == firing it.
            ((KeyMappingAccessor) (Object) opt.keyUse).hyperionPilot$setIsDown(STATE.use);
            ((KeyMappingAccessor) (Object) opt.keyAttack).hyperionPilot$setIsDown(STATE.attack);

            int a = PENDING_ATTACK.getAndSet(0);
            if (a > 0) ((KeyMappingAccessor) (Object) opt.keyAttack).hyperionPilot$setClickCount(a);
            int u = PENDING_USE.getAndSet(0);
            if (u > 0) ((KeyMappingAccessor) (Object) opt.keyUse).hyperionPilot$setClickCount(u);
        }

        if (STATE.lookActive && mc.player != null) {
            float yaw = mc.player.getYRot();
            float pitch = mc.player.getXRot();
            float step = STATE.lookStepDeg;
            float ny = approach(yaw, STATE.yawTarget, step);
            float np = approach(pitch, STATE.pitchTarget, step);
            mc.player.setYRot(ny);
            mc.player.setYHeadRot(ny);
            mc.player.setXRot(np);
            if (ny == STATE.yawTarget && np == STATE.pitchTarget) {
                STATE.lookActive = false;
            }
        }
    }

    /** Move current toward target by at most step degrees, taking the short way round. */
    private static float approach(float current, float target, float step) {
        float delta = target - current;
        delta %= 360f;
        if (delta > 180f) delta -= 360f;
        if (delta < -180f) delta += 360f;
        if (Math.abs(delta) <= step) return target;
        return current + Math.signum(delta) * step;
    }
}

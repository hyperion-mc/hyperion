package dev.hyperion.pilot.mixin;

import dev.hyperion.pilot.HyperionPilotClient;
import dev.hyperion.pilot.PilotState;
import net.minecraft.client.player.ClientInput;
import net.minecraft.client.player.KeyboardInput;
import net.minecraft.world.entity.player.Input;
import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.injection.At;
import org.spongepowered.asm.mixin.injection.Inject;
import org.spongepowered.asm.mixin.injection.callback.CallbackInfo;

/**
 * After vanilla rebuilds the movement input from the real keyboard, OR in the
 * agent's held movement. Folding it in here (rather than forcing the key
 * mappings) means the packet the client sends each tick,
 * ServerboundPlayerInputPacket(input.keyPresses), already carries the agent's
 * intent, and the operator can still co-drive with their own keys.
 */
@Mixin(KeyboardInput.class)
public abstract class KeyboardInputMixin {

    @Inject(method = "tick", at = @At("TAIL"))
    private void hyperionPilot$applyHeldMovement(CallbackInfo ci) {
        PilotState p = HyperionPilotClient.STATE;
        if (p == null) return;
        ClientInput self = (ClientInput) (Object) this;
        Input v = self.keyPresses;
        Input merged = new Input(
                v.forward() || p.forward,
                v.backward() || p.back,
                v.left() || p.left,
                v.right() || p.right,
                v.jump() || p.jump,
                v.shift() || p.sneak,
                v.sprint() || p.sprint);
        self.keyPresses = merged;
    }
}

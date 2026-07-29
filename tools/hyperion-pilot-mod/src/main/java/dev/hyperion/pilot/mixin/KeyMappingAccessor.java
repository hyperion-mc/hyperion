package dev.hyperion.pilot.mixin;

import net.minecraft.client.KeyMapping;
import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.gen.Accessor;

/**
 * Direct access to KeyMapping's private held/click state, bypassing
 * ToggleKeyMapping.setDown() whose toggle behaviour would fight a held button.
 * Used to hold right-click (use) for a bow draw and left-click (attack).
 */
@Mixin(KeyMapping.class)
public interface KeyMappingAccessor {
    @Accessor("isDown")
    void hyperionPilot$setIsDown(boolean value);

    @Accessor("clickCount")
    void hyperionPilot$setClickCount(int value);
}

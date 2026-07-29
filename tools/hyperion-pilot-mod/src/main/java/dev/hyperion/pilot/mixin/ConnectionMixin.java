package dev.hyperion.pilot.mixin;

import dev.hyperion.pilot.PacketLog;
import io.netty.channel.ChannelFutureListener;
import io.netty.channel.ChannelHandlerContext;
import net.minecraft.network.Connection;
import net.minecraft.network.protocol.Packet;
import org.jspecify.annotations.Nullable;
import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.injection.At;
import org.spongepowered.asm.mixin.injection.Inject;
import org.spongepowered.asm.mixin.injection.callback.CallbackInfo;

/**
 * Taps both ends of the client's single {@link Connection}. channelRead0 is the
 * one place every inbound (clientbound) packet passes through after decode;
 * doSendPacket is the one place every outbound (serverbound) packet passes
 * through after queueing, so together they see everything with no duplicates.
 */
@Mixin(Connection.class)
public class ConnectionMixin {

    @Inject(method = "channelRead0", at = @At("HEAD"))
    private void hyperionPilot$inbound(ChannelHandlerContext ctx, Packet<?> packet, CallbackInfo ci) {
        PacketLog.get().capture(true, packet);
    }

    @Inject(method = "doSendPacket", at = @At("HEAD"))
    private void hyperionPilot$outbound(Packet<?> packet, @Nullable ChannelFutureListener listener,
                                        boolean flush, CallbackInfo ci) {
        PacketLog.get().capture(false, packet);
    }
}

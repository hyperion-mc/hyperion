package dev.hyperion.pilot;

import java.util.List;
import net.minecraft.client.Minecraft;
import net.minecraft.client.gui.Font;
import net.minecraft.client.gui.GuiGraphicsExtractor;

/**
 * A lightweight scrolling list of the most recent packets, drawn top-left.
 * Toggled by a keybind; off by default so it never gets in the operator's way.
 */
public final class PacketOverlay {
    private static volatile boolean visible = false;
    private static final int LINES = 20;

    private PacketOverlay() {}

    public static boolean isVisible() { return visible; }
    public static void toggle() { visible = !visible; }

    public static void render(GuiGraphicsExtractor g) {
        if (!visible) return;
        Minecraft mc = Minecraft.getInstance();
        Font font = mc.font;
        List<String> lines = PacketLog.get().recent(LINES);

        int x = 4;
        int y = 4;
        int lineHeight = 10;
        int width = 360;
        int height = (lines.size() + 1) * lineHeight + 6;
        g.fill(x - 2, y - 2, x + width, y + height, 0xA0000000);

        String header = "hyperion-pilot  packets:" + PacketLog.get().writtenCount()
                + " dropped:" + PacketLog.get().droppedCount();
        g.text(font, header, x, y, 0xFF55FF55);
        y += lineHeight + 2;
        for (String line : lines) {
            String s = line.length() > 80 ? line.substring(0, 80) : line;
            g.text(font, s, x, y, 0xFFDDDDDD);
            y += lineHeight;
        }
    }
}

package dev.hyperion.pilot;

import java.io.IOException;
import java.nio.file.Files;
import java.nio.file.Path;
import java.nio.file.Paths;

/**
 * The one on-disk location everything in this mod uses, so an external agent
 * has a single stable place to look. Fixed at {@code ~/.hyperion-pilot} rather
 * than the game directory, because the game directory depends on how the
 * launcher was invoked and an agent driving the client should not have to guess
 * it.
 *
 * <pre>
 *   ~/.hyperion-pilot/
 *     control.sock            unix domain socket the control RPC listens on
 *     packets/                rotating JSONL packet logs
 *     screenshots/            PNGs written by the screenshot command
 * </pre>
 */
public final class PilotHome {
    private PilotHome() {}

    public static final Path ROOT =
            Paths.get(System.getProperty("user.home"), ".hyperion-pilot");
    public static final Path SOCKET = ROOT.resolve("control.sock");
    public static final Path PACKET_DIR = ROOT.resolve("packets");
    public static final Path SCREENSHOT_DIR = ROOT.resolve("screenshots");

    /** Create the directory tree. Safe to call repeatedly. */
    public static void ensure() throws IOException {
        Files.createDirectories(PACKET_DIR);
        Files.createDirectories(SCREENSHOT_DIR);
    }
}

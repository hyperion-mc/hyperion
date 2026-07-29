package dev.hyperion.pilot.control;

import com.google.gson.Gson;
import com.google.gson.JsonObject;
import com.google.gson.JsonParser;
import dev.hyperion.pilot.PilotHome;
import dev.hyperion.pilot.PilotState;
import java.io.BufferedReader;
import java.io.BufferedWriter;
import java.io.IOException;
import java.io.InputStreamReader;
import java.io.OutputStreamWriter;
import java.net.InetSocketAddress;
import java.net.StandardProtocolFamily;
import java.net.UnixDomainSocketAddress;
import java.nio.channels.Channels;
import java.nio.channels.ServerSocketChannel;
import java.nio.channels.SocketChannel;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import org.slf4j.Logger;
import org.slf4j.LoggerFactory;

/**
 * A newline-delimited JSON RPC an agent connects to in order to pilot the live
 * client. One request object per line, one response object per line.
 *
 * Primary transport is a unix domain socket at ~/.hyperion-pilot/control.sock
 * (Java 25 supports these natively, including on macOS). If binding it fails we
 * fall back to loopback TCP on port {@link #TCP_FALLBACK_PORT}. Either way the
 * chosen endpoint is written to ~/.hyperion-pilot/endpoint.txt so a driver can
 * discover it without guessing.
 */
public final class ControlServer {
    private static final Logger LOGGER = LoggerFactory.getLogger("hyperion-pilot");
    private static final Gson GSON = new Gson();
    private static final int TCP_FALLBACK_PORT = 25599;

    private final PilotState state;
    private final CommandDispatch dispatch;
    private volatile boolean running = true;

    public ControlServer(PilotState state) {
        this.state = state;
        this.dispatch = new CommandDispatch(state);
    }

    public void start() {
        Thread t = new Thread(this::run, "hyperion-pilot-control-accept");
        t.setDaemon(true);
        t.start();
    }

    private void run() {
        try {
            PilotHome.ensure();
        } catch (IOException e) {
            LOGGER.error("hyperion-pilot: cannot create home dir", e);
            return;
        }
        ServerSocketChannel server = tryBindUnix();
        String endpoint;
        if (server != null) {
            endpoint = "unix:" + PilotHome.SOCKET;
        } else {
            server = tryBindTcp();
            if (server == null) {
                LOGGER.error("hyperion-pilot: could not bind any control endpoint");
                return;
            }
            endpoint = "tcp:127.0.0.1:" + TCP_FALLBACK_PORT;
        }
        writeEndpoint(endpoint);
        LOGGER.info("hyperion-pilot: control endpoint listening on {}", endpoint);

        try (ServerSocketChannel s = server) {
            while (running) {
                SocketChannel client = s.accept();
                Thread ct = new Thread(() -> handle(client), "hyperion-pilot-control-conn");
                ct.setDaemon(true);
                ct.start();
            }
        } catch (IOException e) {
            if (running) LOGGER.warn("hyperion-pilot: accept loop ended", e);
        }
    }

    private ServerSocketChannel tryBindUnix() {
        try {
            Files.deleteIfExists(PilotHome.SOCKET);
            ServerSocketChannel server = ServerSocketChannel.open(StandardProtocolFamily.UNIX);
            server.bind(UnixDomainSocketAddress.of(PilotHome.SOCKET));
            return server;
        } catch (Throwable t) {
            LOGGER.warn("hyperion-pilot: unix socket bind failed, will try loopback TCP: {}", t.toString());
            return null;
        }
    }

    private ServerSocketChannel tryBindTcp() {
        try {
            ServerSocketChannel server = ServerSocketChannel.open(StandardProtocolFamily.INET);
            server.bind(new InetSocketAddress("127.0.0.1", TCP_FALLBACK_PORT));
            return server;
        } catch (Throwable t) {
            LOGGER.error("hyperion-pilot: loopback TCP bind failed", t);
            return null;
        }
    }

    private void writeEndpoint(String endpoint) {
        try {
            Files.writeString(PilotHome.ROOT.resolve("endpoint.txt"), endpoint + "\n");
        } catch (IOException e) {
            LOGGER.warn("hyperion-pilot: could not write endpoint file", e);
        }
    }

    private void handle(SocketChannel client) {
        try (SocketChannel c = client;
             BufferedReader in = new BufferedReader(new InputStreamReader(
                     Channels.newInputStream(c), StandardCharsets.UTF_8));
             BufferedWriter out = new BufferedWriter(new OutputStreamWriter(
                     Channels.newOutputStream(c), StandardCharsets.UTF_8))) {
            String line;
            while ((line = in.readLine()) != null) {
                if (line.isBlank()) continue;
                JsonObject response;
                try {
                    JsonObject request = JsonParser.parseString(line).getAsJsonObject();
                    response = dispatch.handle(request);
                } catch (Throwable t) {
                    response = new JsonObject();
                    response.addProperty("ok", false);
                    response.addProperty("error", String.valueOf(t.getMessage()));
                }
                out.write(GSON.toJson(response));
                out.write('\n');
                out.flush();
            }
        } catch (IOException e) {
            // client hung up; nothing to do
        }
    }

    public void stop() {
        running = false;
        state.releaseAll();
    }
}

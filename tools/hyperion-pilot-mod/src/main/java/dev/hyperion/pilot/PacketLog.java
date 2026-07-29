package dev.hyperion.pilot;

import com.google.gson.Gson;
import com.google.gson.GsonBuilder;
import java.io.BufferedWriter;
import java.io.IOException;
import java.io.Writer;
import java.lang.reflect.RecordComponent;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.nio.file.StandardOpenOption;
import java.time.Instant;
import java.time.ZoneId;
import java.time.format.DateTimeFormatter;
import java.util.ArrayDeque;
import java.util.Deque;
import java.util.LinkedHashMap;
import java.util.Map;
import java.util.concurrent.ArrayBlockingQueue;
import java.util.concurrent.BlockingQueue;
import java.util.concurrent.atomic.AtomicLong;
import org.slf4j.Logger;
import org.slf4j.LoggerFactory;

/**
 * Every packet the client sends or receives, written to a rotating JSONL file
 * and mirrored into a small ring buffer for the in-client overlay.
 *
 * The netty event loop only pays for a queue offer: capture is minimal (class,
 * direction, nanotime) and the reflective field decode plus disk write happen
 * on a single background thread. If the queue is full we drop and count, so a
 * chunk-heavy join can never stall the network thread.
 *
 * Packet names are normalised to match tools/client-26.2.py PLAY_NAMES: the
 * simple class name with a leading Clientbound/Serverbound and a trailing
 * Packet stripped, so ClientboundAddEntityPacket logs as "AddEntity".
 */
public final class PacketLog {
    private static final Logger LOGGER = LoggerFactory.getLogger("hyperion-pilot");
    private static final Gson GSON = new GsonBuilder().disableHtmlEscaping().create();
    private static final DateTimeFormatter STAMP =
            DateTimeFormatter.ofPattern("yyyyMMdd-HHmmss").withZone(ZoneId.systemDefault());

    /** One captured packet, before decode. */
    private record Entry(long epochMillis, boolean inbound, Object packet) {}

    private static final long ROTATE_BYTES = 64L * 1024 * 1024;
    private static final int QUEUE_CAP = 8192;
    private static final int RING_CAP = 200;

    private static PacketLog instance;

    private final BlockingQueue<Entry> queue = new ArrayBlockingQueue<>(QUEUE_CAP);
    private final Deque<String> ring = new ArrayDeque<>(RING_CAP);
    private final Object ringLock = new Object();
    private final AtomicLong dropped = new AtomicLong();
    private final AtomicLong written = new AtomicLong();

    private volatile boolean enabled = true;
    private volatile boolean logInbound = true;
    private volatile boolean logOutbound = true;

    private Writer writer;
    private Path currentPath;
    private long bytesInCurrent;
    private volatile boolean rotateRequested;

    private PacketLog() {}

    public static synchronized PacketLog get() {
        if (instance == null) {
            instance = new PacketLog();
            instance.start();
        }
        return instance;
    }

    private void start() {
        try {
            PilotHome.ensure();
            openNewFile();
        } catch (IOException e) {
            LOGGER.error("hyperion-pilot: could not open packet log", e);
        }
        Thread t = new Thread(this::drainLoop, "hyperion-pilot-packetlog");
        t.setDaemon(true);
        t.start();
    }

    /** Called from the netty thread. Never blocks. */
    public void capture(boolean inbound, Object packet) {
        if (!enabled) return;
        if (inbound ? !logInbound : !logOutbound) return;
        if (!queue.offer(new Entry(System.currentTimeMillis(), inbound, packet))) {
            dropped.incrementAndGet();
        }
    }

    private void drainLoop() {
        while (true) {
            try {
                Entry e = queue.take();
                if (rotateRequested) {
                    rotateRequested = false;
                    openNewFile();
                }
                writeEntry(e);
            } catch (InterruptedException ie) {
                Thread.currentThread().interrupt();
                return;
            } catch (Exception ex) {
                LOGGER.warn("hyperion-pilot: packet log write failed", ex);
            }
        }
    }

    private void writeEntry(Entry e) throws IOException {
        String name = normalise(e.packet.getClass().getSimpleName());
        Map<String, Object> record = new LinkedHashMap<>();
        record.put("t", e.epochMillis);
        record.put("dir", e.inbound ? "in" : "out");
        record.put("name", name);
        record.put("class", e.packet.getClass().getName());
        record.put("fields", decode(e.packet));
        String line = GSON.toJson(record);

        if (writer != null) {
            writer.write(line);
            writer.write('\n');
            writer.flush();
            bytesInCurrent += line.length() + 1;
            if (bytesInCurrent >= ROTATE_BYTES) openNewFile();
        }
        written.incrementAndGet();

        String summary = (e.inbound ? "▼ " : "▲ ") + name + " " + summarise(record.get("fields"));
        synchronized (ringLock) {
            if (ring.size() >= RING_CAP) ring.removeFirst();
            ring.addLast(summary);
        }
    }

    /** Shallow reflective decode: record components become a field map. */
    private static Object decode(Object packet) {
        Class<?> c = packet.getClass();
        if (!c.isRecord()) return Map.of();
        Map<String, Object> out = new LinkedHashMap<>();
        for (RecordComponent rc : c.getRecordComponents()) {
            try {
                Object v = rc.getAccessor().invoke(packet);
                out.put(rc.getName(), stringifyShallow(v));
            } catch (Throwable t) {
                out.put(rc.getName(), "<err>");
            }
        }
        return out;
    }

    private static Object stringifyShallow(Object v) {
        if (v == null) return null;
        if (v instanceof Number || v instanceof Boolean || v instanceof String) return v;
        if (v instanceof Enum<?> en) return en.name();
        String s;
        try {
            s = String.valueOf(v);
        } catch (Throwable t) {
            s = v.getClass().getSimpleName();
        }
        return s.length() > 200 ? s.substring(0, 200) + "…" : s;
    }

    @SuppressWarnings("unchecked")
    private static String summarise(Object fields) {
        if (!(fields instanceof Map)) return "";
        Map<String, Object> m = (Map<String, Object>) fields;
        StringBuilder sb = new StringBuilder();
        int n = 0;
        for (Map.Entry<String, Object> en : m.entrySet()) {
            if (n++ >= 4) break;
            if (sb.length() > 0) sb.append(' ');
            String val = String.valueOf(en.getValue());
            if (val.length() > 24) val = val.substring(0, 24) + "…";
            sb.append(en.getKey()).append('=').append(val);
        }
        return sb.toString();
    }

    private static String normalise(String simple) {
        String s = simple;
        if (s.startsWith("Clientbound")) s = s.substring("Clientbound".length());
        else if (s.startsWith("Serverbound")) s = s.substring("Serverbound".length());
        if (s.endsWith("Packet")) s = s.substring(0, s.length() - "Packet".length());
        return s;
    }

    private void openNewFile() throws IOException {
        if (writer != null) {
            try { writer.close(); } catch (IOException ignored) {}
        }
        PilotHome.ensure();
        currentPath = PilotHome.PACKET_DIR.resolve("packets-" + STAMP.format(Instant.now()) + ".jsonl");
        writer = new BufferedWriter(new java.io.OutputStreamWriter(
                Files.newOutputStream(currentPath, StandardOpenOption.CREATE, StandardOpenOption.APPEND),
                StandardCharsets.UTF_8));
        bytesInCurrent = 0;
        LOGGER.info("hyperion-pilot: packet log -> {}", currentPath);
    }

    /** Force a fresh file. Returns the new path once the worker rotates. */
    public void requestRotate() {
        rotateRequested = true;
    }

    public java.util.List<String> recent(int max) {
        synchronized (ringLock) {
            int n = Math.min(max, ring.size());
            java.util.List<String> out = new java.util.ArrayList<>(n);
            var it = ring.descendingIterator();
            while (it.hasNext() && out.size() < n) out.add(it.next());
            java.util.Collections.reverse(out);
            return out;
        }
    }

    public Path currentPath() { return currentPath; }
    public long droppedCount() { return dropped.get(); }
    public long writtenCount() { return written.get(); }
    public boolean isEnabled() { return enabled; }
    public void setEnabled(boolean v) { enabled = v; }
    public void setDirections(boolean in, boolean out) { logInbound = in; logOutbound = out; }
}

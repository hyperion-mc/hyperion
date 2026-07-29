package dev.hyperion.pilot;

import java.util.Queue;
import java.util.concurrent.ConcurrentLinkedQueue;

/**
 * The commanded state of the character, held between ticks. The control socket
 * writes these fields from its own threads; the client tick reads them and the
 * KeyboardInput mixin folds the movement half into the vanilla input record.
 *
 * Everything here is a plain volatile because a held input is last-writer-wins:
 * an agent that says "walk forward" and later "stop" just flips a boolean, and a
 * torn read one tick early is invisible at 20 tps.
 */
public final class PilotState {
    // Held movement (folded into net.minecraft...Input by KeyboardInputMixin).
    public volatile boolean forward;
    public volatile boolean back;
    public volatile boolean left;
    public volatile boolean right;
    public volatile boolean jump;
    public volatile boolean sneak;
    public volatile boolean sprint;

    // Held mouse buttons (applied to Options.keyUse / keyAttack each tick).
    // use == holding right-click: this is how a bow is drawn and released.
    public volatile boolean use;
    public volatile boolean attack;

    /**
     * Look control. When a target is set the client tick turns toward it at
     * most {@link #lookStepDeg} per tick; set the step huge for an instant snap.
     * yawTarget is absolute degrees, pitchTarget is absolute degrees (-90 up,
     * +90 down), matching Minecraft's rotation convention.
     */
    public volatile boolean lookActive;
    public volatile float yawTarget;
    public volatile float pitchTarget;
    public volatile float lookStepDeg = 30f;

    /** One-shot actions that must run on the client thread (screenshots, drop, slot, chat). */
    private final Queue<Runnable> mainThreadActions = new ConcurrentLinkedQueue<>();

    public void submit(Runnable r) {
        mainThreadActions.add(r);
    }

    /** Drain and run queued actions. Called on the client thread only. */
    public void runQueued() {
        Runnable r;
        while ((r = mainThreadActions.poll()) != null) {
            try {
                r.run();
            } catch (Throwable t) {
                org.slf4j.LoggerFactory.getLogger("hyperion-pilot")
                        .warn("hyperion-pilot: queued action failed", t);
            }
        }
    }

    /** Release every held input. Used by the "stop" command and on disconnect. */
    public void releaseAll() {
        forward = back = left = right = jump = sneak = sprint = false;
        use = attack = false;
        lookActive = false;
    }
}

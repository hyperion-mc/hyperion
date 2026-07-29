package dev.hyperion.pilot.control;

import com.google.gson.JsonArray;
import com.google.gson.JsonObject;
import dev.hyperion.pilot.ClientActions;
import dev.hyperion.pilot.PacketLog;
import dev.hyperion.pilot.PilotState;
import java.util.concurrent.TimeUnit;

/**
 * The command schema. One request object with a "cmd" field in, one response
 * object out. Held inputs return immediately; game-state actions run on the
 * client thread and this blocks briefly for their result.
 *
 * Held movement/mouse (persist until changed or "stop"):
 *   {"cmd":"hold","forward":true,"sprint":true}          any subset of
 *       forward,back,left,right,jump,sneak,sprint,use,attack
 *   {"cmd":"stop"}                                        release everything
 * Look (absolute degrees; "instant" snaps, else turns "step" deg/tick):
 *   {"cmd":"look","yaw":90,"pitch":0}
 *   {"cmd":"look","dyaw":15,"dpitch":-5}                  relative
 *   {"cmd":"look","yaw":90,"instant":true}
 * One-shot:
 *   {"cmd":"attack"}   {"cmd":"use"}                      single click
 *   {"cmd":"slot","index":0}                              select hotbar 0..8
 *   {"cmd":"drop","all":false}
 *   {"cmd":"chat","message":"hi"}   {"cmd":"command","command":"gamemode creative"}
 * Feedback:
 *   {"cmd":"screenshot"}            -> {"ok":true,"path":"..."}
 *   {"cmd":"state","radius":32}     -> rich world snapshot
 *   {"cmd":"recent","n":50}         -> recent packet summaries
 * Packet log:
 *   {"cmd":"rotate"}               force a fresh JSONL file
 *   {"cmd":"log","enabled":true,"inbound":true,"outbound":true}
 *   {"cmd":"ping"}
 */
public final class CommandDispatch {
    private final PilotState state;

    public CommandDispatch(PilotState state) {
        this.state = state;
    }

    public JsonObject handle(JsonObject req) throws Exception {
        String cmd = req.has("cmd") ? req.get("cmd").getAsString() : "";
        JsonObject res = new JsonObject();
        switch (cmd) {
            case "ping" -> {
                res.addProperty("ok", true);
                res.addProperty("pong", true);
            }
            case "hold" -> {
                if (req.has("forward")) state.forward = req.get("forward").getAsBoolean();
                if (req.has("back")) state.back = req.get("back").getAsBoolean();
                if (req.has("left")) state.left = req.get("left").getAsBoolean();
                if (req.has("right")) state.right = req.get("right").getAsBoolean();
                if (req.has("jump")) state.jump = req.get("jump").getAsBoolean();
                if (req.has("sneak")) state.sneak = req.get("sneak").getAsBoolean();
                if (req.has("sprint")) state.sprint = req.get("sprint").getAsBoolean();
                if (req.has("use")) state.use = req.get("use").getAsBoolean();
                if (req.has("attack")) state.attack = req.get("attack").getAsBoolean();
                res.addProperty("ok", true);
            }
            case "stop" -> {
                state.releaseAll();
                res.addProperty("ok", true);
            }
            case "look" -> {
                if (req.has("step")) state.lookStepDeg = req.get("step").getAsFloat();
                boolean instant = req.has("instant") && req.get("instant").getAsBoolean();
                if (req.has("dyaw") || req.has("dpitch")) {
                    // relative: resolve against current rotation on the client thread
                    float dyaw = req.has("dyaw") ? req.get("dyaw").getAsFloat() : 0f;
                    float dpitch = req.has("dpitch") ? req.get("dpitch").getAsFloat() : 0f;
                    var s = ClientActions.state(0).get(2, TimeUnit.SECONDS);
                    if (s.has("player")) {
                        JsonObject p = s.getAsJsonObject("player");
                        state.yawTarget = p.get("yaw").getAsFloat() + dyaw;
                        state.pitchTarget = clampPitch(p.get("pitch").getAsFloat() + dpitch);
                        state.lookStepDeg = instant ? 100000f : state.lookStepDeg;
                        state.lookActive = true;
                    }
                } else {
                    if (req.has("yaw")) state.yawTarget = req.get("yaw").getAsFloat();
                    if (req.has("pitch")) state.pitchTarget = clampPitch(req.get("pitch").getAsFloat());
                    if (instant) state.lookStepDeg = 100000f;
                    state.lookActive = true;
                }
                res.addProperty("ok", true);
            }
            case "attack" -> {
                dev.hyperion.pilot.HyperionPilotClient.pulseAttack();
                res.addProperty("ok", true);
            }
            case "use" -> {
                dev.hyperion.pilot.HyperionPilotClient.pulseUse();
                res.addProperty("ok", true);
            }
            case "slot" -> {
                ClientActions.setSlot(req.get("index").getAsInt());
                res.addProperty("ok", true);
            }
            case "drop" -> {
                ClientActions.drop(req.has("all") && req.get("all").getAsBoolean());
                res.addProperty("ok", true);
            }
            case "chat" -> {
                ClientActions.chat(req.get("message").getAsString());
                res.addProperty("ok", true);
            }
            case "command" -> {
                ClientActions.command(req.get("command").getAsString());
                res.addProperty("ok", true);
            }
            case "screenshot" -> {
                String path = ClientActions.screenshot().get(10, TimeUnit.SECONDS);
                res.addProperty("ok", true);
                res.addProperty("path", path);
            }
            case "state" -> {
                double radius = req.has("radius") ? req.get("radius").getAsDouble() : 32.0;
                JsonObject snap = ClientActions.state(radius).get(5, TimeUnit.SECONDS);
                res.addProperty("ok", true);
                res.add("state", snap);
            }
            case "recent" -> {
                int n = req.has("n") ? req.get("n").getAsInt() : 50;
                JsonArray arr = new JsonArray();
                for (String s : PacketLog.get().recent(n)) arr.add(s);
                res.addProperty("ok", true);
                res.add("packets", arr);
                res.addProperty("dropped", PacketLog.get().droppedCount());
                res.addProperty("written", PacketLog.get().writtenCount());
            }
            case "rotate" -> {
                PacketLog.get().requestRotate();
                res.addProperty("ok", true);
            }
            case "log" -> {
                PacketLog log = PacketLog.get();
                if (req.has("enabled")) log.setEnabled(req.get("enabled").getAsBoolean());
                boolean in = req.has("inbound") ? req.get("inbound").getAsBoolean() : true;
                boolean out = req.has("outbound") ? req.get("outbound").getAsBoolean() : true;
                if (req.has("inbound") || req.has("outbound")) log.setDirections(in, out);
                res.addProperty("ok", true);
                res.addProperty("path", log.currentPath() == null ? null : log.currentPath().toString());
            }
            default -> {
                res.addProperty("ok", false);
                res.addProperty("error", "unknown cmd: " + cmd);
            }
        }
        return res;
    }

    private static float clampPitch(float p) {
        return Math.max(-90f, Math.min(90f, p));
    }
}

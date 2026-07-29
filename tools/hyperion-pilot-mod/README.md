# hyperion-pilot

A client-side Fabric mod for **Minecraft 26.2 (protocol 776)** that lets an agent
observe and pilot the operator's *real, live* Minecraft client while they watch.
Built to test the hyperion server: it is how we finally *see* what the client
renders (own-avatar kit skins, arrow flight) rather than inferring it from a
scripted bot that has no renderer.

It does three things:

1. **Logs every packet** the client sends and receives to a rotating JSONL file.
2. **Shows a live packet overlay** in-client, toggled by a keybind.
3. **Exposes a control socket** an agent connects to in order to walk, look,
   fire a bow, take screenshots, and read back the world.

## Toolchain

Fabric fully supports 26.2 (verified 2026-07-28): Loader `0.19.3`, Loom `1.17`,
Gradle `9.5.1`, Fabric API `0.156.0+26.2`, Java 25. 26.1+ ships **unobfuscated**
with Mojang official mappings, so there is no Yarn and Loom does not remap.

## Build

Needs a JDK 25 on `JAVA_HOME`. The Gradle wrapper pulls Gradle 9.5.1 itself.

```sh
cd tools/hyperion-pilot-mod
JAVA_HOME=/path/to/jdk-25 ./gradlew build
# -> build/libs/hyperion-pilot-0.1.0.jar
```

On this repo's Nix, a JDK 25 is `nix build --no-link --print-out-paths nixpkgs#jdk25`.
(A reproducible Nix derivation of the Gradle build is a follow-up, see ENG-10985:
Gradle-in-Nix dependency fetching is non-trivial and the working Gradle build is
the primary deliverable.)

## Install into the operator's client

1. Install **Fabric Loader 0.19.3** for Minecraft 26.2 (the fabric installer, or
   your launcher's Fabric option).
2. Drop these two jars into `~/Library/Application Support/minecraft/mods/` (or
   your instance's `mods/` folder):
   - `hyperion-pilot-0.1.0.jar` (this mod)
   - **Fabric API** `0.156.0+26.2` (download from Modrinth/CurseForge).
3. Launch the 26.2 + Fabric profile. On join you should see a log line
   `hyperion-pilot: control endpoint listening on unix:...` and, pressing **F6**,
   the packet overlay.

## Where things live

Everything is under a single fixed directory, `~/.hyperion-pilot/`, so an agent
never has to guess the game directory:

| Path | What |
| --- | --- |
| `~/.hyperion-pilot/control.sock` | unix-domain control socket (primary) |
| `~/.hyperion-pilot/endpoint.txt` | the chosen endpoint, e.g. `unix:/…/control.sock` or `tcp:127.0.0.1:25599` |
| `~/.hyperion-pilot/packets/packets-<stamp>.jsonl` | rotating packet log (64 MB rotation) |
| `~/.hyperion-pilot/screenshots/shot-<ms>.png` | screenshots from the `screenshot` command |

If the unix socket cannot bind, the mod falls back to loopback TCP on
`127.0.0.1:25599` and records that in `endpoint.txt`.

## Driving it: the `pilot.py` CLI

`pilot.py` reads `endpoint.txt`, connects, and sends one command per invocation.
It is the intended agent feedback loop: **drive -> screenshot/state -> decide ->
drive**.

```sh
./pilot.py ping
./pilot.py hold --forward --sprint      # start running forward, held over time
./pilot.py look --yaw 90 --pitch 0      # smoothly turn to face +X, level
./pilot.py hold --forward --left        # strafe while walking
./pilot.py stop                         # release every input
./pilot.py slot 0                       # select hotbar slot 0 (e.g. the bow)
./pilot.py use --hold                   # start drawing the bow (hold right-click)
./pilot.py state                        # read draw progress + arrows in flight
./pilot.py stop                         # release use -> fires the bow
./pilot.py screenshot                   # -> {"ok":true,"path":".../shot-….png"}
./pilot.py state --radius 40            # world snapshot (see below)
```

## Packet log format

One JSON object per line:

```json
{"t":1750000000000,"dir":"in","name":"AddEntity","class":"net.minecraft.network.protocol.game.ClientboundAddEntityPacket","fields":{"id":"...","type":"...","x":..,"y":..,"z":..}}
```

`dir` is `in` (clientbound) or `out` (serverbound). `name` is the packet's class
name with a leading `Clientbound`/`Serverbound` and trailing `Packet` stripped,
matching the `PLAY_NAMES` table in `tools/client-26.2.py` so the two agree.
`fields` is a shallow reflective decode of the packet's record components.

## Control command schema

Newline-delimited JSON: one request object per line, one response object per
line. `{"ok":true}` on success, `{"ok":false,"error":"..."}` on failure.

### Held inputs (persist until changed or `stop`)

```json
{"cmd":"hold","forward":true,"back":false,"left":false,"right":false,
 "jump":false,"sneak":false,"sprint":true,"use":false,"attack":false}
```

Only the fields you include change. `use` held == holding right-click (draws a
bow); `attack` held == holding left-click. Movement is folded into the vanilla
input record, so the server sees a normal `ServerboundPlayerInputPacket` and the
operator can co-drive with their own keyboard.

```json
{"cmd":"stop"}
```

### Look

Absolute degrees (Minecraft convention: pitch -90 up, +90 down). Turns at most
`step` degrees per tick (default 30) unless `instant`.

```json
{"cmd":"look","yaw":90,"pitch":0}
{"cmd":"look","dyaw":15,"dpitch":-5}     // relative to current
{"cmd":"look","yaw":90,"instant":true}
{"cmd":"look","yaw":90,"step":10}        // slow, cinematic turn
```

### One-shot

```json
{"cmd":"attack"}                  // single left-click
{"cmd":"use"}                     // single right-click
{"cmd":"slot","index":0}          // select hotbar 0..8 (notifies server)
{"cmd":"drop","all":false}        // drop held item (all:true drops the stack)
{"cmd":"chat","message":"hi"}
{"cmd":"command","command":"gamemode creative"}   // leading / optional
```

### Feedback

```json
{"cmd":"screenshot"}     -> {"ok":true,"path":"/…/screenshots/shot-….png"}
{"cmd":"state","radius":32}
{"cmd":"recent","n":50}  -> {"ok":true,"packets":["▼ AddEntity …", …],"dropped":0,"written":123}
```

`state` returns the world from the character's point of view. Fields relevant to
the two bugs this mod exists to test are called out:

```json
{"ok":true,"state":{
  "player":{
    "pos":{"x":..,"y":..,"z":..},"velocity":{...},"yaw":..,"pitch":..,
    "onGround":true,"health":20.0,"food":20,"sprinting":true,"sneaking":false,
    "usingItem":true,"useItemRemainingTicks":12,   // bow draw progress
    "selectedSlot":0,"mainHand":{"id":"minecraft:bow","count":1},"offHand":{...}
  },
  "nearbyEntities":[{"id":..,"type":"minecraft:arrow","pos":{...},"velocity":{...},"yaw":..,"pitch":..,"onGround":false}, …],
  "arrows":[ … just the arrows, for watching bow flight … ]
}}
```

### Packet-log control

```json
{"cmd":"rotate"}                                    // start a fresh JSONL file
{"cmd":"log","enabled":true,"inbound":true,"outbound":true}
{"cmd":"ping"}
```

## What is validated, and what is not

Verified here: the mod compiles and packages against the real 26.2 client jar;
every mixin target (Connection.channelRead0 / doSendPacket, KeyboardInput.tick,
KeyMapping fields) and every API call was checked against the decompiled 26.2
sources; and the full control transport (unix socket bind, `endpoint.txt`
discovery, JSON dispatch, `pilot.py`) was driven end to end in a plain JVM.

Not yet exercised: the Minecraft-touching commands (screenshot, state, movement,
overlay, bow hold/release) require a running, authenticated GL client and were
not run headless. Load it in the operator's client to exercise those.

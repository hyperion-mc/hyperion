# Packet Inspector

![packet inspector screenshot](https://raw.githubusercontent.com/valence-rs/valence/main/assets/packet-inspector.png)

The packet inspector is a Minecraft proxy for viewing the contents of packets as
they are sent/received. It uses Valence's protocol facilities to display packet
contents. This was made for three purposes:

- Check that packets between Valence and client are matching your expectations.
- Check that packets between vanilla server and client are parsed correctly by
  Valence.
- Understand how the protocol works between the vanilla server and client.

# Usage

Firstly, we should have a server running that we're going to be
proxying/inspecting.

```sh
cargo r -r --example game_of_life
```

Next up, we need to run the proxy server, To launch in a GUI environment, simply run `packet_inspector`.

```sh
cargo r -r -p packet_inspector
```

Then click the "Start Listening" button in the top left of the UI.

The client can now connect to `localhost:25566`. You should see packets streaming in on the GUI.

## Quick start with a vanilla server

nixpkgs ships the vanilla server, so this needs nothing installed but nix.

```sh
mkdir -p /tmp/mc && cd /tmp/mc
echo 'eula=true' > eula.txt
printf 'online-mode=false\nlevel-type=flat\nview-distance=16\ngamemode=creative\nspawn-protection=0\n' > server.properties
nix run nixpkgs#minecraft-server -- nogui
```

In a separate terminal, start the packet inspector between your client and that server.

```sh
cargo run -r -p packet-inspector -- 127.0.0.1:25566 127.0.0.1:25565
```

Open Minecraft and connect to `localhost:25566`.

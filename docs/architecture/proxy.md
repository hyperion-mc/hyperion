# Proxy

```mermaid
sequenceDiagram
    participant P as Player
    participant PH as Proxy Handler
    participant B as Broadcast System
    participant S as Game Server

    Note over P,S: Player → Server Flow
    P->>PH: Player Packet
    PH->>S: Forward Immediately

    Note over P,S: Server → Player Flow
    S->>B: Server Packets

    Note over B: Broadcasting Decision
    alt Local Broadcast
        B->>P: Send to nearby players (BVH)
    else Channel Broadcast
        B->>P: Send to subscribed players
    else Global Broadcast
        B->>P: Send to all players
    else Unicast
        B->>P: Send to specific player
    end
```

The proxy is special because it is aware of certain parts of the game server's internal state: mainly the
chunk locations of each player. This allows to do regional broadcasting very efficiently from the game server.
Taking compute and I/O off of the game server allows for a massive performance boost as the game server can only
vertically scale while the proxy can horizontally scale.

| Operation Type           | Vanilla Minecraft                                                                                            | Proxy-Based Approach                                                                                                                                       |
|--------------------------|--------------------------------------------------------------------------------------------------------------|------------------------------------------------------------------------------------------------------------------------------------------------------------|
| Global Broadcast         | Server sends one packet to every player                                                                      | Server sends one `BroadcastGlobal` packet to the proxy. The proxy will send a packet to each player.                                                       |
| Channel Broadcast        | Server sends one packet to each player in a region and manages packets sent when entering/leaving the region | Server sends one `BroadcastChannel` packet to the proxy. The proxy will send a packet to each player in a region and manage subscribe/unsubscribe packets. |
| Local/Regional Broadcast | Server sends one packet to each player in a region                                                           | Server sends one `BroadcastLocal` packet to the proxy. The proxy will send a packet to each player in a region.                                            |
| Unicast                  | Server sends one packet to a specific player                                                                 | Server sends one `Unicast` packet to the proxy. The proxy will send a packet to a specific player.                                                         |

Using a proxy is a massive optimization for large player counts. For example, to update player positions in Vanilla
Minecraft, the server would need to send every player's position to everyone in the same area as that player. In the
worst-case scenario where every player is in the same area, meaning that every player needs to know the position of
every other player, the server would need to send $n^2$ packets where $n$ is the number of players. This would lead
to a large amount of CPU, memory, and network usage from one server. However, with a proxy-based system, the server
would only need to send $n$ packets to each proxy. Although there would still be $n^2$ total packets sent from each
proxy, this work is spread out across multiple proxies instead of being done on one server.

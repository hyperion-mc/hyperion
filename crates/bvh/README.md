# bvh


I'm currently working on Hyperion, which requires 10,000 players to be in a PvP battle at once. However, people are definitely not going to want to see 10,000 players at once. And while it's probably possible that a server could support this if it were really beefy, the FPS (Frames Per Second) of clients would be extremely low. So, I think it's more realistic if we have something like 10,000 players, and each player can maybe, on average, see around 300 players tops.

This might sound like a lot.
I know a lot of people have experienced their client lagging out when there were this many players,
but a big reason for the lag is the name tags, which can be disabled.
Disabling them actually improves performance significantly because text isn't being rendered.
However, there still needs to be an efficient server implementation for actually sending out the right packets,
ideally with as many bytes contiguous as possible to ensure cache locality can be used,
and also to reduce the number of opcodes sent to `io_uring`, which is beneficial.

For this purpose, I'm building a BVH (Bounding Volume Hierarchy) specifically designed for packet sending that includes a variety of packet types. This BVH is designed with a focus on a location tied to the packet, allowing for the efficient gathering of all data for packets within a certain bounding box.
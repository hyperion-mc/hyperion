# hyperion-platform

The set of operating-system services hyperion needs, behind one narrow module
surface, so that a target without an OS is a backend rather than a fork.

Five things differ between a hosted OS and a unikernel, and only these five:

| module        | hosted (Linux, macOS)       | unikernel (Hermit)                |
| ------------- | --------------------------- | --------------------------------- |
| `limits`      | `setrlimit(RLIMIT_NOFILE)`  | no such limit; reports the cap    |
| `clock`       | monotonic + wall clock      | monotonic; wall clock may be absent |
| `storage`     | the filesystem              | RAM, seeded by the image          |
| `net`         | `std::net` + `AF_UNIX`      | `std::net` over virtio-net only   |
| `parallelism` | `available_parallelism`     | vCPUs handed over at boot         |

Everything else hyperion does is arithmetic, and needs no seam.

The backend is picked by `cfg(target_os)` at compile time. Hosted is the
default and the only one a normal build ever sees, so adding a third platform
means writing a `backend/*.rs` and one `cfg` arm, not editing call sites.

See `docs/bare-metal.md` for what actually builds on the unikernel today.

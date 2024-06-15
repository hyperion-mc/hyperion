# runs all CI checks
default: fmt lint unused-deps deny test

project_root := `git rev-parse --show-toplevel`
arch := `uname -m`

# builds in release mode
build:
    cargo build --release

# cargo clippy
lint:
    cargo clippy --all-targets --all-features -- -D warnings

# cargo nextest
test:
    cargo nextest run

# cargo miri
miri:
    # only run if test prefixed with "miri"
    MIRIFLAGS='-Zmiri-tree-borrows -Zmiri-ignore-leaks' cargo miri nextest run miri

# cargo fmt
fmt:
    cargo fmt

# cargo machete
unused-deps:
    cargo machete

# cargo deny
deny:
    cargo deny check

# run in debug mode with tracy; auto-restarts on changes
debug:
    #!/usr/bin/env -S parallel --shebang --ungroup --jobs 3
    hyperion-proxy
    cargo watch -x build -s 'touch {{project_root}}/.trigger'
    RUN_MODE=debug-{{arch}} cargo watch --postpone --no-vcs-ignores -w {{project_root}}/.trigger -s './target/debug/infection'

# run in release mode with tracy; auto-restarts on changes
release:
    #!/usr/bin/env -S parallel --shebang --ungroup --jobs 3
    ulimit -Sn 1024 && hyperion-proxy
    cargo watch -x 'build --release' -s 'touch {{project_root}}/.trigger'
    RUN_MODE=release-{{arch}} cargo watch --postpone --no-vcs-ignores -w {{project_root}}/.trigger -s './target/release/infection -t'

# run a given number of bots to connect to hyperion
bots count='1000':
    cargo install -q --git https://github.com/andrewgazelka/rust-mc-bot --branch optimize
    ulimit -Sn 1024 && rust-mc-bot 127.0.0.1:25566 {{count}}

# run in release mode with tracy
run:
    cargo run --release -- -t

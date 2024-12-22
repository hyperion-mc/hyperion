//! A build script so benching works with tango-bench

fn main() {
    // for tango-bench
    println!("cargo:rustc-link-arg-benches=-rdynamic");
    println!("cargo:rerun-if-changed=build.rs");
}

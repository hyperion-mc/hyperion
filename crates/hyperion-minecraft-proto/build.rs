//! Packet structs, generated from the committed `protocol.json`.
//!
//! `protocol.json` is what `nix/extract-protocol.py` recovered by following
//! the server's own stream codecs down to the bytes. This turns the layouts it
//! recovered *in full* into Rust; a layout with an unresolved leaf anywhere
//! under it produces no struct at all, because a partial codec is worse than a
//! missing one -- it looks right and desynchronises the stream.
//!
//! Everything here is deterministic and ordered, so a rebuild against an
//! unchanged `protocol.json` produces byte-identical output.

use std::{
    collections::{BTreeMap, BTreeSet},
    fmt::Write as _,
    fs,
    path::{Path, PathBuf},
    process::Command,
};

use serde_json::Value;

/// Connection states, in the order a connection passes through them.
const STATES: [&str; 5] = ["handshake", "status", "login", "configuration", "play"];

/// Where a packet class used by more than one state is defined.
///
/// `ClientboundKeepAlivePacket` is one class serving configuration and play.
/// Emitting it once and re-exporting keeps it one Rust type too, so a value
/// built for one state is accepted by the other.
const COMMON: &str = "common";

const DIRECTIONS: [&str; 2] = ["clientbound", "serverbound"];

fn main() {
    let manifest = PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").expect("manifest dir"));
    let source = manifest.join("protocol.json");
    println!("cargo::rerun-if-changed={}", source.display());
    println!("cargo::rerun-if-changed=build.rs");

    let text = fs::read_to_string(&source)
        .unwrap_or_else(|error| panic!("reading {}: {error}", source.display()));
    let doc: Value = serde_json::from_str(&text)
        .unwrap_or_else(|error| panic!("parsing {}: {error}", source.display()));

    let out = PathBuf::from(std::env::var_os("OUT_DIR").expect("out dir"));
    let generated = Generator::new(&doc).run();

    write(&out.join("types.rs"), &generated.types);
    for (name, body) in &generated.packets {
        write(&out.join(format!("packets/{name}.rs")), body);
    }

    // A build script cannot return a value, so the count is written where a
    // test can read it back and hold the generator to it.
    fs::write(
        out.join("coverage.txt"),
        format!("{} {}\n", generated.written, generated.skipped),
    )
    .expect("write coverage");
}

/// Write a file, formatting it when rustfmt is available.
///
/// The output is spliced into the crate with `include!`, so it shows up in
/// compiler diagnostics and in rustdoc. Formatting it is the difference
/// between those being readable and not; it is best-effort because a build
/// must not fail on a machine without rustfmt.
fn write(path: &Path, body: &str) {
    fs::create_dir_all(path.parent().expect("parent")).expect("create out dir");
    fs::write(path, body).unwrap_or_else(|error| panic!("writing {}: {error}", path.display()));
    let _unused = Command::new("rustfmt")
        .args(["--edition", "2024", "--emit", "files"])
        .arg(path)
        .status();
}

// ---------------------------------------------------------------------------
// Rust identifiers
// ---------------------------------------------------------------------------

/// Words that cannot be an identifier, and so are written as `r#word`.
const KEYWORDS: [&str; 51] = [
    "abstract", "as", "async", "await", "become", "box", "break", "const", "continue", "crate",
    "do", "dyn", "else", "enum", "extern", "false", "final", "fn", "for", "if", "impl", "in",
    "let", "loop", "macro", "match", "mod", "move", "mut", "override", "priv", "pub", "ref",
    "return", "static", "struct", "super", "trait", "true", "try", "type", "typeof", "unsafe",
    "unsized", "use", "virtual", "where", "while", "yield", "gen", "self",
];

/// Split a Java identifier into lower-case words.
///
/// `newCenterX` is three words and `blockPosXYZ` is two, because a run of
/// capitals is one word up to the capital that starts the next.
fn words(name: &str) -> Vec<String> {
    let chars: Vec<char> = name.chars().collect();
    let mut out: Vec<String> = Vec::new();
    let mut current = String::new();
    for (index, &ch) in chars.iter().enumerate() {
        if ch == '_' || ch == '/' || ch == '.' {
            if !current.is_empty() {
                out.push(std::mem::take(&mut current));
            }
            continue;
        }
        let starts_word = ch.is_ascii_uppercase()
            && !current.is_empty()
            && (!chars[index - 1].is_ascii_uppercase()
                || chars.get(index + 1).is_some_and(char::is_ascii_lowercase));
        if starts_word {
            out.push(std::mem::take(&mut current));
        }
        current.push(ch.to_ascii_lowercase());
    }
    if !current.is_empty() {
        out.push(current);
    }
    out
}

/// `minecraft:add_entity` and `ClientboundAddEntityPacket` both give `AddEntity`.
fn pascal(name: &str) -> String {
    words(name.rsplit(':').next().unwrap_or(name))
        .into_iter()
        .map(|word| {
            let mut chars = word.chars();
            chars.next().map_or_else(String::new, |first| {
                first.to_ascii_uppercase().to_string() + chars.as_str()
            })
        })
        .collect()
}

/// `protocolVersion` gives `protocol_version`, escaped if it is a keyword.
fn snake(name: &str) -> String {
    let joined = words(name.rsplit(':').next().unwrap_or(name)).join("_");
    if KEYWORDS.contains(&joined.as_str()) {
        format!("r#{joined}")
    } else if joined.starts_with(|c: char| c.is_ascii_digit()) {
        // A leading digit cannot start an identifier and no escape helps.
        format!("field_{joined}")
    } else {
        joined
    }
}

/// `properties` gives `property`, so that a list's element type is named for
/// one element rather than for the list.
fn singular(name: &str) -> String {
    match name.strip_suffix("ies") {
        Some(stem) => format!("{stem}y"),
        None if name.ends_with("ss") || !name.ends_with('s') => name.to_owned(),
        None => name[..name.len() - 1].to_owned(),
    }
}

/// The simple name of a Java class, keeping only the innermost nested part.
fn simple_name(owner: &str) -> &str {
    owner
        .rsplit('$')
        .next()
        .and_then(|inner| inner.rsplit('.').next())
        .unwrap_or(owner)
}

/// The class a nested class is declared in, if it is nested at all.
fn outer_name(owner: &str) -> Option<&str> {
    owner.rsplit_once('$').map(|(outer, _)| outer)
}
// ---------------------------------------------------------------------------
// Output tree
// ---------------------------------------------------------------------------

/// One module's worth of generated items.
///
/// Imports are one path per line and left for rustfmt to merge, so the output
/// is correct without rustfmt and tidy with it.
#[derive(Default)]
struct Module {
    imports: BTreeSet<String>,
    reexports: BTreeSet<String>,
    /// Items keyed by type name, which is also the order they are written in.
    items: BTreeMap<String, String>,
    children: BTreeMap<String, (String, Self)>,
}

impl Module {
    fn child(&mut self, name: &str, doc: &str) -> &mut Self {
        let entry = self
            .children
            .entry(name.to_owned())
            .or_insert_with(|| (doc.to_owned(), Self::default()));
        if entry.0.is_empty() {
            doc.clone_into(&mut entry.0);
        }
        &mut entry.1
    }

    fn render(&self, indent: usize, out: &mut String) {
        let pad = "    ".repeat(indent);
        // Imports are recorded while lowering, before it is known whether the
        // packet that wanted one survives; the ones nothing ended up naming
        // are dropped here rather than being guessed at up front.
        let mut body = String::new();
        self.render_items(indent, &mut body);
        let used: Vec<&String> = self
            .imports
            .iter()
            .filter(|path| mentions(&body, path.rsplit("::").next().unwrap_or(path)))
            .collect();
        for path in used {
            let _unused = writeln!(out, "{pad}use {path};");
        }
        for path in &self.reexports {
            let _unused = writeln!(out, "{pad}pub use {path};");
        }
        if !out.is_empty() {
            out.push('\n');
        }
        out.push_str(&body);
        while out.ends_with("\n\n") {
            out.pop();
        }
    }

    fn render_items(&self, indent: usize, out: &mut String) {
        let pad = "    ".repeat(indent);
        for body in self.items.values() {
            for line in body.lines() {
                if line.is_empty() {
                    out.push('\n');
                } else {
                    let _unused = writeln!(out, "{pad}{line}");
                }
            }
            out.push('\n');
        }
        for (name, (doc, child)) in &self.children {
            let _unused = writeln!(out, "{pad}/// {doc}");
            let _unused = writeln!(out, "{pad}pub mod {name} {{");
            let mut inner = String::new();
            child.render(indent + 1, &mut inner);
            out.push_str(&inner);
            let _unused = writeln!(out, "{pad}}}");
            out.push('\n');
        }
        while out.ends_with("\n\n") {
            out.pop();
        }
    }
}

/// The derives a type's fields permit, always in the same order.
fn derive_list(traits: Traits) -> Vec<&'static str> {
    let mut derives = vec!["Debug", "Clone"];
    if traits.copy {
        derives.push("Copy");
    }
    derives.push("PartialEq");
    if traits.eq {
        derives.push("Eq");
        derives.push("Hash");
    }
    derives.push("Encode");
    derives.push("Decode");
    derives
}

/// Whether a name appears in the text as a whole word.
fn mentions(text: &str, name: &str) -> bool {
    let boundary = |ch: Option<char>| ch.is_none_or(|ch| !ch.is_alphanumeric() && ch != '_');
    text.match_indices(name).any(|(index, _)| {
        boundary(text[..index].chars().next_back())
            && boundary(text[index + name.len()..].chars().next())
    })
}

/// Where a generated type lives: which file, and which submodule of it.
#[derive(Clone, PartialEq, Eq, Debug)]
struct Place {
    file: String,
    module: Option<String>,
}

impl Place {
    const fn root(file: String) -> Self {
        Self { file, module: None }
    }

    /// How code sitting in `from` names an item defined here, and the import
    /// that makes that spelling work.
    fn path_from(&self, from: &Self, name: &str) -> (String, Option<String>) {
        if self.file != from.file {
            // Only the shared type file is referenced across files.
            return self.module.as_ref().map_or_else(
                || (name.to_owned(), Some(format!("crate::types::{name}"))),
                |module| {
                    (
                        format!("{module}::{name}"),
                        Some(format!("crate::types::{module}")),
                    )
                },
            );
        }
        match (&from.module, &self.module) {
            (a, b) if a == b => (name.to_owned(), None),
            (_, None) => (format!("super::{name}"), None),
            (None, Some(module)) => (format!("{module}::{name}"), None),
            (Some(_), Some(module)) => (format!("super::{module}::{name}"), None),
        }
    }
}

/// Which derives a type's fields permit.
#[derive(Clone, Copy)]
struct Traits {
    /// Whether the type borrows from the buffer, and so needs a lifetime.
    borrows: bool,
    copy: bool,
    /// `Eq`, and with it `Hash`: false as soon as a float is anywhere inside.
    eq: bool,
}

/// A lowered field or element type.
#[derive(Clone)]
struct Lowered {
    ty: String,
    traits: Traits,
    limits: Limits,
    doc: Option<String>,
    /// A module supplying the whole field's codec, where the derive's
    /// structural rules do not describe the layout.
    with: Option<&'static str>,
}

impl Lowered {
    fn scalar(ty: &str, traits: Traits) -> Self {
        Self {
            ty: ty.to_owned(),
            traits,
            limits: Limits::default(),
            doc: None,
            with: None,
        }
    }

    /// Put this type inside a combinator, keeping what still applies.
    fn wrap(self, ty: String, copy: bool) -> Result<Self, String> {
        if self.with.is_some() {
            // The derive applies a custom codec to a whole field, so it cannot
            // reach inside a list or an `Either`.
            return Err(
                "a custom codec inside a combinator needs a hand-written packet".to_owned(),
            );
        }
        Ok(Self {
            ty,
            traits: Traits {
                copy: self.traits.copy && copy,
                ..self.traits
            },
            doc: None,
            ..self
        })
    }
}

/// The `#[proto(...)]` limits a field's subtree asks for.
///
/// The derive threads one string limit and one collection limit through
/// `Option` and `Vec`, so two of either kind in one field cannot be expressed.
/// Rather than drop one, the field is refused and the packet goes unwritten.
#[derive(Clone, Copy, Default, PartialEq, Eq)]
struct Limits {
    len: Option<u64>,
    count: Option<u64>,
}

impl Limits {
    fn merge(self, other: Self) -> Result<Self, String> {
        Ok(Self {
            len: pick(self.len, other.len, "max_len")?,
            count: pick(self.count, other.count, "max_count")?,
        })
    }

    fn render(self) -> Option<String> {
        let mut parts = Vec::new();
        if let Some(len) = self.len {
            parts.push(format!("max_len = {len}"));
        }
        if let Some(count) = self.count {
            parts.push(format!("max_count = {count}"));
        }
        (!parts.is_empty()).then(|| format!("#[proto({})]", parts.join(", ")))
    }
}

fn pick(a: Option<u64>, b: Option<u64>, what: &str) -> Result<Option<u64>, String> {
    match (a, b) {
        (Some(a), Some(b)) if a != b => Err(format!("two different `{what}` limits in one field")),
        (Some(value), _) | (None, Some(value)) => Ok(Some(value)),
        (None, None) => Ok(None),
    }
}

// ---------------------------------------------------------------------------
// Generator
// ---------------------------------------------------------------------------

/// A packet as the generator needs it: one Java class, and every state that
/// sends it.
#[derive(Clone)]
struct PacketDef {
    java_class: String,
    codec: String,
    resource: String,
    name: String,
    module: String,
    direction: String,
    wire: Value,
    /// `(state, protocol id)` for every channel this class serves, in the
    /// order a connection passes through the states.
    channels: Vec<(String, i64)>,
}

impl PacketDef {
    /// What the struct is called on the wire, where it is sent, and the class
    /// to read if the layout ever needs checking.
    fn doc(&self) -> String {
        let channels: Vec<String> = self
            .channels
            .iter()
            .map(|(state, id)| format!("{state} id {id}"))
            .collect();
        format!(
            "`{}`, sent {} as {}.\n\nLayout from `{}`.",
            self.resource,
            self.direction,
            channels.join(" and "),
            self.codec,
        )
    }

    fn file(&self) -> String {
        format!("{}_{}", self.module, self.direction)
    }

    /// The module holding the types this packet nests.
    fn scope(&self) -> String {
        snake(&self.resource)
    }
}

/// A generated type that has been placed, so a second reference to it is a
/// path rather than a second definition.
#[derive(Clone)]
struct Placed {
    place: Place,
    name: String,
    traits: Traits,
}

struct Generated {
    types: String,
    packets: BTreeMap<String, String>,
    written: usize,
    skipped: usize,
}

struct Generator<'a> {
    types: &'a serde_json::Map<String, Value>,
    packets: Vec<PacketDef>,
    /// Java classes that are packets, which is what decides whether a nested
    /// class belongs beside a packet or in `crate::types`.
    packet_classes: BTreeSet<String>,
    files: BTreeMap<String, Module>,
    placed: BTreeMap<String, Placed>,
    /// Named references currently being lowered, which is how a cycle is
    /// caught before it becomes an infinitely sized Rust type.
    active: BTreeSet<String>,
    skipped: BTreeMap<String, Vec<String>>,
}

impl<'a> Generator<'a> {
    fn new(doc: &'a Value) -> Self {
        let packets = collect_packets(doc);
        Self {
            types: doc["types"].as_object().expect("types table"),
            packet_classes: packets
                .iter()
                .map(|packet| packet.java_class.clone())
                .collect(),
            packets,
            files: BTreeMap::new(),
            placed: BTreeMap::new(),
            active: BTreeSet::new(),
            skipped: BTreeMap::new(),
        }
    }

    fn run(mut self) -> Generated {
        // Every module is written even when it is empty, so that the file each
        // shim includes always exists and a state losing its last generated
        // packet is a diff rather than a build failure.
        self.files.entry("types".to_owned()).or_default();
        for home in STATES.into_iter().chain([COMMON]) {
            for direction in DIRECTIONS {
                self.files.entry(format!("{home}_{direction}")).or_default();
            }
        }
        for index in 0..self.packets.len() {
            self.emit_packet(index);
        }
        self.reexport_common();
        self.render()
    }

    fn emit_packet(&mut self, index: usize) {
        let packet = self.packets[index].clone();
        if self.placed.contains_key(&packet.codec) {
            return;
        }
        let place = Place::root(packet.file());
        // Reserve the name before lowering, so a layout that turns out to
        // reference this packet finds a path rather than defining it twice.
        if !self.active.insert(packet.codec.clone()) {
            return;
        }
        let result = self.struct_body(
            &packet.name,
            Some(&packet.doc()),
            &packet.wire,
            &place,
            &packet.scope(),
        );
        self.active.remove(&packet.codec);

        match result {
            Ok((body, traits)) => {
                let module = self.file(&place);
                module.imports.insert("crate::Decode".to_owned());
                module.imports.insert("crate::Encode".to_owned());
                module.items.insert(packet.name.clone(), body);
                self.placed.insert(packet.codec.clone(), Placed {
                    place,
                    name: packet.name.clone(),
                    traits,
                });
            }
            Err(reason) => self
                .skipped
                .entry(packet.file())
                .or_default()
                .push(format!("{} -- {reason}", packet.resource)),
        }
    }

    /// Bring the packets that several states share into each of those states.
    ///
    /// `ClientboundKeepAlivePacket` is one Java class serving configuration
    /// and play. Defining it once and re-exporting keeps it one Rust type, so
    /// a value built for one state is the same value in the other.
    fn reexport_common(&mut self) {
        let shared: Vec<PacketDef> = self
            .packets
            .iter()
            .filter(|packet| packet.module == COMMON && self.placed.contains_key(&packet.codec))
            .cloned()
            .collect();
        for packet in shared {
            let has_scope = self
                .files
                .get(&packet.file())
                .is_some_and(|file| file.children.contains_key(&packet.scope()));
            for (state, _) in &packet.channels {
                let target = Place::root(format!("{state}_{}", packet.direction));
                let base = format!("crate::packets::common::{}", packet.direction);
                let module = self.file(&target);
                module.reexports.insert(format!("{base}::{}", packet.name));
                if has_scope {
                    module
                        .reexports
                        .insert(format!("{base}::{}", packet.scope()));
                }
            }
        }
    }

    fn render(&self) -> Generated {
        let mut out = Generated {
            types: String::new(),
            packets: BTreeMap::new(),
            written: self
                .packets
                .iter()
                .filter(|packet| self.placed.contains_key(&packet.codec))
                .count(),
            skipped: self.skipped.values().map(Vec::len).sum(),
        };
        for (file, module) in &self.files {
            let mut body = String::from(HEADER);
            if let Some(reasons) = self.skipped.get(file) {
                body.push_str(REFUSED);
                for reason in reasons {
                    let _unused = writeln!(body, "//   {reason}");
                }
            }
            body.push('\n');
            module.render(0, &mut body);
            if file == "types" {
                out.types = body;
            } else {
                out.packets.insert(file.clone(), body);
            }
        }
        out
    }
}

/// Complete packets, one entry per Java class.
fn collect_packets(doc: &Value) -> Vec<PacketDef> {
    let mut by_class: BTreeMap<String, PacketDef> = BTreeMap::new();
    for entry in doc["packets"].as_array().expect("packets") {
        if !entry["complete"].as_bool().unwrap_or(false) {
            continue;
        }
        let state = entry["state"].as_str().expect("state").to_owned();
        let id = entry["protocolId"].as_i64().expect("protocol id");
        let java_class = entry["javaClass"].as_str().unwrap_or_default().to_owned();
        by_class
            .entry(java_class.clone())
            .or_insert_with(|| PacketDef {
                java_class,
                codec: entry["layoutSource"]
                    .as_str()
                    .unwrap_or_default()
                    .to_owned(),
                resource: entry["resource"].as_str().expect("resource").to_owned(),
                name: pascal(entry["resource"].as_str().expect("resource")),
                module: state.clone(),
                direction: entry["direction"].as_str().expect("direction").to_owned(),
                wire: entry["wire"].clone(),
                channels: Vec::new(),
            })
            .channels
            .push((state, id));
    }

    let mut packets: Vec<PacketDef> = by_class.into_values().collect();
    for packet in &mut packets {
        packet
            .channels
            .sort_by_key(|(state, _)| STATES.iter().position(|name| name == state));
        if packet.channels.len() > 1 {
            COMMON.clone_into(&mut packet.module);
        }
    }
    packets
}

const HEADER: &str = "// @generated by build.rs from protocol.json. Do not edit.\n";

const REFUSED: &str = "//\n// Layouts the extractor recovered in full but this generator \
                       declined, and\n// why. Each needs a hand-written codec; none of them is \
                       approximated here.\n";

// ---------------------------------------------------------------------------
// Lowering a wire tree to Rust
// ---------------------------------------------------------------------------

/// A scalar wire kind and its Rust spelling, with the derives it permits.
///
/// Anything not listed has no Rust spelling yet, and a packet that reaches one
/// goes unwritten rather than approximated.
const SCALARS: [(&str, &str, bool, bool, bool); 21] = [
    // kind, rust type, borrows, copy, eq
    ("unit", "()", false, true, true),
    ("bool", "bool", false, true, true),
    ("i8", "i8", false, true, true),
    ("u8", "u8", false, true, true),
    ("i16", "i16", false, true, true),
    ("u16", "u16", false, true, true),
    ("i32", "i32", false, true, true),
    ("i64", "i64", false, true, true),
    ("f32", "f32", false, true, false),
    ("f64", "f64", false, true, false),
    ("varint", "VarInt", false, true, true),
    ("varlong", "VarLong", false, true, true),
    ("uuid", "Uuid", false, true, true),
    ("block_pos", "BlockPos", false, true, true),
    ("chunk_pos", "ChunkPos", false, true, true),
    ("block_hit_result", "BlockHitResult", false, true, false),
    ("identifier", "Identifier<'a>", true, true, true),
    ("string", "&'a str", true, true, true),
    ("byte_array", "&'a [u8]", true, true, true),
    ("long_array", "Vec<i64>", false, false, true),
    ("varint_array", "Vec<VarInt>", false, false, true),
];

/// Types the generated code may name, all re-exported at the crate root so
/// that one import works however deeply a module nests.
const ROOT_TYPES: [&str; 12] = [
    "BlockHitResult",
    "BlockPos",
    "ChunkPos",
    "Either",
    "Holder",
    "Identifier",
    "LengthPrefixed",
    "RegistryId",
    "Uuid",
    "VarInt",
    "VarLong",
    "nbt",
];

impl Generator<'_> {
    /// The module at a place, creating it if this is the first item in it.
    fn file(&mut self, at: &Place) -> &mut Module {
        let file = self.files.entry(at.file.clone()).or_default();
        match &at.module {
            Some(module) => file.child(module, ""),
            None => file,
        }
    }

    /// Follow `named` references to the node that actually describes bytes.
    fn resolve<'v>(&'v self, wire: &'v Value) -> &'v Value {
        let mut wire = wire;
        let mut seen = BTreeSet::new();
        while wire["kind"] == "named" {
            let Some(reference) = wire["ref"].as_str() else {
                break;
            };
            if !seen.insert(reference.to_owned()) {
                break;
            }
            let Some(next) = self.types.get(reference) else {
                break;
            };
            wire = next;
        }
        wire
    }

    /// The Rust type for one wire node, defining any struct it needs on the way.
    ///
    /// `at` is the module the type will be named from and `scope` the
    /// submodule holding the types this packet or class nests. `label` is the
    /// field the node hangs off, which is what an anonymous struct is named
    /// after.
    fn lower(
        &mut self,
        wire: &Value,
        at: &Place,
        scope: &str,
        label: &str,
    ) -> Result<Lowered, String> {
        if wire["kind"] == "named"
            && let Some(reference) = wire["ref"].as_str()
        {
            return self.lower_named(reference, at, scope, label);
        }

        let kind = wire["kind"].as_str().unwrap_or("unresolved");
        if let Some(&(_, ty, borrows, copy, eq)) = SCALARS.iter().find(|entry| entry.0 == kind) {
            let mut lowered = Lowered::scalar(ty, Traits { borrows, copy, eq });
            // A string's limit is UTF-16 code units and a byte array's is
            // bytes; `max_len` means "the length this type is measured in".
            if matches!(kind, "string" | "byte_array") {
                lowered.limits.len = wire["max"].as_u64();
            }
            self.note_import(at, ty);
            return Ok(lowered);
        }

        match kind {
            "registry_id" => {
                self.note_import(at, "RegistryId");
                Ok(Lowered {
                    doc: wire["registry"]
                        .as_str()
                        .map(|registry| format!("Index into `{registry}`.")),
                    ..Lowered::scalar("RegistryId", Traits {
                        borrows: false,
                        copy: true,
                        eq: true,
                    })
                })
            }
            "nbt" => {
                self.note_import(at, "nbt");
                Ok(Lowered::scalar("nbt::Tag<'a>", Traits {
                    borrows: true,
                    copy: false,
                    eq: false,
                }))
            }
            // A root `TAG_End` means absent here, which is not the
            // boolean-prefixed `Option` the derive writes by default.
            "optional_nbt" => {
                self.note_import(at, "nbt");
                Ok(Lowered {
                    with: Some("crate::nbt::optional"),
                    ..Lowered::scalar("Option<nbt::Tag<'a>>", Traits {
                        borrows: true,
                        copy: false,
                        eq: false,
                    })
                })
            }
            "option" => {
                let inner = self.lower(&wire["of"], at, scope, label)?;
                let ty = format!("Option<{}>", inner.ty);
                inner.wrap(ty, true)
            }
            "list" => {
                let inner = self.lower(&wire["of"], at, scope, &singular(label))?;
                let ty = format!("Vec<{}>", inner.ty);
                let count = wire["max"].as_u64();
                let mut out = inner.wrap(ty, false)?;
                out.limits = out.limits.merge(Limits { len: None, count })?;
                Ok(out)
            }
            "length_prefixed" => {
                let inner = self.lower(&wire["of"], at, scope, label)?;
                self.note_import(at, "LengthPrefixed");
                let ty = format!("LengthPrefixed<{}>", inner.ty);
                inner.wrap(ty, true)
            }
            "holder" => {
                let inner = self.lower(&wire["of"], at, scope, label)?;
                self.note_import(at, "Holder");
                let ty = format!("Holder<{}>", inner.ty);
                let doc = wire["registry"]
                    .as_str()
                    .map(|registry| format!("A `{registry}` entry, by id or written out in full."));
                Ok(Lowered {
                    doc,
                    ..inner.wrap(ty, true)?
                })
            }
            "either" => {
                let left = self.lower(&wire["left"], at, scope, label)?;
                let right = self.lower(&wire["right"], at, scope, label)?;
                self.note_import(at, "Either");
                let ty = format!("Either<{}, {}>", left.ty, right.ty);
                let limits = left.limits.merge(right.limits)?;
                let traits = Traits {
                    borrows: left.traits.borrows || right.traits.borrows,
                    copy: left.traits.copy && right.traits.copy,
                    eq: left.traits.eq && right.traits.eq,
                };
                let merged = Lowered {
                    limits,
                    ..Lowered::scalar(&ty, traits)
                };
                if left.with.is_some() || right.with.is_some() {
                    return Err(
                        "a custom codec inside `Either` needs a hand-written packet".to_owned()
                    );
                }
                Ok(merged)
            }
            "enum" => self.lower_enum(wire, at, scope),
            "map" => self.lower_map(wire, at, scope, label),
            // An anonymous struct: no Java class named it, so it takes the
            // name of the field it hangs off and sits in the packet's scope.
            "struct" => {
                let name = pascal(label);
                let place = Place {
                    file: at.file.clone(),
                    module: Some(scope.to_owned()),
                };
                self.define(&name, None, wire, at, &place, scope, None)
            }
            "unresolved" => Err(wire["why"]
                .as_str()
                .unwrap_or("unresolved layout")
                .to_owned()),
            other => Err(format!("no Rust type for wire kind `{other}`")),
        }
    }

    /// A Java enum, which the server sends as a `VarInt` and which is worth
    /// keeping as an enum: a field typed this way cannot hold a number the
    /// server would reject.
    fn lower_enum(&mut self, wire: &Value, at: &Place, scope: &str) -> Result<Lowered, String> {
        let owner = wire["type"].as_str().ok_or("enum without a class")?;
        if let Some(placed) = self.placed.get(owner).cloned() {
            let ty = self.reference(&placed, at);
            return Ok(Lowered::scalar(&ty, placed.traits));
        }

        let name = pascal(simple_name(owner));
        let (place, _) = self.place_for(owner, at, scope, &name);
        let sent = wire["note"].as_str().unwrap_or("varint");
        let mut body = format!("/// `{owner}`, sent as a {sent}.\n");
        body.push_str("#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Encode, Decode)]\n");
        let _unused = writeln!(body, "pub enum {name} {{");
        for constant in wire["constants"]
            .as_array()
            .ok_or("enum without constants")?
        {
            let java = constant["name"].as_str().ok_or("constant without a name")?;
            let value = constant["value"]
                .as_i64()
                .ok_or("constant without a value")?;
            let _unused = writeln!(body, "    /// `{java}`");
            let _unused = writeln!(body, "    {} = {value},", pascal(java));
        }
        body.push_str("}\n");

        self.describe_scope(&place);
        let module = self.file(&place);
        module.imports.insert("crate::Decode".to_owned());
        module.imports.insert("crate::Encode".to_owned());
        module.items.insert(name.clone(), body);

        let placed = Placed {
            place,
            name,
            traits: Traits {
                borrows: false,
                copy: true,
                eq: true,
            },
        };
        self.placed.insert(owner.to_owned(), placed.clone());
        Ok(Lowered::scalar(&self.reference(&placed, at), placed.traits))
    }

    /// A map is a count and then pairs. As a list of named entries it reads
    /// better than a list of tuples, and each half can carry its own limit.
    fn lower_map(
        &mut self,
        wire: &Value,
        at: &Place,
        scope: &str,
        label: &str,
    ) -> Result<Lowered, String> {
        let name = format!("{}Entry", pascal(label));
        let shape = serde_json::json!({
            "kind": "struct",
            "fields": [
                { "name": "key", "wire": wire["key"] },
                { "name": "value", "wire": wire["value"] },
            ],
        });
        let place = Place {
            file: at.file.clone(),
            module: Some(scope.to_owned()),
        };
        let entry = self.define(&name, None, &shape, at, &place, scope, None)?;
        let ty = format!("Vec<{}>", entry.ty);
        let count = wire["max"].as_u64();
        let mut out = entry.wrap(ty, false)?;
        out.limits = out.limits.merge(Limits { len: None, count })?;
        Ok(out)
    }

    /// A reference to a named codec: a struct becomes a Rust type, anything
    /// else is simply the layout it stands for.
    fn lower_named(
        &mut self,
        reference: &str,
        at: &Place,
        scope: &str,
        label: &str,
    ) -> Result<Lowered, String> {
        let owner = reference.split('#').next().unwrap_or(reference);
        // A packet's own codec: emit that packet where it belongs rather than
        // a second copy of it here.
        if self.packet_classes.contains(owner)
            && !self.placed.contains_key(reference)
            && let Some(index) = self
                .packets
                .iter()
                .position(|packet| packet.codec == reference)
        {
            self.emit_packet(index);
        }

        if let Some(placed) = self.placed.get(reference).cloned() {
            let ty = self.reference(&placed, at);
            return Ok(Lowered::scalar(&ty, placed.traits));
        }

        let Some(target) = self.types.get(reference).cloned() else {
            return Err(format!("no layout for `{reference}`"));
        };
        if target["kind"] != "struct" {
            // A named scalar such as `ByteBufCodecs#VAR_INT` names a layout,
            // not a type, so it lowers to that layout under the field's own
            // name: the codec holder's class name says nothing useful.
            return self.lower(&target, at, scope, label);
        }
        if !self.active.insert(reference.to_owned()) {
            return Err(format!("`{reference}` contains itself"));
        }
        let result = self.define_named(reference, &target, at, scope);
        self.active.remove(reference);
        result
    }

    /// Record that a file names one of the crate's own types.
    fn note_import(&mut self, at: &Place, ty: &str) {
        let head = ty
            .trim_start_matches("&'a ")
            .split(['<', ':'])
            .next()
            .unwrap_or(ty);
        if ROOT_TYPES.contains(&head) {
            self.file(at).imports.insert(format!("crate::{head}"));
        }
    }
}

// ---------------------------------------------------------------------------
// Defining and rendering a struct
// ---------------------------------------------------------------------------

impl Generator<'_> {
    /// Define a named codec's struct and return how `at` refers to it.
    fn define_named(
        &mut self,
        reference: &str,
        target: &Value,
        at: &Place,
        scope: &str,
    ) -> Result<Lowered, String> {
        let owner = reference.split('#').next().unwrap_or(reference);
        let member = reference.rsplit('#').next().unwrap_or("STREAM_CODEC");
        // A member ending in `_CODEC` describes a variant of the class it sits
        // in -- `AttributeSnapshot#MODIFIER_STREAM_CODEC` is a modifier, not a
        // second snapshot -- while one that does not is the type's own name,
        // as with the codecs collected in `ByteBufCodecs`.
        let qualifier = member
            .trim_end_matches("_STREAM_CODEC")
            .trim_end_matches("_CODEC");
        let name = if member == "STREAM_CODEC" {
            pascal(simple_name(owner))
        } else if qualifier == member {
            pascal(member)
        } else {
            format!("{}{}", pascal(simple_name(owner)), pascal(qualifier))
        };

        let (place, child_scope) = self.place_for(owner, at, scope, &name);
        let doc = format!("Layout from `{reference}`.");
        self.define(
            &name,
            Some(&doc),
            target,
            at,
            &place,
            &child_scope,
            Some(reference),
        )
    }

    /// Where a named class's Rust type goes.
    ///
    /// A class nested in a packet belongs beside that packet; anything else is
    /// a domain type shared across packets and belongs in `crate::types`. In
    /// both cases a nested class keeps Java's own scoping, as a module named
    /// after the class it is declared in.
    fn place_for(&self, owner: &str, at: &Place, scope: &str, name: &str) -> (Place, String) {
        match outer_name(owner) {
            Some(outer) if self.packet_classes.contains(outer) => (
                Place {
                    file: at.file.clone(),
                    module: Some(scope.to_owned()),
                },
                scope.to_owned(),
            ),
            Some(outer) => {
                let module = snake(simple_name(outer));
                (
                    Place {
                        file: "types".to_owned(),
                        module: Some(module.clone()),
                    },
                    module,
                )
            }
            None => (Place::root("types".to_owned()), snake(name)),
        }
    }

    /// Render a struct into `place` and return how `at` refers to it.
    #[expect(
        clippy::too_many_arguments,
        reason = "where a type goes, where it is named from, and where its own nested types go \
                  are three independent facts"
    )]
    fn define(
        &mut self,
        name: &str,
        doc: Option<&str>,
        wire: &Value,
        at: &Place,
        place: &Place,
        scope: &str,
        reference: Option<&str>,
    ) -> Result<Lowered, String> {
        let (body, traits) = self.struct_body(name, doc, wire, place, scope)?;
        self.describe_scope(place);
        let module = self.file(place);
        module.imports.insert("crate::Decode".to_owned());
        module.imports.insert("crate::Encode".to_owned());
        module.items.insert(name.to_owned(), body);

        let placed = Placed {
            place: place.clone(),
            name: name.to_owned(),
            traits,
        };
        if let Some(reference) = reference {
            self.placed.insert(reference.to_owned(), placed.clone());
        }
        Ok(Lowered::scalar(&self.reference(&placed, at), traits))
    }

    /// How code in `at` spells a type that has already been placed.
    ///
    /// A short name is imported unless the referring module already defines
    /// that name -- `minecraft:recipe_book_settings` is a packet and
    /// `net.minecraft.stats.RecipeBookSettings` is a domain type, and one of
    /// them has to be spelled out.
    fn reference(&mut self, placed: &Placed, at: &Place) -> String {
        let (short, import) = placed.place.path_from(at, &placed.name);
        let ty = match import {
            // Only the shared type file is referenced across files, so the
            // spelled-out form is always rooted there.
            Some(_) if self.occupied(at, &placed.name) => format!("crate::types::{short}"),
            Some(import) => {
                self.file(at).imports.insert(import);
                short
            }
            None => short,
        };
        if placed.traits.borrows {
            format!("{ty}<'a>")
        } else {
            ty
        }
    }

    /// Whether a file's root module already defines this type name.
    fn occupied(&self, at: &Place, name: &str) -> bool {
        self.packets
            .iter()
            .any(|packet| packet.file() == at.file && packet.name == name)
    }

    /// Give a submodule its doc comment, once it has an occupant.
    ///
    /// A scope is always named after the type whose nested types it holds, so
    /// the module name is enough to say whose they are.
    fn describe_scope(&mut self, place: &Place) {
        let Some(module) = place.module.clone() else {
            return;
        };
        let doc = format!("Types declared inside `{}`.", pascal(&module));
        self.files
            .entry(place.file.clone())
            .or_default()
            .child(&module, &doc);
    }

    /// The text of one struct, and the derives its fields permit.
    fn struct_body(
        &mut self,
        name: &str,
        doc: Option<&str>,
        wire: &Value,
        place: &Place,
        scope: &str,
    ) -> Result<(String, Traits), String> {
        let resolved = self.resolve(wire).clone();
        let kind = resolved["kind"].as_str().unwrap_or("unresolved");

        let mut out = String::new();
        for line in doc.unwrap_or_default().lines() {
            let _unused = if line.is_empty() {
                writeln!(out, "///")
            } else {
                writeln!(out, "/// {line}")
            };
        }

        if kind == "unit" {
            out.push_str(
                "#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Encode, Decode)]\n",
            );
            let _unused = writeln!(out, "pub struct {name};");
            return Ok((out, Traits {
                borrows: false,
                copy: true,
                eq: true,
            }));
        }
        if kind != "struct" {
            // One value with nothing around it: the server's codec for
            // `keep_alive` is `ByteBufCodecs.LONG`, not a composite, and a
            // newtype says that more honestly than a one-field struct would.
            let lowered = self.lower(&resolved, place, scope, &snake(name))?;
            let lifetime = if lowered.traits.borrows { "<'a>" } else { "" };
            if let Some(doc) = &lowered.doc {
                let _unused = writeln!(out, "///\n/// {doc}");
            }
            let _unused = writeln!(out, "#[derive({})]", derive_list(lowered.traits).join(", "));
            let attr = lowered
                .with
                .map(|with| format!("#[proto(with = {with})] "))
                .or_else(|| lowered.limits.render().map(|attr| format!("{attr} ")))
                .unwrap_or_default();
            let _unused = writeln!(
                out,
                "pub struct {name}{lifetime}({attr}pub {});",
                lowered.ty
            );
            return Ok((out, lowered.traits));
        }

        let empty = Vec::new();
        let mut fields = Vec::new();
        let mut traits = Traits {
            borrows: false,
            copy: true,
            eq: true,
        };
        let mut seen: BTreeSet<String> = BTreeSet::new();
        for field in resolved["fields"].as_array().unwrap_or(&empty) {
            let label = field["name"].as_str().unwrap_or("value");
            let rust = snake(label);
            if !seen.insert(rust.clone()) {
                return Err(format!("two fields would both be named `{rust}`"));
            }
            let lowered = self.lower(&field["wire"], place, scope, label)?;
            traits.borrows |= lowered.traits.borrows;
            traits.copy &= lowered.traits.copy;
            traits.eq &= lowered.traits.eq;
            fields.push((rust, lowered));
        }

        let _unused = writeln!(out, "#[derive({})]", derive_list(traits).join(", "));
        // clippy suggests a state machine past three flags. Here the flags are
        // the bytes, and grouping them would not change what is written.
        if fields
            .iter()
            .filter(|(_, field)| field.ty == "bool")
            .count()
            > 3
        {
            out.push_str(
                "#[expect(\n    clippy::struct_excessive_bools,\n    reason = \"the server writes \
                 this many independent flags\"\n)]\n",
            );
        }

        let lifetime = if traits.borrows { "<'a>" } else { "" };
        let _unused = writeln!(out, "pub struct {name}{lifetime} {{");
        for (rust, lowered) in fields {
            if let Some(doc) = &lowered.doc {
                let _unused = writeln!(out, "    /// {doc}");
            }
            if let Some(with) = lowered.with {
                let _unused = writeln!(out, "    #[proto(with = {with})]");
            } else if let Some(attr) = lowered.limits.render() {
                let _unused = writeln!(out, "    {attr}");
            }
            let _unused = writeln!(out, "    pub {rust}: {},", lowered.ty);
        }
        out.push_str("}\n");
        Ok((out, traits))
    }
}

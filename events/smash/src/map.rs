//! Map descriptions: what a Super Smash Mobs arena is, as data.
//!
//! Mineplex shipped every map as a world folder plus a `WorldConfig.dat` text
//! file, so a map was data a builder edited and never code a programmer
//! recompiled. That split is worth keeping, and it is the reason this is a
//! parsed text format rather than a table of Rust constants: the arenas here
//! are reconstructions, they will be wrong in their details, and a wrong
//! reconstruction should be fixable by editing a file rather than by knowing
//! Rust.
//!
//! It is not Mineplex's format. Theirs pointed at a world folder for the
//! geometry and carried only the metadata; the geometry is the part we do not
//! have, so this format has to carry it too. The brushes below are the smallest
//! set that draws a floating platform over a void, which is every SSM map.
//!
//! Files are compiled in with `include_str!` rather than read from disk at
//! boot. A game server started by `nix run` has no stable working directory,
//! and a map that fails to load is not a degraded server, it is a server with
//! no floor.

use glam::Vec3;

/// One solid to stamp into the world.
///
/// Coordinates are absolute block positions and every range is inclusive, which
/// is how a builder thinks about a platform ("y 63 to 66") and avoids the
/// off-by-one that an exclusive end invites in hand-written data.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Brush {
    /// Axis-aligned cuboid between two corners.
    Box {
        min: [i32; 3],
        max: [i32; 3],
        block: &'static str,
    },
    /// Vertical cylinder: centre of the base, radius, height upwards.
    Cylinder {
        centre: [i32; 3],
        radius: i32,
        height: i32,
        block: &'static str,
    },
    /// Solid sphere.
    Sphere {
        centre: [i32; 3],
        radius: i32,
        block: &'static str,
    },
    /// Downward taper: `radius` at `centre.y`, shrinking to a point `depth`
    /// blocks below. The underside every floating island has.
    Cone {
        centre: [i32; 3],
        radius: i32,
        depth: i32,
        block: &'static str,
    },
}

/// A whole arena.
#[derive(Debug, Clone, PartialEq)]
pub struct MapSpec {
    pub name: &'static str,
    pub author: &'static str,
    /// Below this Y a player is dead.
    pub kill_y: f32,
    pub spawns: Vec<Vec3>,
    /// Where the Smash Crystal can land. Mineplex's map spec asks for exactly
    /// three of these ("3 red data points for areas where the Smash Crystal
    /// will spawn"), so a map with a different count is a map that does not
    /// follow the spec rather than an error.
    pub crystals: Vec<Vec3>,
    pub brushes: Vec<Brush>,
}

/// Where a map file went wrong, with the line that did it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseError {
    pub line: usize,
    pub reason: String,
}

impl core::fmt::Display for ParseError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "line {}: {}", self.line, self.reason)
    }
}

struct Cursor<'a> {
    words: core::str::SplitWhitespace<'a>,
    line: usize,
}

impl Cursor<'_> {
    fn int(&mut self, what: &str) -> Result<i32, ParseError> {
        let word = self.words.next().ok_or_else(|| self.missing(what))?;
        word.parse().map_err(|_| ParseError {
            line: self.line,
            reason: format!("{what} is not a whole number: {word:?}"),
        })
    }

    fn float(&mut self, what: &str) -> Result<f32, ParseError> {
        let word = self.words.next().ok_or_else(|| self.missing(what))?;
        word.parse().map_err(|_| ParseError {
            line: self.line,
            reason: format!("{what} is not a number: {word:?}"),
        })
    }

    /// A block id, checked only for the `minecraft:` prefix. Whether the id
    /// names a real block is the block table's job, and it says so at boot.
    fn block(&mut self, source: &'static str) -> Result<&'static str, ParseError> {
        let word = self.words.next().ok_or_else(|| self.missing("a block"))?;
        if !word.starts_with("minecraft:") {
            return Err(ParseError {
                line: self.line,
                reason: format!("block {word:?} is missing its `minecraft:` prefix"),
            });
        }
        Ok(borrow_static(source, word))
    }

    fn triple(&mut self, what: &str) -> Result<[i32; 3], ParseError> {
        Ok([self.int(what)?, self.int(what)?, self.int(what)?])
    }

    fn missing(&self, what: &str) -> ParseError {
        ParseError {
            line: self.line,
            reason: format!("expected {what}"),
        }
    }

    fn end(&mut self) -> Result<(), ParseError> {
        match self.words.next() {
            None => Ok(()),
            Some(extra) => Err(ParseError {
                line: self.line,
                reason: format!("unexpected trailing {extra:?}"),
            }),
        }
    }
}

/// Recover the `'static` lifetime of a slice of a `'static` source.
///
/// `include_str!` yields a `&'static str`, but the iterators walking it hand
/// back slices borrowed from the local binding rather than from the original.
/// The offset arithmetic proves the slice really is inside `source`, so the
/// longer lifetime is the true one.
fn borrow_static(source: &'static str, slice: &str) -> &'static str {
    let start = slice.as_ptr() as usize - source.as_ptr() as usize;
    &source[start..start + slice.len()]
}

/// Parse a map file.
///
/// # Errors
/// On an unknown directive, a malformed number, a block id without its
/// namespace, a file with no spawn points, or a kill plane above them.
pub fn parse(source: &'static str) -> Result<MapSpec, ParseError> {
    let mut name = "";
    let mut author = "";
    let mut kill_y = None;
    let mut spawns = Vec::new();
    let mut crystals = Vec::new();
    let mut brushes = Vec::new();

    for (index, raw) in source.lines().enumerate() {
        let line = index + 1;
        let text = raw.split('#').next().unwrap_or("").trim();
        if text.is_empty() {
            continue;
        }

        let mut words = text.split_whitespace();
        let Some(directive) = words.next() else {
            continue;
        };
        let mut cursor = Cursor { words, line };

        match directive {
            // Free text to the end of the line, so a map can be called
            // "Sky Fortress" without needing quotes.
            "name" | "author" => {
                let rest = text[directive.len()..].trim();
                if rest.is_empty() {
                    return Err(cursor.missing(directive));
                }
                let value = borrow_static(source, rest);
                if directive == "name" {
                    name = value;
                } else {
                    author = value;
                }
                continue;
            }
            "kill_y" => kill_y = Some(cursor.float("kill_y")?),
            "spawn" | "crystal" => {
                let x = cursor.float("an x")?;
                let y = cursor.float("a y")?;
                let z = cursor.float("a z")?;
                let at = Vec3::new(x, y, z);
                if directive == "spawn" {
                    spawns.push(at);
                } else {
                    crystals.push(at);
                }
            }
            "box" => {
                let min = cursor.triple("a box corner")?;
                let max = cursor.triple("a box corner")?;
                let block = cursor.block(source)?;
                brushes.push(Brush::Box { min, max, block });
            }
            "cylinder" => {
                let centre = cursor.triple("a cylinder centre")?;
                let radius = cursor.int("a cylinder radius")?;
                let height = cursor.int("a cylinder height")?;
                let block = cursor.block(source)?;
                brushes.push(Brush::Cylinder {
                    centre,
                    radius,
                    height,
                    block,
                });
            }
            "sphere" => {
                let centre = cursor.triple("a sphere centre")?;
                let radius = cursor.int("a sphere radius")?;
                let block = cursor.block(source)?;
                brushes.push(Brush::Sphere {
                    centre,
                    radius,
                    block,
                });
            }
            "cone" => {
                let centre = cursor.triple("a cone centre")?;
                let radius = cursor.int("a cone radius")?;
                let depth = cursor.int("a cone depth")?;
                let block = cursor.block(source)?;
                brushes.push(Brush::Cone {
                    centre,
                    radius,
                    depth,
                    block,
                });
            }
            other => {
                return Err(ParseError {
                    line,
                    reason: format!("unknown directive {other:?}"),
                });
            }
        }
        cursor.end()?;
    }

    if name.is_empty() {
        return Err(ParseError {
            line: 0,
            reason: "no `name`".to_owned(),
        });
    }
    if spawns.is_empty() {
        return Err(ParseError {
            line: 0,
            reason: "no `spawn` points, so nobody could stand on it".to_owned(),
        });
    }
    let Some(kill_y) = kill_y else {
        return Err(ParseError {
            line: 0,
            reason: "no `kill_y`, so the map would have no death plane".to_owned(),
        });
    };
    // A kill plane above the lowest spawn is the bug that made the downloaded
    // world unplayable: everyone died the instant they were placed on it.
    if let Some(lowest) = spawns.iter().map(|spawn| spawn.y).reduce(f32::min)
        && kill_y >= lowest
    {
        return Err(ParseError {
            line: 0,
            reason: format!("kill_y {kill_y} is at or above the lowest spawn {lowest}"),
        });
    }

    Ok(MapSpec {
        name,
        author,
        kill_y,
        spawns,
        crystals,
        brushes,
    })
}

impl Brush {
    /// Call `place` for every block this brush covers.
    ///
    /// Iteration rather than a returned `Vec` because a map is a few hundred
    /// thousand blocks and none of them need to exist at once.
    pub fn each_block(self, mut place: impl FnMut([i32; 3], &'static str)) {
        match self {
            Self::Box { min, max, block } => {
                for x in min[0].min(max[0])..=min[0].max(max[0]) {
                    for y in min[1].min(max[1])..=min[1].max(max[1]) {
                        for z in min[2].min(max[2])..=min[2].max(max[2]) {
                            place([x, y, z], block);
                        }
                    }
                }
            }
            Self::Cylinder {
                centre,
                radius,
                height,
                block,
            } => {
                let squared = radius * radius;
                for dx in -radius..=radius {
                    for dz in -radius..=radius {
                        if dx * dx + dz * dz > squared {
                            continue;
                        }
                        for dy in 0..height {
                            place([centre[0] + dx, centre[1] + dy, centre[2] + dz], block);
                        }
                    }
                }
            }
            Self::Sphere {
                centre,
                radius,
                block,
            } => {
                let squared = radius * radius;
                for dx in -radius..=radius {
                    for dy in -radius..=radius {
                        for dz in -radius..=radius {
                            if dx * dx + dy * dy + dz * dz > squared {
                                continue;
                            }
                            place([centre[0] + dx, centre[1] + dy, centre[2] + dz], block);
                        }
                    }
                }
            }
            Self::Cone {
                centre,
                radius,
                depth,
                block,
            } => {
                for level in 0..depth {
                    // Linear taper: full radius at the top, a single block at
                    // the bottom.
                    let shrunk = radius - (radius * level) / depth.max(1);
                    let squared = shrunk * shrunk;
                    for dx in -shrunk..=shrunk {
                        for dz in -shrunk..=shrunk {
                            if dx * dx + dz * dz > squared {
                                continue;
                            }
                            place([centre[0] + dx, centre[1] - level, centre[2] + dz], block);
                        }
                    }
                }
            }
        }
    }
}

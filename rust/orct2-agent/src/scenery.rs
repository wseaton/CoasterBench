//! Park decoration: the scenery objects a loaded park can place, and the
//! decorations a model asks for, resolved against that catalog.
//!
//! Decorations are presentation. They are applied to the park a round has
//! already banked, after its ratings were measured, so nothing here can move
//! a score.

use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};

use crate::host;
use crate::presentation::COLOURS;

/// Tiles one decoration may cover. A rectangle is a convenience for lawns,
/// plazas and paths, not a way to carpet the map in one call.
pub const MAX_TILES_PER_DECORATION: i64 = 400;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    Small,
    Large,
    Wall,
    PathSurface,
    PathRailings,
}

impl Kind {
    pub fn parse(name: &str) -> Option<Kind> {
        match name {
            "small" => Some(Kind::Small),
            "large" => Some(Kind::Large),
            "wall" => Some(Kind::Wall),
            "path_surface" => Some(Kind::PathSurface),
            "path_railings" => Some(Kind::PathRailings),
            _ => None,
        }
    }
}

/// One placeable object. `details` is whatever else the host said about it
/// (footprint, height, how many colours it takes), passed through to the
/// model untouched.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct CatalogEntry {
    pub kind: Kind,
    pub index: u16,
    pub id: String,
    pub name: String,
    #[serde(flatten)]
    pub details: Map<String, Value>,
}

/// Loads the catalog from the host. An error means the host had none to give.
pub fn load() -> Result<Vec<CatalogEntry>, String> {
    let json = host::scenery_catalog_json().ok_or("scenery catalog unavailable")?;
    parse_catalog(&json)
}

fn parse_catalog(json: &str) -> Result<Vec<CatalogEntry>, String> {
    serde_json::from_str(json).map_err(|e| format!("scenery catalog parse: {e}"))
}

/// Catalog entries of one kind and/or matching a case-insensitive substring
/// of the name or id.
pub fn search<'a>(
    catalog: &'a [CatalogEntry],
    kind: Option<Kind>,
    query: Option<&str>,
) -> Vec<&'a CatalogEntry> {
    let query = query.map(str::to_lowercase);
    catalog
        .iter()
        .filter(|entry| kind.is_none_or(|kind| entry.kind == kind))
        .filter(|entry| {
            query.as_deref().is_none_or(|q| {
                entry.name.to_lowercase().contains(q) || entry.id.to_lowercase().contains(q)
            })
        })
        .collect()
}

fn find<'a>(catalog: &'a [CatalogEntry], id: &str) -> Result<&'a CatalogEntry, String> {
    catalog
        .iter()
        .find(|entry| entry.id == id)
        .ok_or_else(|| format!("unknown object {id:?}; use scenery_catalog for valid ids"))
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Placement {
    Scenery {
        kind: u8,
        index: u16,
        z: i32,
        direction: u8,
        quadrant: u8,
        colours: [u8; 3],
    },
    Path {
        surface: u16,
        railings: u16,
    },
}

/// One requested decoration, resolved against the catalog: an object and the
/// rectangle of tiles to put it on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Decoration {
    placement: Placement,
    x: (i32, i32),
    y: (i32, i32),
}

/// What became of one decoration: how many of its tiles took the object.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Outcome {
    pub placed: u32,
    pub rejected: u32,
    /// The game's reason for the first tile it refused.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub first_error: Option<String>,
}

fn int(item: &Value, key: &str) -> Result<Option<i64>, String> {
    match item.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(value) => value
            .as_i64()
            .map(Some)
            .ok_or_else(|| format!("{key} must be an integer")),
    }
}

fn required_tile(item: &Value, key: &str) -> Result<i32, String> {
    let value = int(item, key)?.ok_or_else(|| format!("{key} required"))?;
    i32::try_from(value).map_err(|_| format!("{key} out of range"))
}

fn span(item: &Value, from: &str, to: &str) -> Result<(i32, i32), String> {
    let a = required_tile(item, from)?;
    let b = match int(item, to)? {
        Some(value) => i32::try_from(value).map_err(|_| format!("{to} out of range"))?,
        None => a,
    };
    Ok((a.min(b), a.max(b)))
}

fn quarter_turn(item: &Value, key: &str) -> Result<u8, String> {
    match int(item, key)? {
        None => Ok(0),
        Some(value @ 0..=3) => Ok(value as u8),
        Some(_) => Err(format!("{key} must be 0-3")),
    }
}

fn colours(item: &Value) -> Result<[u8; 3], String> {
    let mut out = [0u8; 3];
    let Some(value) = item.get("colours").filter(|v| !v.is_null()) else {
        return Ok(out);
    };
    let names = value.as_array().ok_or("colours must be an array")?;
    if names.len() > out.len() {
        return Err("at most three colours".into());
    }
    for (slot, name) in out.iter_mut().zip(names) {
        let name = name.as_str().ok_or("colours must be colour names")?;
        let index = COLOURS
            .iter()
            .position(|known| *known == name)
            .ok_or_else(|| format!("unknown colour {name:?}"))?;
        *slot = index as u8;
    }
    Ok(out)
}

impl Decoration {
    /// Resolves one requested decoration. Everything the game cannot judge
    /// (unknown ids, malformed fields, an oversized rectangle) is refused
    /// here; whether a tile can actually take the object is the game's call.
    pub fn parse(item: &Value, catalog: &[CatalogEntry]) -> Result<Decoration, String> {
        let id = item
            .get("object")
            .and_then(Value::as_str)
            .ok_or("object required")?;
        let entry = find(catalog, id)?;
        let x = span(item, "x", "x2")?;
        let y = span(item, "y", "y2")?;
        let tiles = (i64::from(x.1) - i64::from(x.0) + 1) * (i64::from(y.1) - i64::from(y.0) + 1);
        if tiles > MAX_TILES_PER_DECORATION {
            return Err(format!(
                "rectangle covers {tiles} tiles; at most {MAX_TILES_PER_DECORATION} per decoration"
            ));
        }
        let placement = match entry.kind {
            Kind::PathSurface => {
                let railings = match item.get("railings").and_then(Value::as_str) {
                    Some(id) => {
                        let railings = find(catalog, id)?;
                        if railings.kind != Kind::PathRailings {
                            return Err(format!("{id:?} is not a path_railings object"));
                        }
                        railings.index
                    }
                    None => catalog
                        .iter()
                        .find(|entry| entry.kind == Kind::PathRailings)
                        .map(|entry| entry.index)
                        .ok_or("this park has no path railings loaded")?,
                };
                Placement::Path {
                    surface: entry.index,
                    railings,
                }
            }
            Kind::PathRailings => {
                return Err(format!(
                    "{id:?} is path railings; place a path_surface and name this as its railings"
                ));
            }
            Kind::Small | Kind::Large | Kind::Wall => {
                let z = match int(item, "z")? {
                    None => 0,
                    Some(value) => i32::try_from(value)
                        .ok()
                        .filter(|z| *z >= 0)
                        .ok_or("z out of range")?,
                };
                Placement::Scenery {
                    kind: match entry.kind {
                        Kind::Small => 0,
                        Kind::Large => 1,
                        _ => 2,
                    },
                    index: entry.index,
                    z,
                    direction: quarter_turn(item, "dir")?,
                    quadrant: quarter_turn(item, "quadrant")?,
                    colours: colours(item)?,
                }
            }
        };
        Ok(Decoration { placement, x, y })
    }

    fn place_at(&self, tile_x: i32, tile_y: i32) -> Result<(), String> {
        match self.placement {
            Placement::Scenery {
                kind,
                index,
                z,
                direction,
                quadrant,
                colours,
            } => host::scenery_place(&host::SceneryPlacement {
                kind,
                index,
                tile_x,
                tile_y,
                z,
                direction,
                quadrant,
                colours,
            }),
            Placement::Path { surface, railings } => {
                host::footpath_place(tile_x, tile_y, surface, railings)
            }
        }
    }

    /// Places the object on every tile of the rectangle. A refused tile does
    /// not stop the rest: track, water or a slope in the middle of a lawn
    /// should leave a gap, not abandon the lawn.
    pub fn apply(&self) -> Outcome {
        let mut outcome = Outcome {
            placed: 0,
            rejected: 0,
            first_error: None,
        };
        for tile_y in self.y.0..=self.y.1 {
            for tile_x in self.x.0..=self.x.1 {
                match self.place_at(tile_x, tile_y) {
                    Ok(()) => outcome.placed += 1,
                    Err(error) => {
                        outcome.rejected += 1;
                        outcome.first_error.get_or_insert(error);
                    }
                }
            }
        }
        outcome
    }
}

/// Tiles kept clear around decorations when framing them: a large object
/// extends past the tile it is anchored on.
const FRAME_MARGIN_TILES: i32 = 2;
/// Headroom above the track for trees and towers standing beside it.
const FRAME_HEADROOM: i32 = 96;

impl Decoration {
    /// `frame` grown to hold this decoration's rectangle.
    pub fn widen(&self, frame: host::TrackBounds) -> host::TrackBounds {
        frame.union(host::TrackBounds {
            min_tile_x: (self.x.0 - FRAME_MARGIN_TILES).max(0),
            min_tile_y: (self.y.0 - FRAME_MARGIN_TILES).max(0),
            max_tile_x: self.x.1 + FRAME_MARGIN_TILES,
            max_tile_y: self.y.1 + FRAME_MARGIN_TILES,
            min_z: frame.min_z,
            max_z: frame.max_z.max(frame.min_z + FRAME_HEADROOM),
        })
    }
}

/// The record a report keeps of one decoration: the request as the model
/// wrote it, and what the game made of it.
pub fn record(item: &Value, outcome: &Outcome) -> Value {
    json!({"item": item, "placed": outcome.placed, "rejected": outcome.rejected})
}

#[cfg(test)]
mod tests {
    use crate::scenery::*;

    fn catalog() -> Vec<CatalogEntry> {
        parse_catalog(
            r#"[
            {"kind": "small", "index": 4, "id": "rct2.scenery_small.tl0", "name": "Oak Tree",
             "footprint": "full", "height": 120, "colours": 0, "tree": true},
            {"kind": "small", "index": 9, "id": "rct2.scenery_small.tg1", "name": "Tulips",
             "footprint": "quarter", "height": 16, "colours": 1},
            {"kind": "large", "index": 2, "id": "rct2.scenery_large.scol", "name": "Roman Colosseum",
             "tiles": [[0, 0], [1, 0]], "colours": 0},
            {"kind": "wall", "index": 7, "id": "rct2.scenery_wall.wmf", "name": "Mesh Fence",
             "height": 2, "colours": 1},
            {"kind": "path_surface", "index": 1, "id": "rct2.footpath_surface.tarmac", "name": "Tarmac"},
            {"kind": "path_railings", "index": 3, "id": "rct2.footpath_railings.wood", "name": "Wooden"},
            {"kind": "path_railings", "index": 5, "id": "rct2.footpath_railings.concrete", "name": "Concrete"}
        ]"#,
        )
        .expect("catalog")
    }

    #[test]
    fn catalog_keeps_the_hosts_extra_fields() {
        let catalog = catalog();
        assert_eq!(catalog.len(), 7);
        assert_eq!(catalog[0].kind, Kind::Small);
        assert_eq!(catalog[0].details["footprint"], json!("full"));
        let back = serde_json::to_value(&catalog[0]).expect("serialise");
        assert_eq!(back["id"], json!("rct2.scenery_small.tl0"));
        assert_eq!(back["tree"], json!(true));
        assert_eq!(back["kind"], json!("small"));
    }

    #[test]
    fn a_malformed_catalog_is_an_error_and_so_is_a_missing_host() {
        assert!(parse_catalog("{").is_err());
        assert!(
            parse_catalog(r#"[{"kind": "banner", "index": 0, "id": "a", "name": "b"}]"#).is_err()
        );
        // The test stub host returns null for the catalog.
        assert!(load().is_err());
    }

    #[test]
    fn search_filters_by_kind_and_by_name_or_id() {
        let catalog = catalog();
        assert_eq!(search(&catalog, None, None).len(), 7);
        assert_eq!(search(&catalog, Some(Kind::Small), None).len(), 2);
        let trees = search(&catalog, None, Some("TREE"));
        assert_eq!(trees.len(), 1);
        assert_eq!(trees[0].name, "Oak Tree");
        assert_eq!(search(&catalog, None, Some("footpath_railings")).len(), 2);
        assert!(search(&catalog, Some(Kind::Wall), Some("tree")).is_empty());
        for name in ["small", "large", "wall", "path_surface", "path_railings"] {
            let kind = Kind::parse(name).expect(name);
            assert_eq!(serde_json::to_value(kind).expect("kind"), json!(name));
        }
        assert_eq!(Kind::parse("banner"), None);
    }

    #[test]
    fn scenery_resolves_with_defaults_and_with_every_field() {
        let catalog = catalog();
        let plain = Decoration::parse(
            &json!({"object": "rct2.scenery_small.tl0", "x": 40, "y": 50}),
            &catalog,
        )
        .expect("plain");
        assert_eq!(
            plain,
            Decoration {
                placement: Placement::Scenery {
                    kind: 0,
                    index: 4,
                    z: 0,
                    direction: 0,
                    quadrant: 0,
                    colours: [0, 0, 0],
                },
                x: (40, 40),
                y: (50, 50),
            }
        );
        let full = Decoration::parse(
            &json!({"object": "rct2.scenery_wall.wmf", "x": 9, "y": 8, "x2": 5, "y2": 8, "z": 144,
                    "dir": 3, "quadrant": 2, "colours": ["white", "bright_red"]}),
            &catalog,
        )
        .expect("full");
        assert_eq!(
            full,
            Decoration {
                placement: Placement::Scenery {
                    kind: 2,
                    index: 7,
                    z: 144,
                    direction: 3,
                    quadrant: 2,
                    colours: [2, 28, 0],
                },
                x: (5, 9),
                y: (8, 8),
            }
        );
        let large = Decoration::parse(
            &json!({"object": "rct2.scenery_large.scol", "x": 1, "y": 1}),
            &catalog,
        )
        .expect("large");
        assert!(matches!(
            large.placement,
            Placement::Scenery {
                kind: 1,
                index: 2,
                ..
            }
        ));
    }

    #[test]
    fn paths_take_named_railings_or_the_first_loaded() {
        let catalog = catalog();
        let default = Decoration::parse(
            &json!({"object": "rct2.footpath_surface.tarmac", "x": 1, "y": 1, "x2": 6}),
            &catalog,
        )
        .expect("default railings");
        assert_eq!(
            default.placement,
            Placement::Path {
                surface: 1,
                railings: 3
            }
        );
        assert_eq!(default.x, (1, 6));
        let named = Decoration::parse(
            &json!({"object": "rct2.footpath_surface.tarmac", "x": 1, "y": 1,
                    "railings": "rct2.footpath_railings.concrete"}),
            &catalog,
        )
        .expect("named railings");
        assert_eq!(
            named.placement,
            Placement::Path {
                surface: 1,
                railings: 5
            }
        );

        let no_railings: Vec<CatalogEntry> = catalog
            .iter()
            .filter(|entry| entry.kind != Kind::PathRailings)
            .cloned()
            .collect();
        let err = Decoration::parse(
            &json!({"object": "rct2.footpath_surface.tarmac", "x": 1, "y": 1}),
            &no_railings,
        )
        .expect_err("no railings loaded");
        assert!(err.contains("no path railings"), "{err}");
    }

    #[test]
    fn malformed_decorations_are_refused_before_the_game_sees_them() {
        let catalog = catalog();
        let refuse = |item: Value, needle: &str| {
            let err = Decoration::parse(&item, &catalog).expect_err(needle);
            assert!(err.contains(needle), "{err} should mention {needle}");
        };
        refuse(json!({"x": 1, "y": 1}), "object required");
        refuse(json!({"object": "nope", "x": 1, "y": 1}), "unknown object");
        refuse(
            json!({"object": "rct2.scenery_small.tl0", "y": 1}),
            "x required",
        );
        refuse(
            json!({"object": "rct2.scenery_small.tl0", "x": 1}),
            "y required",
        );
        refuse(
            json!({"object": "rct2.scenery_small.tl0", "x": "1", "y": 1}),
            "x must be an integer",
        );
        refuse(
            json!({"object": "rct2.scenery_small.tl0", "x": 1, "y": 1, "dir": 4}),
            "dir must be 0-3",
        );
        refuse(
            json!({"object": "rct2.scenery_small.tl0", "x": 1, "y": 1, "quadrant": -1}),
            "quadrant must be 0-3",
        );
        refuse(
            json!({"object": "rct2.scenery_small.tl0", "x": 1, "y": 1, "z": -16}),
            "z out of range",
        );
        refuse(
            json!({"object": "rct2.scenery_small.tl0", "x": 1, "y": 1, "colours": ["mauve"]}),
            "unknown colour",
        );
        refuse(
            json!({"object": "rct2.scenery_small.tl0", "x": 1, "y": 1,
                   "colours": ["white", "white", "white", "white"]}),
            "at most three colours",
        );
        refuse(
            json!({"object": "rct2.scenery_small.tl0", "x": 1, "y": 1, "colours": "white"}),
            "colours must be an array",
        );
        refuse(
            json!({"object": "rct2.footpath_railings.wood", "x": 1, "y": 1}),
            "is path railings",
        );
        refuse(
            json!({"object": "rct2.footpath_surface.tarmac", "x": 1, "y": 1,
                   "railings": "rct2.scenery_small.tl0"}),
            "not a path_railings object",
        );
    }

    #[test]
    fn a_rectangle_is_capped_by_area() {
        let catalog = catalog();
        let item = |x2: i64, y2: i64| json!({"object": "rct2.scenery_small.tl0", "x": 0, "y": 0, "x2": x2, "y2": y2});
        assert!(Decoration::parse(&item(19, 19), &catalog).is_ok());
        let err = Decoration::parse(&item(20, 19), &catalog).expect_err("420 tiles");
        assert!(err.contains("420 tiles"), "{err}");
        assert!(Decoration::parse(&item(i64::from(i32::MAX), 0), &catalog).is_err());
    }

    #[test]
    fn the_frame_grows_to_hold_a_decoration_and_never_shrinks() {
        let catalog = catalog();
        let track = host::TrackBounds {
            min_tile_x: 40,
            min_tile_y: 40,
            max_tile_x: 60,
            max_tile_y: 60,
            min_z: 112,
            max_z: 400,
        };
        let outside = Decoration::parse(
            &json!({"object": "rct2.scenery_small.tl0", "x": 1, "y": 70, "x2": 30, "y2": 75}),
            &catalog,
        )
        .expect("outside");
        assert_eq!(
            outside.widen(track),
            host::TrackBounds {
                min_tile_x: 0,
                min_tile_y: 40,
                max_tile_x: 60,
                max_tile_y: 77,
                min_z: 112,
                max_z: 400,
            }
        );
        let inside = Decoration::parse(
            &json!({"object": "rct2.scenery_small.tl0", "x": 50, "y": 50}),
            &catalog,
        )
        .expect("inside");
        assert_eq!(inside.widen(track), track);

        let low = host::TrackBounds {
            max_z: 128,
            ..track
        };
        assert_eq!(
            inside.widen(low).max_z,
            208,
            "room for a tree beside flat track"
        );
    }

    #[test]
    fn every_tile_is_attempted_and_the_first_refusal_is_kept() {
        // The test stub host refuses every placement with an empty message.
        let catalog = catalog();
        let item = json!({"object": "rct2.scenery_small.tl0", "x": 2, "y": 2, "x2": 4, "y2": 3});
        let outcome = Decoration::parse(&item, &catalog).expect("parse").apply();
        assert_eq!(outcome.placed, 0);
        assert_eq!(outcome.rejected, 6);
        assert!(outcome.first_error.is_some());
        assert_eq!(
            record(&item, &outcome),
            json!({"item": item, "placed": 0, "rejected": 6})
        );
    }
}

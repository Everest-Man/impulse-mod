//! A game's interactive map: tiles, pins and how the game's coordinates lie on it.

use crate::{Ref, Text};

/// A map a source has for a game, before it is opened.
#[derive(Clone, Debug, PartialEq)]
pub struct MapOffer {
    pub id: Ref,
    pub name: String,
    pub thumbnail: Option<String>,
    /// Changes whenever the map does.
    pub revision: Option<String>,
}

/// A point of the unit square the whole tile pyramid covers; y grows down.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Point {
    pub x: f64,
    pub y: f64,
}

pub struct Map {
    /// Never empty: the first shows until the user chooses another.
    pub tile_sets: Vec<TileSet>,
    /// RGB behind the tiles.
    pub background: u32,
    pub categories: Vec<Category>,
    pub pins: Vec<Pin>,
    pub regions: Vec<Region>,
    pub labels: Vec<Label>,
    /// `None`: the map cannot place the player.
    pub frame: Option<Frame>,
    pub teleport: bool,
    /// Still being worked on: pins may be missing or move.
    pub unfinished: bool,
}

impl Map {
    /// The tile set `chosen`, else the first, with its index.
    pub fn tile_set(&self, chosen: Option<&str>) -> (usize, &TileSet) {
        let index = chosen.and_then(|id| self.tile_sets.iter().position(|set| set.id == id)).unwrap_or(0);
        (index, &self.tile_sets[index])
    }
}

/// One look of the map's ground: satellite, drawn, a season.
#[derive(Debug, PartialEq)]
pub struct TileSet {
    /// Stable across revisions: the user's choice refers to it.
    pub id: String,
    pub name: Text,
    pub tiles: Tiles,
}

#[derive(Debug, PartialEq)]
pub struct Tiles {
    /// A URL with `{z}`, `{x}` and `{y}`.
    pub template: String,
    /// Rows count from the bottom.
    pub flipped: bool,
    /// Pixels along a tile's side.
    pub size: u32,
    /// `min_zoom <= max_zoom <= MAX_ZOOM`.
    pub min_zoom: u8,
    pub max_zoom: u8,
    /// The part of the plane the tiles cover, top left and bottom right, the bottom right edges excluded;
    /// `None`: all of it.
    pub bounds: Option<[Point; 2]>,
}

impl Tiles {
    /// The deepest level a map may have: a level's columns, and two levels under it, count in a `u32`.
    pub const MAX_ZOOM: u8 = 24;

    /// `column` and `row` count from the top left, below `2^zoom`.
    pub fn url(&self, zoom: u8, column: u32, row: u32) -> String {
        let row = if self.flipped { (1u32 << zoom) - 1 - row } else { row };
        self.template.replace("{z}", &zoom.to_string()).replace("{x}", &column.to_string()).replace("{y}", &row.to_string())
    }

    /// Whether the tile is one to ask for: its level is served and it meets the bounds.
    pub fn covers(&self, zoom: u8, column: u32, row: u32) -> bool {
        let count = f64::from(1u32 << zoom);
        let meets = |[low, high]: [Point; 2]| {
            let (left, top) = (f64::from(column) / count, f64::from(row) / count);
            left < high.x && left + 1. / count > low.x && top < high.y && top + 1. / count > low.y
        };
        (self.min_zoom..=self.max_zoom).contains(&zoom) && self.bounds.is_none_or(meets)
    }
}

pub struct Category {
    pub name: String,
    pub color: u32,
    /// Whether its pins show before the user chooses.
    pub shown: bool,
    pub groups: Vec<Group>,
}

pub struct Group {
    /// Stable across revisions of the map.
    pub id: String,
    pub name: String,
    pub icon: Option<Icon>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Icon {
    /// Tinted to read on its marker.
    Glyph(Glyph),
    /// Drawn as it is over the marker's head (selected: its shape in the mark's ink): a symbol, never a whole pin.
    /// The whole picture, or a cell of a sheet of icons (left, top, width, height in its pixels).
    Image { url: String, cell: Option<[u32; 4]> },
}

/// An icon's outline: SVG path data fitted and centered in the unit square, y down.
#[derive(Clone, Debug, PartialEq)]
pub struct Glyph {
    pub path: String,
}

/// Where a pin lies against the ground the map shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Level {
    Above,
    Below,
}

/// How a description links to another pin of its map: this, then the pin's id.
pub const PIN_LINK: &str = "pin:";

/// A picture's URLs: one sized for a card, one to see it whole.
#[derive(Clone, Debug, PartialEq)]
pub struct Picture {
    pub preview: String,
    pub full: String,
}

pub struct Pin {
    /// Stable across revisions of the map.
    pub id: String,
    pub at: Point,
    pub name: String,
    /// Markdown; it links to other pins of the map with `PIN_LINK`.
    pub description: String,
    /// Indexes of its category and of the group in it.
    pub group: (usize, usize),
    pub image: Option<Picture>,
    pub video: Option<String>,
    /// Where it is in the game's own coordinates.
    pub position: Option<[f64; 3]>,
    pub level: Option<Level>,
}

pub struct Region {
    /// Stable across revisions: the user's state refers to it.
    pub id: String,
    pub name: String,
    pub color: u32,
    pub thickness: f32,
    /// One outer ring per polygon.
    pub outlines: Vec<Vec<Point>>,
    /// The zoom levels it shows at, inclusive.
    pub zoom: (f32, f32),
}

pub struct Label {
    pub text: String,
    pub color: u32,
    pub size: f32,
    pub at: Point,
    pub zoom: (f32, f32),
}

/// How the game's coordinates lie on the map plane.
#[derive(Clone, Debug, PartialEq)]
pub struct Frame {
    /// The game axes that run along the plane's x, along its y, and up: a permutation of 0, 1, 2.
    pub axes: [usize; 3],
    /// Where the game's zero lies.
    pub origin: Point,
    /// Plane units per game unit along x and y; never zero.
    pub scale: (f64, f64),
}

impl Frame {
    pub fn to_map(&self, position: [f64; 3]) -> Point {
        Point { x: self.origin.x + position[self.axes[0]] * self.scale.0, y: self.origin.y + position[self.axes[1]] * self.scale.1 }
    }

    /// `height` is the game's up coordinate, which the plane does not hold.
    pub fn to_game(&self, at: Point, height: f64) -> [f64; 3] {
        let mut position = [0.; 3];
        position[self.axes[0]] = (at.x - self.origin.x) / self.scale.0;
        position[self.axes[1]] = (at.y - self.origin.y) / self.scale.1;
        position[self.axes[2]] = height;
        position
    }

    pub fn height(&self, position: [f64; 3]) -> f64 {
        position[self.axes[2]]
    }

    /// `position`, `by` game units higher.
    pub fn raised(&self, mut position: [f64; 3], by: f64) -> [f64; 3] {
        position[self.axes[2]] += by;
        position
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tile_urls_follow_the_template_and_the_row_order() {
        let mut tiles =
            Tiles { template: "https://cdn/{z}/{y}/{x}.webp".into(), flipped: false, size: 256, min_zoom: 0, max_zoom: 8, bounds: None };
        assert_eq!(tiles.url(3, 5, 1), "https://cdn/3/1/5.webp");
        tiles.flipped = true;
        assert_eq!(tiles.url(3, 5, 1), "https://cdn/3/6/5.webp");
        assert_eq!(tiles.url(0, 0, 0), "https://cdn/0/0/0.webp");
    }

    #[test]
    fn tiles_are_asked_within_the_levels_and_the_bounds() {
        let at = |x, y| Point { x, y };
        let bounds = Some([at(0.25, 0.5), at(0.5, 0.75)]);
        let tiles = Tiles { template: String::new(), flipped: false, size: 256, min_zoom: 1, max_zoom: 3, bounds };
        assert!(!tiles.covers(0, 0, 0) && !tiles.covers(4, 5, 9), "levels the provider does not serve");
        assert!(tiles.covers(1, 0, 1) && tiles.covers(2, 1, 2));
        // The right and bottom edges are outside.
        assert!(!tiles.covers(2, 2, 2) && !tiles.covers(2, 1, 3) && !tiles.covers(2, 0, 2));
        assert!(Tiles { bounds: None, ..tiles }.covers(3, 7, 0));
    }

    #[test]
    fn an_unknown_tile_set_falls_back_to_the_first() {
        let set = |id: &str| TileSet {
            id: id.into(),
            name: Text::Plain(id.into()),
            tiles: Tiles { template: String::new(), flipped: false, size: 256, min_zoom: 0, max_zoom: 0, bounds: None },
        };
        let map = Map {
            tile_sets: vec![set("sat"), set("drawn")],
            background: 0,
            categories: Vec::new(),
            pins: Vec::new(),
            regions: Vec::new(),
            labels: Vec::new(),
            frame: None,
            teleport: false,
            unfinished: false,
        };
        assert_eq!(map.tile_set(Some("drawn")).0, 1);
        assert_eq!((map.tile_set(Some("gone")).0, map.tile_set(None).0), (0, 0));
    }

    #[test]
    fn a_frame_maps_both_ways_through_swapped_axes() {
        let frame = Frame { axes: [2, 0, 1], origin: Point { x: 0.5, y: 0.25 }, scale: (0.001, -0.002) };
        let position = [40., 7., -100.];
        let at = frame.to_map(position);
        assert!((at.x - 0.4).abs() < 1e-12 && (at.y - 0.17).abs() < 1e-12, "{at:?}");
        assert_eq!(frame.height(position), 7.);
        let back = frame.to_game(at, 7.);
        assert!(back.iter().zip(position).all(|(got, want)| (got - want).abs() < 1e-9), "{back:?}");
    }
}

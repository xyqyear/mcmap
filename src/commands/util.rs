// Small helpers shared across commands: region filename parsing, bounds of
// a set of region coordinates.

use crate::anvil::RCoord;

#[derive(Debug)]
pub struct Rectangle {
    pub xmin: RCoord,
    pub xmax: RCoord,
    pub zmin: RCoord,
    pub zmax: RCoord,
}

/// Parse `r.<x>.<z>.mca` into its integer coordinates. Returns `None` for any
/// other filename shape (including dimension-scoped names like
/// `r.0.0.mca.tmp`).
pub fn parse_region_filename(filename: &str) -> Option<(RCoord, RCoord)> {
    let parts: Vec<&str> = filename.split('.').collect();
    if parts.len() != 4 || parts[0] != "r" || parts[3] != "mca" {
        return None;
    }
    let x: isize = parts[1].parse().ok()?;
    let z: isize = parts[2].parse().ok()?;
    Some((RCoord(x), RCoord(z)))
}

/// Tight bounding box around a set of region coords. `xmax` / `zmax` are
/// returned exclusive (one past the max coord) so callers can use them
/// directly as a Rust range end.
pub fn auto_size(coords: &[(RCoord, RCoord)]) -> Option<Rectangle> {
    if coords.is_empty() {
        return None;
    }
    let mut bounds = Rectangle {
        xmin: RCoord(isize::MAX),
        zmin: RCoord(isize::MAX),
        xmax: RCoord(isize::MIN),
        zmax: RCoord(isize::MIN),
    };
    for coord in coords {
        bounds.xmin = std::cmp::min(bounds.xmin, coord.0);
        bounds.xmax = std::cmp::max(bounds.xmax, coord.0);
        bounds.zmin = std::cmp::min(bounds.zmin, coord.1);
        bounds.zmax = std::cmp::max(bounds.zmax, coord.1);
    }
    // Exclusive upper bound.
    bounds.xmax = RCoord(bounds.xmax.0 + 1);
    bounds.zmax = RCoord(bounds.zmax.0 + 1);
    Some(bounds)
}

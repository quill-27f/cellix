//! Interface icons. A font glyph is one cell and looks tiny beside a real IDE, so icons
//! span several cells: the pixel tier draws them as shapes, and the cell tier as block art.

/// Base of the Unicode private plane used to carry icon pieces through the grid.
const BASE: u32 = 0xF0000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Icon {
    Files = 0,
    Search = 1,
    Git = 2,
    Settings = 3,
}

/// One cell of an icon that is `width` by `height` cells.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Piece {
    pub icon: Icon,
    pub width: u8,
    pub height: u8,
    pub column: u8,
    pub row: u8,
}

impl Piece {
    pub fn symbol(self) -> char {
        let code = (self.icon as u32) << 8
            | u32::from(self.width) << 6
            | u32::from(self.height) << 4
            | u32::from(self.column) << 2
            | u32::from(self.row);
        char::from_u32(BASE + code).expect("the private plane holds every code")
    }

    pub fn decode(symbol: &str) -> Option<Self> {
        let mut chars = symbol.chars();
        let code = (chars.next()? as u32).checked_sub(BASE)?;
        if chars.next().is_some() || code >= 1 << 10 {
            return None;
        }
        let icon = match code >> 8 {
            0 => Icon::Files,
            1 => Icon::Search,
            2 => Icon::Git,
            _ => Icon::Settings,
        };
        Some(Self {
            icon,
            width: (code >> 6 & 3) as u8,
            height: (code >> 4 & 3) as u8,
            column: (code >> 2 & 3) as u8,
            row: (code & 3) as u8,
        })
    }

    /// What the cell tier shows, since terminals can't draw the private plane.
    pub fn fallback(self) -> &'static str {
        const ART: [[[&str; 3]; 2]; 4] = [
            [["▛", "▀", "▜"], ["▙", "▄", "▟"]],
            [["▞", "▀", "▚"], ["▚", "▄", "▟"]],
            [["●", "─", "●"], ["●", "─", "╯"]],
            [["▚", "▀", "▞"], ["▞", "▄", "▚"]],
        ];
        if self.width == 1 {
            return match self.icon {
                Icon::Files => "▤",
                Icon::Search => "⌕",
                Icon::Git => "⎇",
                Icon::Settings => "⚙",
            };
        }
        ART[self.icon as usize][self.row.min(1) as usize][self.column.min(2) as usize]
    }
}

fn segment(p: (f32, f32), a: (f32, f32), b: (f32, f32)) -> f32 {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let t = (((p.0 - a.0) * dx + (p.1 - a.1) * dy) / (dx * dx + dy * dy)).clamp(0.0, 1.0);
    ((p.0 - a.0 - t * dx).powi(2) + (p.1 - a.1 - t * dy).powi(2)).sqrt()
}

fn circle(p: (f32, f32), c: (f32, f32)) -> f32 {
    ((p.0 - c.0).powi(2) + (p.1 - c.1).powi(2)).sqrt()
}

/// Distance to the outline of a box.
fn outline(p: (f32, f32), min: (f32, f32), max: (f32, f32)) -> f32 {
    let (cx, cy) = ((min.0 + max.0) / 2.0, (min.1 + max.1) / 2.0);
    let (qx, qy) = (
        (p.0 - cx).abs() - (max.0 - min.0) / 2.0,
        (p.1 - cy).abs() - (max.1 - min.1) / 2.0,
    );
    let outside = (qx.max(0.0).powi(2) + qy.max(0.0).powi(2)).sqrt();
    (outside + qx.max(qy).min(0.0)).abs()
}

/// Whether a point in the icon's unit square is inked.
pub fn inked(icon: Icon, u: f32, v: f32) -> bool {
    const STROKE: f32 = 0.04;
    let p = (u, v);
    match icon {
        Icon::Files => {
            let (front_min, front_max) = ((0.14, 0.30), (0.68, 0.92));
            let inside_front =
                u > front_min.0 && u < front_max.0 && v > front_min.1 && v < front_max.1;
            outline(p, front_min, front_max) < STROKE
                || (!inside_front && outline(p, (0.34, 0.08), (0.88, 0.70)) < STROKE)
        }
        Icon::Search => {
            (circle(p, (0.42, 0.42)) - 0.28).abs() < STROKE
                || segment(p, (0.63, 0.63), (0.90, 0.90)) < STROKE * 1.3
        }
        Icon::Git => {
            let ring = |c| (circle(p, c) - 0.10).abs() < STROKE;
            ring((0.30, 0.18))
                || ring((0.30, 0.82))
                || ring((0.72, 0.30))
                || segment(p, (0.30, 0.28), (0.30, 0.72)) < STROKE
                || segment(p, (0.72, 0.40), (0.72, 0.48)) < STROKE
                || segment(p, (0.72, 0.48), (0.30, 0.62)) < STROKE
        }
        Icon::Settings => {
            let (x, y) = (u - 0.5, v - 0.5);
            let radius = (x * x + y * y).sqrt();
            let tooth = (y.atan2(x) * 8.0).sin() > 0.0;
            radius > 0.16 && radius < if tooth { 0.44 } else { 0.34 }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pieces_survive_the_grid() {
        for icon in [Icon::Files, Icon::Search, Icon::Git, Icon::Settings] {
            let piece = Piece {
                icon,
                width: 3,
                height: 2,
                column: 2,
                row: 1,
            };
            let text = piece.symbol().to_string();
            assert_eq!(Piece::decode(&text), Some(piece));
        }
        assert_eq!(Piece::decode("a"), None);
    }

    #[test]
    fn every_icon_has_ink() {
        for icon in [Icon::Files, Icon::Search, Icon::Git, Icon::Settings] {
            let dots = (0..50)
                .flat_map(|y| (0..50).map(move |x| (x, y)))
                .filter(|(x, y)| inked(icon, *x as f32 / 50.0, *y as f32 / 50.0))
                .count();
            assert!(dots > 150, "{icon:?} has {dots}");
        }
    }
}

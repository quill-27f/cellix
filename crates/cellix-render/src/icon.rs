//! Interface icons. A font glyph is one cell and looks tiny beside a real IDE, so icons
//! span several cells: the pixel tier draws them as shapes, and the cell tier as block art.

/// Base of the Unicode private plane used to carry icon pieces through the grid.
const BASE: u32 = 0xF0000;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Icon {
    Files = 0,
    Search = 1,
    Git = 2,
    Settings = 3,
    Folder = 4,
    FolderOpen = 5,
    File = 6,
    ChevronRight = 7,
    ChevronDown = 8,
    Close = 9,
    ReplaceAll = 10,
    Globe = 11,
    Check = 12,
    Agents = 13,
    // The document toolbar's, which only ever appear as paint.
    ListBullet = 14,
    ListNumber = 15,
    ListTask = 16,
    Quote = 17,
    Code = 18,
    Link = 19,
    Table = 20,
    Picture = 21,
    Sigma = 22,
    Rule = 23,
    Comment = 24,
    Export = 25,
    Play = 26,
    Cite = 27,
    More = 28,
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
        if chars.next().is_some() || code >= 1 << 12 {
            return None;
        }
        let icon = match code >> 8 {
            0 => Icon::Files,
            1 => Icon::Search,
            2 => Icon::Git,
            13 => Icon::Agents,
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
        const ART: [[[&str; 3]; 2]; 5] = [
            [["▛", "▀", "▜"], ["▙", "▄", "▟"]],
            [["▞", "▀", "▚"], ["▚", "▄", "▟"]],
            [["●", "─", "●"], ["●", "─", "╯"]],
            [["▚", "▀", "▞"], ["▞", "▄", "▚"]],
            [["▗", "▄", "▖"], ["▐", "█", "▌"]],
        ];
        let art = if self.icon == Icon::Agents {
            4
        } else {
            (self.icon as usize).min(3)
        };
        if self.width == 1 {
            return match self.icon {
                Icon::Files => "▤",
                Icon::Search => "⌕",
                Icon::Git => "⎇",
                Icon::Agents => "◉",
                _ => "⚙",
            };
        }
        ART[art][self.row.min(1) as usize][self.column.min(2) as usize]
    }
}

type Point = (f32, f32);

fn segment(p: Point, a: Point, b: Point) -> f32 {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let t = (((p.0 - a.0) * dx + (p.1 - a.1) * dy) / (dx * dx + dy * dy)).clamp(0.0, 1.0);
    ((p.0 - a.0 - t * dx).powi(2) + (p.1 - a.1 - t * dy).powi(2)).sqrt()
}

/// Distance to a line through several points. Segments have round ends, so joins do too.
fn path(p: Point, points: &[Point]) -> f32 {
    points
        .windows(2)
        .map(|pair| segment(p, pair[0], pair[1]))
        .fold(f32::MAX, f32::min)
}

fn circle(p: Point, c: Point) -> f32 {
    ((p.0 - c.0).powi(2) + (p.1 - c.1).powi(2)).sqrt()
}

/// Distance to a circle's outline.
fn ring(p: Point, c: Point, radius: f32) -> f32 {
    (circle(p, c) - radius).abs()
}

/// Distance to the outline of a box with round corners.
fn rounded(p: Point, min: Point, max: Point, radius: f32) -> f32 {
    let (cx, cy) = ((min.0 + max.0) / 2.0, (min.1 + max.1) / 2.0);
    let (qx, qy) = (
        (p.0 - cx).abs() - (max.0 - min.0) / 2.0 + radius,
        (p.1 - cy).abs() - (max.1 - min.1) / 2.0 + radius,
    );
    let outside = (qx.max(0.0).powi(2) + qy.max(0.0).powi(2)).sqrt();
    (outside + qx.max(qy).min(0.0) - radius).abs()
}

/// A page with its top right corner folded, starting at `x` across and `top` down.
fn page(p: Point, left: f32, right: f32, top: f32, bottom: f32) -> f32 {
    const FOLD: f32 = 4.5;
    const CORNER: f32 = 1.5;
    let (fold_x, fold_y) = (right - FOLD, top + FOLD);
    let outline = path(
        p,
        &[
            (fold_x, top),
            (left + CORNER, top),
            (left, top + CORNER),
            (left, bottom - CORNER),
            (left + CORNER, bottom),
            (right - CORNER, bottom),
            (right, bottom - CORNER),
            (right, fold_y),
            (fold_x, top),
        ],
    );
    outline.min(path(p, &[(fold_x, top), (fold_x, fold_y), (right, fold_y)]))
}

/// A ring, a hub and eight short teeth. Traced teeth turn to whiskers at this size.
fn gear(p: Point) -> f32 {
    const CENTRE: Point = (12.0, 12.0);
    const BODY: f32 = 6.6;
    const REACH: f32 = 9.8;
    const TEETH: usize = 8;
    /// Each tooth is heavier than the line, so it reads as a solid part of the wheel.
    const TOOTH_EXTRA: f32 = 0.55;
    let (x, y) = (p.0 - CENTRE.0, p.1 - CENTRE.1);
    let step = std::f32::consts::TAU / TEETH as f32;
    let nearest = (y.atan2(x) / step).round() * step;
    let at = |radius: f32| (12.0 + radius * nearest.cos(), 12.0 + radius * nearest.sin());
    let tooth = segment(p, at(BODY), at(REACH)) - TOOTH_EXTRA;
    ring(p, CENTRE, BODY).min(ring(p, CENTRE, 2.6)).min(tooth)
}

/// Whether a point is inside a box with round corners.
fn within(p: Point, min: Point, max: Point, radius: f32) -> bool {
    let (cx, cy) = ((min.0 + max.0) / 2.0, (min.1 + max.1) / 2.0);
    let (qx, qy) = (
        (p.0 - cx).abs() - (max.0 - min.0) / 2.0 + radius,
        (p.1 - cy).abs() - (max.1 - min.1) / 2.0 + radius,
    );
    (qx.max(0.0).powi(2) + qy.max(0.0).powi(2)).sqrt() + qx.max(qy).min(0.0) < radius
}

/// Folders and files are solid, as in VS Code's icon themes, so colour can carry the kind.
fn solid(icon: Icon, p: Point) -> Option<bool> {
    let folder =
        || within(p, (2.0, 7.5), (22.0, 20.5), 2.2) || within(p, (2.0, 4.0), (11.0, 11.0), 1.8);
    match icon {
        Icon::Folder => Some(folder()),
        Icon::FolderOpen => Some(folder() && !(p.1 > 10.2 && p.1 < 11.8)),
        Icon::File => {
            let page = within(p, (4.5, 2.0), (19.5, 22.0), 2.2);
            let fold = p.1 - 2.0 < p.0 - 13.5;
            let line = |y: f32| segment(p, (8.0, y), (16.0, y)) < 1.0;
            Some(page && !fold && !line(12.5) && !line(16.8))
        }
        _ => None,
    }
}

/// Half the stroke's width, in 24ths of the icon. The large icons stay fine-lined, and the
/// small ones are heavier so they keep their weight at a few pixels across.
fn half_stroke(icon: Icon) -> f32 {
    match icon {
        Icon::Files | Icon::Search | Icon::Git | Icon::Settings | Icon::Agents => 0.8,
        Icon::Check => 1.7,
        _ => 1.15,
    }
}

/// Whether a point in the icon's unit square is inked. Icons share a 24-unit grid and one
/// round-ended stroke, which is what makes them read as a set.
pub fn inked(icon: Icon, u: f32, v: f32) -> bool {
    let p = (u * 24.0, v * 24.0);
    if let Some(ink) = solid(icon, p) {
        return ink;
    }
    let distance = match icon {
        Icon::Files => {
            let back = path(p, &[(9.0, 2.5), (15.0, 2.5), (20.5, 8.0), (20.5, 17.5)]);
            back.min(page(p, 4.0, 16.0, 6.5, 21.5))
        }
        Icon::Search => ring(p, (10.5, 10.5), 7.0).min(segment(p, (15.6, 15.6), (21.0, 21.0))),
        Icon::Git => {
            let joint = (9.0, 9.0);
            let quarter = p.0 >= joint.0 && p.1 >= joint.1;
            let curve = if quarter {
                ring(p, joint, 9.0)
            } else {
                f32::MAX
            };
            ring(p, (6.0, 18.0), 3.0)
                .min(ring(p, (18.0, 6.0), 3.0))
                .min(segment(p, (6.0, 3.0), (6.0, 15.0)))
                .min(curve)
        }
        Icon::Settings => gear(p),
        Icon::Agents => rounded(p, (4.0, 8.5), (20.0, 20.5), 3.0)
            .min(path(p, &[(12.0, 8.5), (12.0, 3.5), (8.5, 3.5)]))
            .min(segment(p, (9.0, 13.0), (9.0, 15.5)))
            .min(segment(p, (15.0, 13.0), (15.0, 15.5)))
            .min(segment(p, (1.5, 14.5), (4.0, 14.5)))
            .min(segment(p, (20.0, 14.5), (22.5, 14.5))),
        Icon::Folder | Icon::FolderOpen | Icon::File => f32::MAX,
        Icon::ListBullet => (circle(p, (5.0, 6.5)) - 0.4)
            .min(circle(p, (5.0, 12.0)) - 0.4)
            .min(circle(p, (5.0, 17.5)) - 0.4)
            .min(segment(p, (9.5, 6.5), (20.5, 6.5)))
            .min(segment(p, (9.5, 12.0), (20.5, 12.0)))
            .min(segment(p, (9.5, 17.5), (20.5, 17.5))),
        Icon::ListNumber => path(p, &[(3.8, 5.2), (5.4, 4.0), (5.4, 9.5)])
            .min(path(
                p,
                &[
                    (3.6, 15.0),
                    (5.0, 13.8),
                    (6.8, 14.8),
                    (3.6, 19.8),
                    (7.2, 19.8),
                ],
            ))
            .min(segment(p, (10.5, 6.8), (20.5, 6.8)))
            .min(segment(p, (10.5, 16.8), (20.5, 16.8))),
        Icon::ListTask => rounded(p, (4.0, 4.0), (20.0, 20.0), 3.5)
            .min(path(p, &[(8.0, 12.2), (11.0, 15.2), (16.2, 9.0)])),
        Icon::Quote => segment(p, (4.5, 5.0), (4.5, 19.0))
            .min(segment(p, (9.5, 7.0), (20.5, 7.0)))
            .min(segment(p, (9.5, 12.0), (20.5, 12.0)))
            .min(segment(p, (9.5, 17.0), (16.5, 17.0))),
        Icon::Code => path(p, &[(8.5, 6.5), (3.0, 12.0), (8.5, 17.5)])
            .min(path(p, &[(15.5, 6.5), (21.0, 12.0), (15.5, 17.5)])),
        Icon::Link => rounded(p, (2.5, 8.0), (13.5, 16.0), 4.0).min(rounded(
            p,
            (10.5, 8.0),
            (21.5, 16.0),
            4.0,
        )),
        Icon::Table => rounded(p, (3.0, 4.5), (21.0, 19.5), 2.5)
            .min(segment(p, (3.0, 9.5), (21.0, 9.5)))
            .min(segment(p, (3.0, 14.5), (21.0, 14.5)))
            .min(segment(p, (12.0, 4.5), (12.0, 19.5))),
        Icon::Picture => rounded(p, (3.0, 4.5), (21.0, 19.5), 2.5)
            .min(ring(p, (8.5, 9.8), 1.5))
            .min(path(
                p,
                &[
                    (3.5, 18.0),
                    (9.5, 12.8),
                    (13.5, 16.2),
                    (16.2, 13.8),
                    (20.5, 17.5),
                ],
            )),
        Icon::Sigma => path(
            p,
            &[
                (18.0, 7.5),
                (18.0, 4.5),
                (5.5, 4.5),
                (12.5, 12.0),
                (5.5, 19.5),
                (18.0, 19.5),
                (18.0, 16.5),
            ],
        ),
        Icon::Rule => segment(p, (3.0, 12.0), (21.0, 12.0))
            .min(segment(p, (8.0, 6.5), (16.0, 6.5)))
            .min(segment(p, (8.0, 17.5), (16.0, 17.5))),
        Icon::Comment => rounded(p, (3.0, 4.0), (21.0, 16.5), 3.5)
            .min(path(p, &[(8.0, 16.5), (8.0, 20.5), (12.5, 16.5)])),
        Icon::Export => path(p, &[(12.0, 3.5), (12.0, 15.0)])
            .min(path(p, &[(7.0, 10.5), (12.0, 15.5), (17.0, 10.5)]))
            .min(segment(p, (4.0, 20.0), (20.0, 20.0))),
        Icon::Play => path(p, &[(7.0, 4.5), (19.5, 12.0), (7.0, 19.5), (7.0, 4.5)]),
        Icon::Cite => path(p, &[(9.0, 4.5), (5.5, 4.5), (5.5, 19.5), (9.0, 19.5)])
            .min(path(
                p,
                &[(15.0, 4.5), (18.5, 4.5), (18.5, 19.5), (15.0, 19.5)],
            ))
            .min(segment(p, (12.0, 9.0), (12.0, 15.0))),
        Icon::More => (circle(p, (5.0, 12.0)) - 0.5)
            .min(circle(p, (12.0, 12.0)) - 0.5)
            .min(circle(p, (19.0, 12.0)) - 0.5),
        Icon::ChevronRight => path(p, &[(9.5, 6.5), (15.0, 12.0), (9.5, 17.5)]),
        Icon::ChevronDown => path(p, &[(6.5, 9.5), (12.0, 15.0), (17.5, 9.5)]),
        Icon::Close => {
            segment(p, (6.5, 6.5), (17.5, 17.5)).min(segment(p, (17.5, 6.5), (6.5, 17.5)))
        }
        Icon::ReplaceAll => path(p, &[(3.0, 8.0), (19.5, 8.0), (15.0, 3.5)])
            .min(path(p, &[(21.0, 16.0), (4.5, 16.0), (9.0, 20.5)])),
        Icon::Check => path(p, &[(4.5, 12.5), (9.5, 17.5), (19.5, 6.5)]),
        Icon::Globe => {
            let (x, y) = ((p.0 - 12.0) / 4.5, (p.1 - 12.0) / 9.5);
            let meridian = ((x * x + y * y).sqrt() - 1.0).abs() * 4.5;
            ring(p, (12.0, 12.0), 9.5)
                .min(meridian)
                .min(segment(p, (2.5, 12.0), (21.5, 12.0)))
        }
    };
    distance < half_stroke(icon)
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
        for icon in [
            Icon::Files,
            Icon::Search,
            Icon::Git,
            Icon::Settings,
            Icon::Agents,
            Icon::Folder,
            Icon::File,
        ] {
            let dots = (0..50)
                .flat_map(|y| (0..50).map(move |x| (x, y)))
                .filter(|(x, y)| inked(icon, *x as f32 / 50.0, *y as f32 / 50.0))
                .count();
            assert!(dots > 150, "{icon:?} has {dots}");
        }
    }
}

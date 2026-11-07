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
    Outline = 14,
    /// Sheets stacked: what an agent's context is built from.
    Layers = 15,
    // The document toolbar's, which only ever appear as paint.
    ListBullet = 30,
    ListNumber = 53,
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
    Diagram = 29,
    Track = 31,
    PlayAll = 32,
    Stop = 33,
    Restart = 34,
    AddCell = 35,
    AddText = 36,
    Terminal = 37,
    Diff = 38,
    Replace = 39,
    ArrowUp = 40,
    ArrowDown = 41,
    /// Lucide's maximize-2 and minimize-2: arrows out to two corners, and back in.
    Maximize = 42,
    Minimize = 43,
    /// Lucide's panel-bottom: a window with a panel along its foot.
    Panel = 44,
    /// A symbol's kind, as Go to Symbol lists them: a cube for a function or method, braces
    /// for a type, a bar for a field, and two lines for a constant, variable or module.
    SymbolFunction = 45,
    SymbolType = 46,
    SymbolField = 47,
    SymbolValue = 48,
    /// What waits for the person.
    Bell = 49,
    /// The inbox's marks for what's waiting: asking, done, failed.
    Question = 50,
    CircleCheck = 51,
    CircleX = 52,
    /// A window with its right part filled, as VS Code's title bar has: the side pane
    /// shown, and the button that hides it. Not an ×, so the only × in the corner quits.
    HidePane = 54,
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
            14 => Icon::Outline,
            15 => Icon::Layers,
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
        const ART: [[[&str; 3]; 2]; 7] = [
            [["▛", "▀", "▜"], ["▙", "▄", "▟"]],
            [["▞", "▀", "▚"], ["▚", "▄", "▟"]],
            [["●", "─", "●"], ["●", "─", "╯"]],
            [["▚", "▀", "▞"], ["▞", "▄", "▚"]],
            [["▗", "▄", "▖"], ["▐", "█", "▌"]],
            [["▪", "─", "─"], [" ", "▪", "─"]],
            [["▗", "▀", "▖"], ["▝", "▀", "▘"]],
        ];
        let art = match self.icon {
            Icon::Agents => 4,
            Icon::Outline => 5,
            Icon::Layers => 6,
            other => (other as usize).min(3),
        };
        if self.width == 1 || self.height == 1 {
            if self.column > 0 {
                return " ";
            }
            return match self.icon {
                Icon::Files => "▤",
                Icon::Search => "⌕",
                Icon::Git => "⎇",
                Icon::Agents => "◉",
                Icon::Outline => "≡",
                Icon::Layers => "≋",
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

/// Distance to a box with round corners, filled: nothing inside it.
fn solid(p: Point, min: Point, max: Point, radius: f32) -> f32 {
    let (cx, cy) = ((min.0 + max.0) / 2.0, (min.1 + max.1) / 2.0);
    let (qx, qy) = (
        (p.0 - cx).abs() - (max.0 - min.0) / 2.0 + radius,
        (p.1 - cy).abs() - (max.1 - min.1) / 2.0 + radius,
    );
    let outside = (qx.max(0.0).powi(2) + qy.max(0.0).powi(2)).sqrt();
    (outside + qx.max(qy).min(0.0) - radius).max(0.0)
}

/// A box at the bottom left, an arrow turning up from it, and the corners of the box it
/// becomes at the top right.
fn replace(p: Point) -> f32 {
    rounded(p, (2.0, 14.0), (10.0, 22.0), 2.0)
        .min(path(
            p,
            &[(6.0, 10.0), (6.0, 5.0), (6.9, 2.9), (9.0, 2.0), (10.0, 2.0)],
        ))
        .min(path(p, &[(3.0, 7.0), (6.0, 10.0), (9.0, 7.0)]))
        .min(path(p, &[(14.0, 4.0), (14.6, 2.6), (16.0, 2.0)]))
        .min(path(p, &[(16.0, 10.0), (14.6, 9.4), (14.0, 8.0)]))
        .min(path(p, &[(20.0, 2.0), (21.4, 2.6), (22.0, 4.0)]))
        .min(path(p, &[(22.0, 8.0), (21.4, 9.4), (20.0, 10.0)]))
}

/// Half the stroke's width, in 24ths of the icon. The large icons stay fine-lined, and the
/// small ones are heavier so they keep their weight at a few pixels across.
fn half_stroke(icon: Icon) -> f32 {
    match icon {
        Icon::Files
        | Icon::Search
        | Icon::Git
        | Icon::Settings
        | Icon::Outline
        | Icon::Layers
        | Icon::File
        | Icon::Folder
        | Icon::FolderOpen
        | Icon::ChevronRight
        | Icon::ChevronDown
        | Icon::Close
        | Icon::ArrowUp
        | Icon::ArrowDown
        | Icon::Replace
        | Icon::Maximize
        | Icon::Minimize
        | Icon::SymbolFunction
        | Icon::SymbolType
        | Icon::SymbolField
        | Icon::SymbolValue
        | Icon::HidePane
        | Icon::ReplaceAll => 0.9,
        Icon::Agents => 0.8,
        Icon::Check => 1.7,
        _ => 1.15,
    }
}

/// Whether a point in the icon's unit square is inked. Icons share a 24-unit grid and one
/// round-ended stroke, which is what makes them read as a set.
pub fn inked(icon: Icon, u: f32, v: f32) -> bool {
    let p = (u * 24.0, v * 24.0);
    let distance = match icon {
        // A page with its corner folded, and the edge of one behind it.
        Icon::Files => path(
            p,
            &[
                (9.0, 18.0),
                (7.6, 17.4),
                (7.0, 16.0),
                (7.0, 4.0),
                (7.6, 2.6),
                (9.0, 2.0),
                (16.0, 2.0),
                (20.0, 6.0),
                (20.0, 16.0),
                (19.4, 17.4),
                (18.0, 18.0),
                (9.0, 18.0),
            ],
        )
        .min(path(
            p,
            &[
                (20.0, 7.0),
                (17.0, 7.0),
                (15.6, 6.4),
                (15.0, 5.0),
                (15.0, 2.0),
            ],
        ))
        .min(path(
            p,
            &[
                (3.0, 7.6),
                (3.0, 20.4),
                (3.5, 21.5),
                (4.6, 22.0),
                (14.4, 22.0),
            ],
        )),
        Icon::Search => ring(p, (11.0, 11.0), 7.0).min(segment(p, (16.65, 16.65), (20.0, 20.0))),
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
        // Two sliders, as settings are in most of today's apps.
        Icon::Settings => segment(p, (20.0, 7.0), (11.0, 7.0))
            .min(segment(p, (14.0, 17.0), (5.0, 17.0)))
            .min(ring(p, (17.0, 17.0), 3.0))
            .min(ring(p, (7.0, 7.0), 3.0)),
        Icon::Agents => rounded(p, (4.0, 8.5), (20.0, 20.5), 3.0)
            .min(path(p, &[(12.0, 8.5), (12.0, 3.5), (8.5, 3.5)]))
            .min(segment(p, (9.0, 13.0), (9.0, 15.5)))
            .min(segment(p, (15.0, 13.0), (15.0, 15.5)))
            .min(segment(p, (1.5, 14.5), (4.0, 14.5)))
            .min(segment(p, (20.0, 14.5), (22.5, 14.5))),
        // Lucide's file, folder and folder-open.
        Icon::File => path(
            p,
            &[
                (15.0, 2.0),
                (6.0, 2.0),
                (4.6, 2.6),
                (4.0, 4.0),
                (4.0, 20.0),
                (4.6, 21.4),
                (6.0, 22.0),
                (18.0, 22.0),
                (19.4, 21.4),
                (20.0, 20.0),
                (20.0, 7.0),
                (15.0, 2.0),
            ],
        )
        .min(path(
            p,
            &[
                (14.0, 2.0),
                (14.0, 6.0),
                (14.6, 7.4),
                (16.0, 8.0),
                (20.0, 8.0),
            ],
        )),
        Icon::Folder => path(
            p,
            &[
                (20.0, 20.0),
                (21.4, 19.4),
                (22.0, 18.0),
                (22.0, 8.0),
                (21.4, 6.6),
                (20.0, 6.0),
                (12.1, 6.0),
                (10.4, 5.1),
                (9.6, 3.9),
                (7.9, 3.0),
                (4.0, 3.0),
                (2.6, 3.6),
                (2.0, 5.0),
                (2.0, 18.0),
                (2.6, 19.4),
                (4.0, 20.0),
                (20.0, 20.0),
            ],
        ),
        Icon::FolderOpen => path(
            p,
            &[
                (6.0, 14.0),
                (7.5, 11.1),
                (9.2, 10.0),
                (20.0, 10.0),
                (21.9, 12.5),
                (20.4, 18.5),
                (18.5, 20.0),
                (4.0, 20.0),
                (2.6, 19.4),
                (2.0, 18.0),
                (2.0, 5.0),
                (2.6, 3.6),
                (4.0, 3.0),
                (7.9, 3.0),
                (9.6, 3.9),
                (10.4, 5.1),
                (12.1, 6.0),
                (18.0, 6.0),
                (19.4, 6.6),
                (20.0, 8.0),
                (20.0, 10.0),
            ],
        ),
        // Three sheets stacked, the top one whole.
        Icon::Layers => path(
            p,
            &[
                (12.0, 2.5),
                (21.5, 7.5),
                (12.0, 12.5),
                (2.5, 7.5),
                (12.0, 2.5),
            ],
        )
        .min(path(p, &[(2.5, 12.5), (12.0, 17.5), (21.5, 12.5)]))
        .min(path(p, &[(2.5, 17.0), (12.0, 22.0), (21.5, 17.0)])),
        // A tree of headings.
        Icon::Outline => segment(p, (21.0, 6.0), (8.0, 6.0))
            .min(segment(p, (21.0, 12.0), (13.0, 12.0)))
            .min(segment(p, (21.0, 18.0), (13.0, 18.0)))
            .min(path(
                p,
                &[
                    (3.0, 6.0),
                    (3.0, 10.0),
                    (3.6, 11.4),
                    (5.0, 12.0),
                    (8.0, 12.0),
                ],
            ))
            .min(path(
                p,
                &[
                    (3.0, 10.0),
                    (3.0, 16.0),
                    (3.6, 17.4),
                    (5.0, 18.0),
                    (8.0, 18.0),
                ],
            )),
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
        // A pencil over a line.
        Icon::Track => path(
            p,
            &[
                (4.0, 16.5),
                (15.5, 5.0),
                (19.0, 8.5),
                (7.5, 20.0),
                (4.0, 20.0),
                (4.0, 16.5),
            ],
        )
        .min(path(p, &[(13.0, 20.5), (21.0, 20.5)])),
        Icon::Diagram => rounded(p, (2.5, 3.5), (10.0, 9.5), 1.5)
            .min(rounded(p, (14.0, 3.5), (21.5, 9.5), 1.5))
            .min(rounded(p, (14.0, 14.5), (21.5, 20.5), 1.5))
            .min(path(p, &[(10.0, 6.5), (14.0, 6.5)]))
            .min(path(p, &[(6.2, 9.5), (6.2, 17.5), (14.0, 17.5)])),
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
        Icon::PlayAll => path(p, &[(3.0, 5.5), (12.0, 12.0), (3.0, 18.5), (3.0, 5.5)]).min(path(
            p,
            &[(12.0, 5.5), (21.0, 12.0), (12.0, 18.5), (12.0, 5.5)],
        )),
        Icon::Stop => rounded(p, (6.0, 6.0), (18.0, 18.0), 1.5),
        Icon::Restart => {
            // A circle with a gap at the top right, and an arrowhead closing it.
            let angle = (p.1 - 12.0).atan2(p.0 - 12.0);
            let gap = angle > -1.1 && angle < 0.1;
            let arc = if gap {
                f32::MAX
            } else {
                ring(p, (12.0, 12.0), 7.5)
            };
            arc.min(path(p, &[(19.5, 4.0), (19.5, 10.0), (13.8, 10.0)]))
        }
        Icon::AddCell => rounded(p, (3.0, 6.0), (21.0, 18.0), 2.0)
            .min(segment(p, (12.0, 9.0), (12.0, 15.0)))
            .min(segment(p, (9.0, 12.0), (15.0, 12.0))),
        Icon::AddText => segment(p, (3.0, 6.0), (21.0, 6.0))
            .min(segment(p, (3.0, 11.0), (13.0, 11.0)))
            .min(segment(p, (3.0, 16.0), (10.0, 16.0)))
            .min(segment(p, (18.0, 13.0), (18.0, 21.0)))
            .min(segment(p, (14.0, 17.0), (22.0, 17.0))),
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
        Icon::Replace => replace(p),
        Icon::ArrowUp => path(p, &[(5.0, 12.0), (12.0, 5.0), (19.0, 12.0)]).min(segment(
            p,
            (12.0, 5.0),
            (12.0, 19.0),
        )),
        Icon::ArrowDown => path(p, &[(19.0, 12.0), (12.0, 19.0), (5.0, 12.0)]).min(segment(
            p,
            (12.0, 5.0),
            (12.0, 19.0),
        )),
        Icon::ReplaceAll => replace(p)
            .min(path(
                p,
                &[
                    (14.0, 14.0),
                    (15.4, 14.6),
                    (16.0, 16.0),
                    (16.0, 20.0),
                    (15.4, 21.4),
                    (14.0, 22.0),
                ],
            ))
            .min(path(
                p,
                &[
                    (20.0, 14.0),
                    (21.4, 14.6),
                    (22.0, 16.0),
                    (22.0, 20.0),
                    (21.4, 21.4),
                    (20.0, 22.0),
                ],
            )),
        Icon::Check => path(p, &[(4.5, 12.5), (9.5, 17.5), (19.5, 6.5)]),
        // Lucide's panel-right, its right part filled.
        Icon::HidePane => rounded(p, (3.0, 3.0), (21.0, 21.0), 2.0)
            .min(segment(p, (15.0, 3.0), (15.0, 21.0)))
            .min(solid(p, (15.0, 3.0), (21.0, 21.0), 2.0)),
        Icon::Maximize => path(p, &[(15.0, 3.0), (21.0, 3.0), (21.0, 9.0)])
            .min(path(p, &[(9.0, 21.0), (3.0, 21.0), (3.0, 15.0)]))
            .min(segment(p, (21.0, 3.0), (14.0, 10.0)))
            .min(segment(p, (3.0, 21.0), (10.0, 14.0))),
        Icon::Minimize => path(p, &[(4.0, 14.0), (10.0, 14.0), (10.0, 20.0)])
            .min(path(p, &[(20.0, 10.0), (14.0, 10.0), (14.0, 4.0)]))
            .min(segment(p, (14.0, 10.0), (21.0, 3.0)))
            .min(segment(p, (3.0, 21.0), (10.0, 14.0))),
        // Lucide's box.
        Icon::SymbolFunction => path(
            p,
            &[
                (12.0, 2.5),
                (20.5, 7.2),
                (20.5, 16.8),
                (12.0, 21.5),
                (3.5, 16.8),
                (3.5, 7.2),
                (12.0, 2.5),
            ],
        )
        .min(path(p, &[(3.8, 7.3), (12.0, 12.0), (20.2, 7.3)]))
        .min(segment(p, (12.0, 12.0), (12.0, 21.5))),
        Icon::SymbolType => path(
            p,
            &[
                (8.5, 3.0),
                (7.0, 3.0),
                (5.5, 4.5),
                (5.5, 10.0),
                (3.5, 12.0),
                (5.5, 14.0),
                (5.5, 19.5),
                (7.0, 21.0),
                (8.5, 21.0),
            ],
        )
        .min(path(
            p,
            &[
                (15.5, 3.0),
                (17.0, 3.0),
                (18.5, 4.5),
                (18.5, 10.0),
                (20.5, 12.0),
                (18.5, 14.0),
                (18.5, 19.5),
                (17.0, 21.0),
                (15.5, 21.0),
            ],
        )),
        Icon::SymbolField => {
            rounded(p, (3.0, 7.5), (21.0, 16.5), 2.5).min(segment(p, (7.0, 12.0), (12.0, 12.0)))
        }
        Icon::SymbolValue => {
            segment(p, (4.5, 9.0), (19.5, 9.0)).min(segment(p, (4.5, 15.0), (19.5, 15.0)))
        }
        Icon::Terminal => path(p, &[(3.5, 6.0), (10.0, 12.0), (3.5, 18.0)]).min(segment(
            p,
            (12.5, 18.0),
            (20.5, 18.0),
        )),
        Icon::Panel => {
            rounded(p, (3.0, 3.0), (21.0, 21.0), 4.0).min(segment(p, (3.0, 15.0), (21.0, 15.0)))
        }
        Icon::Diff => rounded(p, (3.0, 3.0), (21.0, 21.0), 4.0)
            .min(segment(p, (12.0, 6.5), (12.0, 12.5)))
            .min(segment(p, (9.0, 9.5), (15.0, 9.5)))
            .min(segment(p, (9.0, 16.5), (15.0, 16.5))),
        // Lucide's bell: a dome on a flared rim, and the clapper under it.
        Icon::Bell => {
            let dome = if p.1 <= 9.0 {
                ring(p, (12.0, 9.0), 6.0)
            } else {
                f32::MAX
            };
            dome.min(path(
                p,
                &[(6.0, 9.0), (6.0, 12.5), (5.2, 15.4), (3.2, 17.5)],
            ))
            .min(path(
                p,
                &[(18.0, 9.0), (18.0, 12.5), (18.8, 15.4), (20.8, 17.5)],
            ))
            .min(segment(p, (3.2, 17.5), (20.8, 17.5)))
            .min(path(p, &[(10.2, 20.8), (12.0, 21.8), (13.8, 20.8)]))
        }
        Icon::Question => ring(p, (12.0, 12.0), 9.5)
            .min(path(
                p,
                &[
                    (9.3, 9.2),
                    (10.1, 7.6),
                    (12.0, 7.0),
                    (14.0, 7.7),
                    (14.8, 9.4),
                    (14.0, 11.0),
                    (12.0, 12.2),
                    (12.0, 13.6),
                ],
            ))
            .min(circle(p, (12.0, 17.0)) - 0.4),
        Icon::CircleCheck => {
            ring(p, (12.0, 12.0), 9.5).min(path(p, &[(8.0, 12.3), (10.8, 15.0), (16.0, 9.5)]))
        }
        Icon::CircleX => ring(p, (12.0, 12.0), 9.5)
            .min(segment(p, (8.8, 8.8), (15.2, 15.2)))
            .min(segment(p, (15.2, 8.8), (8.8, 15.2))),
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
        for icon in [
            Icon::Files,
            Icon::Search,
            Icon::Git,
            Icon::Settings,
            Icon::Layers,
        ] {
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
            Icon::Layers,
            Icon::Folder,
            Icon::File,
            Icon::Maximize,
            Icon::Minimize,
            Icon::Panel,
            Icon::SymbolFunction,
            Icon::SymbolType,
            Icon::SymbolField,
            Icon::SymbolValue,
            Icon::Bell,
            Icon::Question,
            Icon::CircleCheck,
            Icon::CircleX,
            Icon::HidePane,
        ] {
            let dots = (0..50)
                .flat_map(|y| (0..50).map(move |x| (x, y)))
                .filter(|(x, y)| inked(icon, *x as f32 / 50.0, *y as f32 / 50.0))
                .count();
            assert!(dots > 150, "{icon:?} has {dots}");
        }
    }
}

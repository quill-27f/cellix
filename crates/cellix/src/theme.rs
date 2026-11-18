use cellix_lang::Token;
use cellix_render::{Rgb, Style};

/// Colours for the whole interface.
#[derive(Clone, Copy, Debug)]
pub struct Theme {
    pub text: Style,
    pub sidebar: Style,
    pub bar: Style,
    pub tab: Style,
    pub tab_active: Style,
    pub gutter: Style,
    pub selection: Rgb,
    /// Matches of a search, other than the selected one.
    pub found: Rgb,
    /// The wash on a row under the pointer, on the sidebar.
    pub row_hover: Rgb,
    /// The same on the sheet, and on a floating panel.
    pub page_hover: Rgb,
    pub popup_hover: Rgb,
    pub row_active: Rgb,
    pub accent: Rgb,
    /// A wash of the accent, behind what's selected.
    pub tint: Rgb,
    /// A wash of green, behind a hunk's Accept.
    pub accept_tint: Rgb,
    /// What wants the reviewer: an agent waiting, a failed step.
    pub attention: Rgb,
    pub keyword: Rgb,
    pub string: Rgb,
    pub comment: Rgb,
    pub function: Rgb,
    pub kind: Rgb,
    pub number: Rgb,
    pub added: Rgb,
    pub removed: Rgb,
    /// Stronger versions, for the words that changed within a line.
    pub added_word: Rgb,
    pub removed_word: Rgb,
    /// Lines that moved rather than changed.
    pub moved: Rgb,
    pub activity: Style,
    pub status: Style,
    /// The bar of workspaces under the status bar. It's the bottom row, so it's the
    /// terminal's own colour like the status bar, and its tiles stand out as chips.
    pub workspaces: Style,
    pub breadcrumb: Style,
    /// Background of the line holding the cursor.
    pub line: Rgb,
    pub number_active: Rgb,
    pub guide: Rgb,
    pub scrollbar: Rgb,
    pub error: Rgb,
    pub heading: Rgb,
    pub git_modified: Rgb,
    pub git_added: Rgb,
    /// Background of a text box.
    pub input: Rgb,
    pub find_match: Rgb,
    /// Behind code blocks on a rendered Markdown page.
    pub code_bg: Rgb,
    /// The part of a minimap in view.
    pub minimap_view: Rgb,
    /// Hairlines between regions.
    pub border: Rgb,
    /// The border of a focused box.
    pub focus: Rgb,
    /// Floating panels such as the palette.
    pub popup: Rgb,
    /// Secondary text on a floating panel, such as shortcuts.
    pub popup_dim: Rgb,
    /// What can't be done right now.
    pub disabled: Rgb,
    /// A gitignored file's name and icon in the tree: there, but quiet.
    pub ignored: Rgb,
    pub popup_border: Rgb,
    /// The highlighted row of a list in a floating panel.
    pub pick: Rgb,
    pub button: Rgb,
    pub button_text: Rgb,
}

fn style(fg: Rgb, bg: Rgb) -> Style {
    Style {
        fg,
        bg,
        ..Style::default()
    }
}

fn shifted(color: Rgb, by: (i16, i16, i16)) -> Rgb {
    let move_by = |channel: u8, by: i16| (i16::from(channel) + by).clamp(0, 255) as u8;
    Rgb(
        move_by(color.0, by.0),
        move_by(color.1, by.1),
        move_by(color.2, by.2),
    )
}

/// VS Code's Modern themes, since that's the look people already know. Surfaces are quiet
/// and the blue is kept for what's active.
impl Theme {
    /// Text a tracked change adds: dark, the green of what's added.
    pub fn inserted(&self) -> Rgb {
        if self.is_light() {
            Rgb(0x1a, 0x7f, 0x37)
        } else {
            self.git_added
        }
    }

    /// Text a tracked change takes away: dark, the red of what's removed.
    pub fn deleted(&self) -> Rgb {
        if self.is_light() {
            Rgb(0xcf, 0x22, 0x2e)
        } else {
            self.error
        }
    }

    /// What highlighted text stands on.
    pub fn highlighter(&self) -> Rgb {
        if self.is_light() {
            Rgb(0xff, 0xec, 0x9e)
        } else {
            Rgb(0x6b, 0x5a, 0x1d)
        }
    }

    /// Bracket pairs by depth: gold, orchid and sky, as VS Code's, toned to sit with the
    /// syntax colours. The light ones pass 4.5:1 on white.
    pub fn brackets(&self) -> [Rgb; 3] {
        if self.is_light() {
            [
                Rgb(0x9a, 0x67, 0x00),
                Rgb(0xa2, 0x34, 0x9f),
                Rgb(0x1f, 0x63, 0xc7),
            ]
        } else {
            [
                Rgb(0xe6, 0xc0, 0x7b),
                Rgb(0xd2, 0x9f, 0xe3),
                Rgb(0x6c, 0xb6, 0xff),
            ]
        }
    }

    /// What the caret's matching brackets stand on, part way from the text to a selection.
    pub fn bracket_match(&self) -> Rgb {
        let mix = |a: u8, b: u8| ((u16::from(a) * 3 + u16::from(b) * 2) / 5) as u8;
        let (s, bg) = (self.selection, self.text.bg);
        Rgb(mix(s.0, bg.0), mix(s.1, bg.1), mix(s.2, bg.2))
    }

    /// A folded region's chip: lighter than both the text and the caret's line, so it shows
    /// on either. Dark, white at 14% over the sheet, as herdr lays its strongest wash.
    pub fn fold_chip(&self) -> Rgb {
        if self.is_light() {
            Rgb(0xdc, 0xe0, 0xe8)
        } else {
            mix(self.text.bg, Rgb(255, 255, 255), 36.0 / 255.0)
        }
    }

    /// The ring around the caret's matching brackets, on the pixel tiers: dark, part way from
    /// the sheet to the ring round what has focus.
    pub fn bracket_ring(&self) -> Rgb {
        if self.is_light() {
            Rgb(0x9d, 0xb3, 0xe6)
        } else {
            mix(self.text.bg, self.focus, 0.45)
        }
    }

    /// An inlay hint's text and chip on a line of colour `under`: the chip a little toward the
    /// text, so it shows on the current line's highlight and in light themes too.
    pub fn hint(&self, under: Rgb) -> (Rgb, Rgb) {
        let toward = |from: u8, to: u8| {
            let (from, to) = (f32::from(from), f32::from(to));
            (from + (to - from) * 0.08).round() as u8
        };
        let Rgb(r, g, b) = under;
        let Rgb(tr, tg, tb) = self.text.fg;
        (
            self.popup_dim,
            Rgb(toward(r, tr), toward(g, tg), toward(b, tb)),
        )
    }

    /// The theme the sidebar is drawn in. The file and the side pane are sheets; the sidebar,
    /// with the views over it and the bar under it, is the frame they stand on. So it has the
    /// panels' colours on the frame's ground, and what washes a row moved as far as the
    /// ground did.
    pub fn on_frame(&self) -> Self {
        let (was, now) = (self.sidebar.bg, self.activity.bg);
        let by = (
            i16::from(now.0) - i16::from(was.0),
            i16::from(now.1) - i16::from(was.1),
            i16::from(now.2) - i16::from(was.2),
        );
        let mut side = *self;
        side.sidebar.bg = now;
        side.row_hover = shifted(self.row_hover, by);
        side.row_active = shifted(self.row_active, by);
        side
    }

    /// Claude Code's mark: a warm clay, deeper on a light ground so it reads there.
    pub fn claude_mark(&self) -> Rgb {
        if self.is_light() {
            Rgb(0xb0, 0x55, 0x2e)
        } else {
            Rgb(0xde, 0x8a, 0x66)
        }
    }

    /// Gemini CLI's mark: a blue of its own, deeper on a light ground.
    pub fn gemini_mark(&self) -> Rgb {
        if self.is_light() {
            Rgb(0x2c, 0x5e, 0xd0)
        } else {
            Rgb(0x7a, 0xa2, 0xf7)
        }
    }

    pub fn is_light(&self) -> bool {
        let Rgb(r, g, b) = self.text.bg;
        u16::from(r) + u16::from(g) + u16::from(b) > 3 * 128
    }

    /// Moves every surface by the same amount, so the panels keep their distance from the
    /// terminal's background. The frame sits exactly on it, light or dark, so the window's
    /// padding, which is the terminal's background, can't show as a seam around the rail and
    /// the status bar. Light, that puts the rest a step or two paler than Latte on a Latte
    /// terminal; the seam was the worse of the two (decided with the user on 2026-11-09).
    /// Text colours stay, but in the dark theme the quiet ones on a ground that came up toward
    /// them are brought toward the words' own colour until they read as well as they did.
    pub fn on_background(mut self, background: Rgb) -> Self {
        let base = self.activity.bg;
        let by = (
            i16::from(background.0) - i16::from(base.0),
            i16::from(background.1) - i16::from(base.1),
            i16::from(background.2) - i16::from(base.2),
        );
        let dark = !self.is_light();
        // The rail and the status bar are the terminal's own colour, so its window padding
        // can't show as a seam around them.
        let (text_was, sidebar_was, popup_was) = (self.text.bg, self.sidebar.bg, self.popup);
        for surface in [
            &mut self.sidebar,
            &mut self.bar,
            &mut self.tab,
            &mut self.activity,
            &mut self.status,
            &mut self.workspaces,
            &mut self.text,
            &mut self.tab_active,
            &mut self.gutter,
            &mut self.breadcrumb,
        ] {
            surface.bg = shifted(surface.bg, by);
        }
        self.popup = shifted(self.popup, by);
        // What's drawn on a surface moves as far as that surface did. A light surface stops at
        // white, and a wash moved further would fade into it: a menu's highlight, a text box,
        // a code block on the editor.
        let moved = |was: Rgb, now: Rgb| {
            (
                i16::from(now.0) - i16::from(was.0),
                i16::from(now.1) - i16::from(was.1),
                i16::from(now.2) - i16::from(was.2),
            )
        };
        let on_text = moved(text_was, self.text.bg);
        let on_sidebar = moved(sidebar_was, self.sidebar.bg);
        let on_popup = moved(popup_was, self.popup);
        for color in [
            &mut self.page_hover,
            &mut self.line,
            &mut self.guide,
            &mut self.scrollbar,
            &mut self.code_bg,
            &mut self.minimap_view,
            &mut self.border,
            &mut self.tint,
            &mut self.accept_tint,
        ] {
            *color = shifted(*color, on_text);
        }
        for color in [&mut self.row_hover, &mut self.row_active] {
            *color = shifted(*color, on_sidebar);
        }
        for color in [
            &mut self.popup_hover,
            &mut self.input,
            &mut self.popup_border,
            &mut self.pick,
        ] {
            *color = shifted(*color, on_popup);
        }
        // Dark, herdr's frame is darker than most terminals, so the sheets come well up from
        // where they were: the washes on them come with them, or an added line would be
        // darker than the sheet it's on, and quiet words keep the contrast they had on their
        // own ground. The light theme's stay where Latte has them.
        if !dark {
            return self;
        }
        for color in [
            &mut self.selection,
            &mut self.found,
            &mut self.find_match,
            &mut self.added,
            &mut self.removed,
            &mut self.added_word,
            &mut self.removed_word,
            &mut self.moved,
        ] {
            *color = shifted(*color, on_text);
        }
        let words = self.text.fg;
        let (frame_was, frame) = (base, self.activity.bg);
        let (text_now, popup_now) = (self.text.bg, self.popup);
        let kept = |color: Rgb, was: Rgb, now: Rgb| {
            let wanted = contrast(color, was);
            (0..=20)
                .map(|step| mix(color, words, step as f32 / 20.0))
                .find(|lifted| contrast(*lifted, now) >= wanted)
                .unwrap_or(words)
        };
        for color in [
            &mut self.gutter.fg,
            &mut self.tab.fg,
            &mut self.breadcrumb.fg,
            &mut self.comment,
            &mut self.disabled,
            &mut self.ignored,
        ] {
            *color = kept(*color, text_was, text_now);
        }
        for color in [
            &mut self.activity.fg,
            &mut self.status.fg,
            &mut self.workspaces.fg,
        ] {
            *color = kept(*color, frame_was, frame);
        }
        self.popup_dim = kept(self.popup_dim, popup_was, popup_now);
        self
    }

    /// The match Find is on, in a file, on a page, in a terminal or in a list: filled with the
    /// blue, with a ring in the focus colour round it where there are pixels (`focus`). It was
    /// the selection's colour, 1.2 to 1 from the other matches. Light, the selection's blue,
    /// which the light theme's words read on.
    pub fn current_match(&self) -> Rgb {
        if self.is_light() {
            self.selection
        } else {
            mix(self.text.bg, self.accent, 0.35)
        }
    }

    /// What can't be done here but has to be read, as an agent that isn't installed and the
    /// note that says so. Dark, the quiet words' colour, herdr's muted. Light, Latte's subtext0,
    /// #6c6f85, 4.9 to 1 on white: the light theme's quiet words are all but its text's colour
    /// (1.1 to 1), and a row in them read as one that can be picked.
    pub fn unavailable(&self) -> Rgb {
        if self.is_light() {
            Rgb(0x6c, 0x6f, 0x85)
        } else {
            self.popup_dim
        }
    }

    /// A warning, beside an error in the deleted colour. Dark, graphite's yellow, quieter than
    /// its red; light, the attention colour, as it was.
    pub fn warning(&self) -> Rgb {
        if self.is_light() {
            self.attention
        } else {
            self.git_modified
        }
    }

    /// A built-in theme by name. Unknown names get the default.
    pub fn named(name: Option<&str>) -> Self {
        match name {
            Some("light") => Self::light(),
            _ => Self::default(),
        }
    }

    /// Catppuccin Latte's surfaces: the editor on base, the sidebars on mantle, the frame on
    /// crust, and menus white. Its text and code colours are deepened until each reads at
    /// 4.5 to 1 on those surfaces, which Latte's own green, yellow and blue don't.
    fn light() -> Self {
        let text = Rgb(0x4C, 0x4F, 0x69);
        let second = Rgb(0x53, 0x56, 0x6F);
        let third = Rgb(0x5C, 0x5F, 0x77);
        let base = Rgb(0xEF, 0xF1, 0xF5);
        let mantle = Rgb(0xE6, 0xE9, 0xEF);
        let crust = Rgb(0xDC, 0xE0, 0xE8);
        let surface0 = Rgb(0xCC, 0xD0, 0xDA);
        let surface1 = Rgb(0xBC, 0xC0, 0xCC);
        let line = Rgb(0xAC, 0xB0, 0xBE);
        let overlay0 = Rgb(0x9C, 0xA0, 0xB0);
        let overlay2 = Rgb(0x7C, 0x7F, 0x93);
        let blue = Rgb(0x1A, 0x5A, 0xE0);
        let mauve = Rgb(0x7C, 0x3A, 0xED);
        let green = Rgb(0x1F, 0x7A, 0x1A);
        // Washes keep Latte's own green, yellow and red: they sit behind text, not as it.
        let wash_green = Rgb(0x40, 0xA0, 0x2B);
        let wash_yellow = Rgb(0xDF, 0x8E, 0x1D);
        let wash_red = Rgb(0xD2, 0x0F, 0x39);
        let amber = Rgb(0x8A, 0x5A, 0x00);
        let peach = Rgb(0xB3, 0x4A, 0x05);
        let red = Rgb(0xC2, 0x0E, 0x35);
        let lavender = Rgb(0x72, 0x87, 0xFD);
        let white = Rgb(0xFF, 0xFF, 0xFF);
        let chosen = Rgb(0xC8, 0xD6, 0xF8);
        Self {
            text: style(text, base),
            // The side pane is a sheet as the file is, and so is the ground of every tab.
            // The sidebar itself stands on the frame: see `on_frame`.
            sidebar: style(text, base),
            bar: style(text, mantle),
            tab: style(second, base),
            tab_active: style(text, base),
            gutter: style(third, base),
            selection: chosen,
            found: mix(base, wash_yellow, 0.3),
            row_hover: mantle,
            page_hover: mantle,
            popup_hover: mantle,
            row_active: chosen,
            accent: blue,
            tint: mix(base, blue, 0.13),
            accept_tint: mix(base, wash_green, 0.15),
            attention: peach,
            keyword: mauve,
            string: green,
            comment: third,
            function: blue,
            kind: amber,
            number: peach,
            added: mix(base, wash_green, 0.15),
            removed: mix(base, wash_red, 0.15),
            added_word: mix(base, wash_green, 0.3),
            removed_word: mix(base, wash_red, 0.3),
            moved: mix(base, lavender, 0.15),
            activity: style(third, crust),
            status: style(second, crust),
            workspaces: style(second, crust),
            breadcrumb: style(second, base),
            line: mantle,
            number_active: text,
            guide: surface0,
            scrollbar: surface1,
            error: red,
            heading: text,
            git_modified: amber,
            git_added: green,
            input: white,
            find_match: mix(base, wash_yellow, 0.45),
            code_bg: mantle,
            minimap_view: crust,
            border: surface0,
            focus: blue,
            popup: white,
            popup_dim: second,
            disabled: overlay0,
            ignored: overlay2,
            popup_border: line,
            pick: mix(white, blue, 0.16),
            button: blue,
            button_text: white,
        }
    }

    /// What's quieter on the picked row of a list in a floating panel: halfway from the
    /// quiet colour to the words' own. The quiet colour itself is 4.4 to 1 on the band, and
    /// the words' own made a file's folder as loud as its name.
    pub fn quiet_on_pick(&self) -> Rgb {
        mix(self.popup_dim, self.text.fg, 0.5)
    }

    /// The thumb at the edge of a list in a floating panel: a third of the way from the
    /// scrollbar's colour, which is the panel's edge line's too, to the quiet text's.
    pub fn popup_thumb(&self) -> Rgb {
        mix(self.scrollbar, self.popup_dim, 0.35)
    }

    /// What a text box in a floating panel is filled with. A box is sunk into what holds
    /// it. Where a text box is the panel's own colour, as both are white in the light
    /// theme, it's a step from the panel toward the words' colour. It was the sheet's
    /// colour, which a light terminal's background moves to white as well.
    pub fn popup_input(&self) -> Rgb {
        if self.input == self.popup {
            mix(self.popup, self.text.fg, 0.09)
        } else {
            self.input
        }
    }

    /// The line round a text box that doesn't have the keyboard. Dark, white at 20% over a
    /// menu, 2.4 to 1 on the frame: the box is sunk only a step under the frame, so on the
    /// sidebar its fill is all but the frame's, and a menu's line round it (1.69 to 1) left an
    /// empty Replace box nearly gone. Light, a menu's line, which reads on white and on Latte's
    /// grounds.
    pub fn field_line(&self) -> Rgb {
        if self.is_light() {
            self.popup_border
        } else {
            mix(self.popup, Rgb(255, 255, 255), 0.2)
        }
    }

    /// The theme a menu is drawn in where it opens over a card, which is a menu's colour: dark,
    /// the menu and what's on it a step up, white at 11% over it, so it stands above the card
    /// and above what's set on a card in a quiet button's fill, an agent's answer (they were one
    /// colour, the menu's edge the only line between them; at 5.5% the menu was darker than an
    /// answer under it). Light, as it is: a white menu over a white card has its line and its
    /// shadow.
    pub fn over_card(&self) -> Self {
        if self.is_light() {
            return *self;
        }
        let raised = mix(self.popup, Rgb(255, 255, 255), 0.11);
        let by = (
            i16::from(raised.0) - i16::from(self.popup.0),
            i16::from(raised.1) - i16::from(self.popup.1),
            i16::from(raised.2) - i16::from(self.popup.2),
        );
        let mut theme = *self;
        theme.popup = raised;
        for color in [
            &mut theme.popup_hover,
            &mut theme.pick,
            &mut theme.popup_border,
        ] {
            *color = shifted(*color, by);
        }
        // Its quiet words, brought toward the words' own colour until they read as well as on a
        // menu's own colour.
        let wanted = contrast(self.popup_dim, self.popup);
        theme.popup_dim = (0..=20)
            .map(|step| mix(self.popup_dim, self.text.fg, step as f32 / 20.0))
            .find(|lifted| contrast(*lifted, raised) >= wanted)
            .unwrap_or(self.text.fg);
        theme
    }

    /// Every button that isn't the main one, wherever it stands: the quiet fill over what's
    /// behind it, and a second step off that under the pointer, so the hover always shows.
    /// A pointed one took the menu's hover in places, which is darker than the quiet fill on a
    /// card, so it went dimmer under the pointer.
    pub fn secondary(&self, behind: Rgb, hovered: bool) -> Rgb {
        let quiet = self.quiet_button(behind);
        if hovered {
            self.quiet_button(quiet)
        } else {
            quiet
        }
    }

    /// A button that isn't the main one, as a wash over whatever's behind it.
    pub fn quiet_button(&self, behind: Rgb) -> Rgb {
        // Dark, about herdr's raised, white at 9% over graphite's sheet (#2c2e31): the 14 of
        // the blue-grey theme before it left a button on the sheet dimmer than a picked row
        // beside it, and its hover hard to see. Light grounds need a deeper wash to read as a
        // button at all: Latte's base and a 14 step apart are 1.14:1, under the 1.3:1 a quiet
        // button needs.
        const WASH: i16 = 20;
        const LIGHT_WASH: i16 = 24;
        let lighter = behind.0 < 128;
        let by = if lighter { WASH } else { -LIGHT_WASH };
        shifted(behind, (by, by, by))
    }

    pub fn token(&self, token: Token) -> Rgb {
        match token {
            Token::Keyword => self.keyword,
            Token::String => self.string,
            Token::Comment => self.comment,
            Token::Function => self.function,
            Token::Type => self.kind,
            Token::Number | Token::Constant => self.number,
            Token::Attribute | Token::Heading => self.accent,
        }
    }
}

impl Default for Theme {
    /// The dark theme: herdr's graphite, the default dark look of herdr's desktop app, from
    /// herdr-app's `settings.rs` and checked against herdr.dev. Two surfaces, the frame and the
    /// sheet, with what's raised over them; three weights of text; one blue; herdr's colours for
    /// meaning and Catppuccin Mocha's for code.
    fn default() -> Self {
        // herdr's chrome (its sidebar, tab strip and status bar): the sheet 32% toward black.
        let frame = Rgb(0x10, 0x11, 0x14);
        // herdr's canvas, between its panes: 16% toward black.
        let panel = Rgb(0x13, 0x15, 0x18);
        // Graphite's background, what herdr's panes stand on.
        let sheet = Rgb(0x17, 0x19, 0x1d);
        // herdr's popover, a menu's ground: 6% toward white.
        let raised = Rgb(0x25, 0x27, 0x2b);
        // A text box is sunk a step under the frame, as it was before herdr's colours.
        let sunk = Rgb(0x0d, 0x0e, 0x11);
        let text = Rgb(0xee, 0xee, 0xef);
        // herdr's muted. Its dim, 0.38 of the way, is 3.3 to 1 on the sheet, so what has to be
        // read is a little brighter, and dim is for what can't be done.
        let second = mix(sheet, text, 0.6);
        let third = mix(sheet, text, 0.55);
        let dim = mix(sheet, text, 0.38);
        // The site's blue for what's active; the code's accent rings what has focus.
        let blue = Rgb(0x76, 0xa8, 0xff);
        let ring = Rgb(0x96, 0xb2, 0xd7);
        // Meaning: done and added, herdr's blocked for what needs you, graphite's yellow and
        // red for what changed and what's removed or failed.
        let green = Rgb(0x4c, 0xc4, 0x8c);
        let blocked = Rgb(0xf0, 0x70, 0x5f);
        let yellow = Rgb(0xd8, 0xbe, 0x87);
        let red = Rgb(0xdc, 0x8c, 0x95);
        // Code keeps Catppuccin Mocha's hues: each reads at 7 to 1 or better here.
        let mauve = Rgb(203, 166, 247);
        let leaf = Rgb(166, 227, 161);
        let straw = Rgb(249, 226, 175);
        let peach = Rgb(250, 179, 135);
        // herdr lays white over whatever is under it, at these strengths of 255.
        let white = Rgb(255, 255, 255);
        let hover = |ground: Rgb| mix(ground, white, 14.0 / 255.0);
        // What's picked is the blue over its ground, as herdr.dev's picked rows are: one kind
        // of picked, in every list, palette and menu and behind a commented span. herdr's
        // white at 9% was 1.10 to 1 from a hover and read as one beside it.
        let picked = |ground: Rgb| mix(ground, blue, 0.16);
        let hairline = |ground: Rgb| mix(ground, white, 24.0 / 255.0);
        let strong = |ground: Rgb| mix(ground, white, 36.0 / 255.0);
        Self {
            text: style(text, sheet),
            // The side pane is a sheet as the file is, and so is the ground of every tab.
            // The sidebar itself stands on the frame: see `on_frame`.
            sidebar: style(text, sheet),
            bar: style(text, panel),
            tab: style(second, sheet),
            tab_active: style(text, sheet),
            gutter: style(third, sheet),
            selection: Rgb(0x39, 0x44, 0x54),
            // The yellow at 28%, 1.9 to 1 on the sheet: at 18% it was 1.48, too faint to find.
            // The match Find is on is the blue, with a ring round it (`current_match`).
            found: mix(sheet, yellow, 0.28),
            row_hover: hover(sheet),
            page_hover: hover(sheet),
            popup_hover: hover(raised),
            row_active: picked(sheet),
            accent: blue,
            tint: picked(sheet),
            accept_tint: mix(sheet, green, 0.15),
            attention: blocked,
            keyword: mauve,
            string: leaf,
            comment: third,
            function: blue,
            kind: straw,
            number: peach,
            // Stronger than herdr's 9% washes, which read as grey: a line added has to read
            // as green at a glance.
            added: mix(sheet, green, 0.2),
            removed: mix(sheet, blocked, 0.2),
            added_word: mix(sheet, green, 0.36),
            removed_word: mix(sheet, blocked, 0.36),
            moved: mix(sheet, blue, 0.1),
            activity: style(third, frame),
            status: style(second, frame),
            workspaces: style(second, frame),
            breadcrumb: style(second, sheet),
            line: hover(sheet),
            number_active: text,
            guide: hairline(sheet),
            scrollbar: strong(sheet),
            error: red,
            heading: text,
            git_modified: yellow,
            git_added: green,
            input: sunk,
            find_match: mix(sheet, yellow, 0.45),
            // A page's code block is sunk to the frame's depth.
            code_bg: frame,
            minimap_view: strong(sheet),
            border: hairline(sheet),
            focus: ring,
            popup: raised,
            popup_dim: second,
            disabled: dim,
            ignored: dim,
            popup_border: hairline(raised),
            pick: picked(raised),
            button: blue,
            // herdr writes on its blue in the sheet's colour.
            button_text: sheet,
        }
    }
}

/// `by` of the way from `from` to `to`: a wash of one colour over another.
fn mix(from: Rgb, to: Rgb, by: f32) -> Rgb {
    let channel = |a: u8, b: u8| (f32::from(a) + (f32::from(b) - f32::from(a)) * by).round() as u8;
    Rgb(
        channel(from.0, to.0),
        channel(from.1, to.1),
        channel(from.2, to.2),
    )
}

/// How far apart two colours are to the eye, as WCAG counts it: 1 to 21.
pub(crate) fn contrast(a: Rgb, b: Rgb) -> f32 {
    let channel = |c: u8| {
        let c = f32::from(c) / 255.0;
        if c <= 0.03928 {
            c / 12.92
        } else {
            ((c + 0.055) / 1.055).powf(2.4)
        }
    };
    let light = |c: Rgb| 0.2126 * channel(c.0) + 0.7152 * channel(c.1) + 0.0722 * channel(c.2);
    let (a, b) = (light(a), light(b));
    (a.max(b) + 0.05) / (a.min(b) + 0.05)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Light is Catppuccin Latte's surfaces: base, mantle and crust for the editor, the
    /// sidebars and the frame, and menus white. Its code colours are Latte's hues deepened,
    /// since Latte's own green is 3 to 1 on base and its yellow less.
    #[test]
    fn the_light_theme_is_catppuccin_latte() {
        let latte = Theme::named(Some("light"));
        assert_eq!(latte.text.bg, Rgb(0xEF, 0xF1, 0xF5), "base");
        assert_eq!(
            latte.sidebar.bg, latte.text.bg,
            "the side pane is a sheet too"
        );
        assert_eq!(
            latte.on_frame().sidebar.bg,
            latte.activity.bg,
            "the sidebar is the frame"
        );
        assert_eq!(latte.activity.bg, Rgb(0xDC, 0xE0, 0xE8), "crust");
        assert_eq!(latte.popup, Rgb(0xFF, 0xFF, 0xFF), "menus");
        assert_eq!(latte.text.fg, Rgb(0x4C, 0x4F, 0x69), "Latte's text");
        for (name, color) in [
            ("comment", latte.comment),
            ("keyword", latte.keyword),
            ("string", latte.string),
            ("function", latte.function),
            ("type", latte.kind),
            ("number", latte.number),
            ("quiet text", latte.gutter.fg),
        ] {
            let ratio = contrast(color, latte.text.bg);
            assert!(ratio >= 4.5, "{name} is {ratio:.2} to 1 on base");
        }
    }

    #[test]
    fn the_frame_is_the_terminal_background() {
        let blue = Rgb(30, 30, 46);
        let theme = Theme::default().on_background(blue);
        assert_eq!(
            theme.activity.bg, blue,
            "so no seam shows at the window's edge"
        );
        assert_eq!(theme.status.bg, blue);
        assert_ne!(theme.text.bg, blue, "the sheets stand apart from the frame");
        assert_eq!(
            theme.sidebar.bg, theme.text.bg,
            "the side pane is a sheet as the file is"
        );
        assert_eq!(
            theme.on_frame().sidebar.bg,
            blue,
            "and the sidebar is the frame"
        );
    }

    #[test]
    fn a_light_theme_meets_a_light_terminal_without_a_seam() {
        // Latte's base, as a light terminal's background.
        let base = Rgb(0xEF, 0xF1, 0xF5);
        let theme = Theme::named(Some("light")).on_background(base);
        assert_eq!(
            theme.activity.bg, base,
            "the window's padding meets the rail"
        );
        assert_eq!(theme.status.bg, base);
        assert!(brightness(theme.text.bg) > brightness(theme.activity.bg));
        assert_eq!(theme.sidebar.bg, theme.text.bg, "the side pane is a sheet");
        assert_eq!(
            theme.on_frame().sidebar.bg,
            base,
            "the sidebar is the frame"
        );
        // Surfaces stop at white; what's drawn on them keeps its step from them.
        let latte = Theme::named(Some("light"));
        assert_eq!(theme.popup, latte.popup, "menus were white already");
        assert_eq!(
            theme.pick, latte.pick,
            "so their highlight stays where it was"
        );
        assert_eq!(theme.popup_hover, latte.popup_hover);
        assert_eq!(theme.input, latte.input);
        // Its words and washes are Latte's, whatever the dark theme does with its own.
        assert_eq!(theme.gutter.fg, latte.gutter.fg);
        assert_eq!(theme.status.fg, latte.status.fg);
        assert_eq!(theme.added, latte.added);
        assert_eq!(theme.selection, latte.selection);
        // On the editor, code blocks and lines keep about the step they had in Latte.
        let step = |t: &Theme, wash: Rgb| brightness(t.text.bg) as i32 - brightness(wash) as i32;
        for (now, then) in [
            (step(&theme, theme.code_bg), step(&latte, latte.code_bg)),
            (step(&theme, theme.border), step(&latte, latte.border)),
        ] {
            assert!((now - then).abs() <= 3 && now > 0, "{now} against {then}");
        }
    }

    #[test]
    fn the_surfaces_keep_their_order_on_any_terminal() {
        let brown = Rgb(60, 40, 30);
        let theme = Theme::default().on_background(brown);
        assert!(brightness(theme.text.bg) > brightness(theme.activity.bg));
        assert_eq!(
            theme.on_frame().sidebar.bg,
            brown,
            "the sidebar is the frame"
        );
        let side = theme.on_frame();
        assert!(
            brightness(side.row_hover) > brightness(side.sidebar.bg),
            "and a row under the pointer stands out of it"
        );
    }

    /// The dark theme's text, code and meaning read at 4.5 to 1 on the sheet and on the
    /// caret's line, its quiet words on the frame and on a menu, and its surfaces are the
    /// design's: the frame, and the sheets on it.
    #[test]
    fn the_dark_theme_is_three_surfaces_and_text_that_reads_on_them() {
        let dark = Theme::default();
        assert_eq!(dark.activity.bg, Rgb(0x10, 0x11, 0x14), "the frame");
        assert_eq!(dark.text.bg, Rgb(0x17, 0x19, 0x1d), "the sheet");
        assert!(
            brightness(dark.popup) > brightness(dark.text.bg),
            "a menu is raised"
        );
        assert!(
            brightness(dark.input) < brightness(dark.text.bg),
            "a box is sunk"
        );
        assert_eq!(
            dark.sidebar.bg, dark.text.bg,
            "the side pane is a sheet too"
        );
        assert_eq!(
            dark.on_frame().sidebar.bg,
            dark.activity.bg,
            "the sidebar is the frame"
        );
        assert_eq!(dark.tab.bg, dark.text.bg, "tabs stand on their sheet");
        assert_eq!(
            dark.tab_active.bg, dark.text.bg,
            "the one in front on its sheet"
        );
        for (name, color) in [
            ("text", dark.text.fg),
            ("comment", dark.comment),
            ("keyword", dark.keyword),
            ("string", dark.string),
            ("function", dark.function),
            ("type", dark.kind),
            ("number", dark.number),
            ("quiet text", dark.gutter.fg),
            ("a failure", dark.error),
            ("the blue", dark.accent),
            ("what needs you", dark.attention),
            ("what's added", dark.git_added),
            ("what changed", dark.git_modified),
        ] {
            let ratio = contrast(color, dark.text.bg);
            assert!(ratio >= 4.5, "{name} is {ratio:.2} to 1 on the sheet");
            let ratio = contrast(color, dark.line);
            assert!(
                ratio >= 4.5,
                "{name} is {ratio:.2} to 1 on the caret's line"
            );
        }
        for (name, color) in [
            ("quiet text", dark.status.fg),
            ("an icon", dark.activity.fg),
        ] {
            let ratio = contrast(color, dark.activity.bg);
            assert!(ratio >= 4.5, "{name} is {ratio:.2} to 1 on the frame");
        }
        for (name, color, ground) in [
            ("a menu's words", dark.text.fg, dark.popup),
            ("its quiet words", dark.popup_dim, dark.popup),
            ("the picked row's", dark.text.fg, dark.pick),
            ("a button's words", dark.button_text, dark.button),
        ] {
            let ratio = contrast(color, ground);
            assert!(ratio >= 4.5, "{name} are {ratio:.2} to 1");
        }
    }

    /// The dark theme is herdr's graphite, as herdr-app's `settings.rs` works it out from the
    /// preset's palette and as herdr.dev shows it.
    #[test]
    fn the_dark_theme_is_herdrs_graphite() {
        let dark = Theme::default();
        let side = dark.on_frame();
        assert_eq!(dark.activity.bg, Rgb(0x10, 0x11, 0x14), "herdr's chrome");
        assert_eq!(side.sidebar.bg, dark.activity.bg, "under the sidebar too");
        assert_eq!(dark.text.bg, Rgb(0x17, 0x19, 0x1d), "graphite's background");
        assert_eq!(dark.popup, Rgb(0x25, 0x27, 0x2b), "herdr's popover");
        assert_eq!(dark.text.fg, Rgb(0xee, 0xee, 0xef), "herdr's words");
        assert_eq!(dark.status.fg, Rgb(0x98, 0x99, 0x9b), "herdr's muted");
        assert_eq!(dark.disabled, Rgb(0x69, 0x6a, 0x6d), "herdr's dim");
        assert_eq!(
            dark.selection,
            Rgb(0x39, 0x44, 0x54),
            "graphite's selection"
        );
        assert_eq!(
            dark.accent,
            Rgb(0x76, 0xa8, 0xff),
            "the blue herdr.dev turns in"
        );
        assert_eq!(
            dark.focus,
            Rgb(0x96, 0xb2, 0xd7),
            "the ring round a focused pane"
        );
        assert_eq!(
            dark.attention,
            Rgb(0xf0, 0x70, 0x5f),
            "blocked: one needs you"
        );
        assert_eq!(dark.git_added, Rgb(0x4c, 0xc4, 0x8c), "done, and added");
        assert_eq!(dark.git_modified, Rgb(0xd8, 0xbe, 0x87), "changed");
        assert_eq!(dark.error, Rgb(0xdc, 0x8c, 0x95), "removed or failed");
        // herdr lays white over its ground: 9.4% for a hairline, measured on herdr.dev, and
        // 5.5% under the pointer in herdr's code. What's picked is the blue at 16% over its
        // ground, as herdr.dev's picked rows are blue: herdr's white at 9% read as a hover.
        let over = |ground: Rgb, share: f32| mix(ground, Rgb(255, 255, 255), share);
        let blue = |ground: Rgb| mix(ground, dark.accent, 0.16);
        let near = |a: Rgb, b: Rgb| {
            a.0.abs_diff(b.0) <= 1 && a.1.abs_diff(b.1) <= 1 && a.2.abs_diff(b.2) <= 1
        };
        // On the frame, a picked row is the sheet's moved as far as the ground did: within a
        // step or two of the blue laid over the frame itself.
        let (now, wanted) = (side.row_active, blue(side.sidebar.bg));
        assert!(
            now.0.abs_diff(wanted.0) <= 2
                && now.1.abs_diff(wanted.1) <= 2
                && now.2.abs_diff(wanted.2) <= 2,
            "a picked row on the frame is {now:?}, not {wanted:?}"
        );
        for (name, now, wanted) in [
            (
                "a pointed row",
                side.row_hover,
                over(side.sidebar.bg, 0.055),
            ),
            ("a hairline", dark.border, over(dark.text.bg, 0.094)),
            ("a menu's line", dark.popup_border, over(dark.popup, 0.094)),
            ("a menu's pick", dark.pick, blue(dark.popup)),
            (
                "a picked row on a sheet",
                dark.row_active,
                blue(dark.text.bg),
            ),
            ("a menu's hover", dark.popup_hover, over(dark.popup, 0.055)),
        ] {
            assert!(near(now, wanted), "{name} is {now:?}, not {wanted:?}");
        }
    }

    /// On a terminal whose background isn't herdr's, the frame is that background and every
    /// surface keeps its step from herdr's frame.
    #[test]
    fn on_another_terminal_the_surfaces_keep_herdrs_steps() {
        let dark = Theme::default();
        let step = |theme: &Theme, color: Rgb| {
            let base = theme.activity.bg;
            (
                i16::from(color.0) - i16::from(base.0),
                i16::from(color.1) - i16::from(base.1),
                i16::from(color.2) - i16::from(base.2),
            )
        };
        let other = Rgb(0x28, 0x2c, 0x34);
        let moved = dark.on_background(other);
        assert_eq!(moved.activity.bg, other, "the frame is the terminal's");
        assert_eq!(
            moved.text.bg,
            Rgb(0x2f, 0x34, 0x3d),
            "the sheet 7, 8 and 9 over it"
        );
        assert_eq!(moved.popup, Rgb(0x3d, 0x42, 0x4b), "a menu 21 to 23");
        for (name, now, then) in [
            ("the sheet", moved.text.bg, dark.text.bg),
            ("a menu", moved.popup, dark.popup),
            ("a box", moved.input, dark.input),
            ("a hairline", moved.border, dark.border),
            ("a picked row", moved.row_active, dark.row_active),
            ("a selection", moved.selection, dark.selection),
            ("a line added", moved.added, dark.added),
            ("a line removed", moved.removed, dark.removed),
            ("a match", moved.found, dark.found),
        ] {
            assert_eq!(step(&moved, now), step(&dark, then), "{name}");
        }
        assert_eq!(moved.text.fg, dark.text.fg, "words keep their colours");
        assert_eq!(moved.accent, dark.accent);
        // Quiet words on a ground that came up toward them are lifted until they read as well
        // as they did on herdr's.
        for (name, (now, ground), (then, was)) in [
            (
                "a line's number",
                (moved.gutter.fg, moved.text.bg),
                (dark.gutter.fg, dark.text.bg),
            ),
            (
                "a comment",
                (moved.comment, moved.text.bg),
                (dark.comment, dark.text.bg),
            ),
            (
                "a tab behind",
                (moved.tab.fg, moved.text.bg),
                (dark.tab.fg, dark.text.bg),
            ),
            (
                "the bar's words",
                (moved.status.fg, moved.activity.bg),
                (dark.status.fg, dark.activity.bg),
            ),
            (
                "a menu's quiet words",
                (moved.popup_dim, moved.popup),
                (dark.popup_dim, dark.popup),
            ),
            (
                "what can't be done",
                (moved.disabled, moved.text.bg),
                (dark.disabled, dark.text.bg),
            ),
        ] {
            let (ratio, wanted) = (contrast(now, ground), contrast(then, was));
            assert!(
                ratio >= wanted,
                "{name} is {ratio:.2} to 1, where it was {wanted:.2}"
            );
        }
        // On black, everything only moves away from them, and nothing is lifted.
        let black = dark.on_background(Rgb(0, 0, 0));
        assert_eq!(black.gutter.fg, dark.gutter.fg);
        assert_eq!(black.status.fg, dark.status.fg);
    }

    /// A terminal with only 256 colours maps each one Cellix writes to the nearest of its
    /// own. Graphite's surfaces are nearly grey, so the frame, a sheet and a menu land on
    /// three steps of its grey ramp, in their order.
    #[test]
    fn the_dark_surfaces_stay_apart_in_256_colours() {
        let table: Vec<Rgb> = (16u8..=255)
            .map(|index| match index {
                16..=231 => {
                    let level = |n: u8| if n == 0 { 0 } else { 55 + n * 40 };
                    let n = index - 16;
                    Rgb(level(n / 36), level(n / 6 % 6), level(n % 6))
                }
                _ => {
                    let grey = 8 + (index - 232) * 10;
                    Rgb(grey, grey, grey)
                }
            })
            .collect();
        let nearest = |c: Rgb| {
            let far = |t: &Rgb| {
                let d = |a: u8, b: u8| (i32::from(a) - i32::from(b)).pow(2);
                d(c.0, t.0) + d(c.1, t.1) + d(c.2, t.2)
            };
            *table.iter().min_by_key(|t| far(t)).unwrap()
        };
        let dark = Theme::default();
        let (frame, sheet, menu) = (
            nearest(dark.activity.bg),
            nearest(dark.text.bg),
            nearest(dark.popup),
        );
        assert!(
            brightness(frame) < brightness(sheet) && brightness(sheet) < brightness(menu),
            "{frame:?} {sheet:?} {menu:?}"
        );
    }

    fn brightness(color: Rgb) -> u32 {
        (color.0 as u32 + color.1 as u32 + color.2 as u32) / 3
    }

    /// A panel left in the other theme's colours stands out as a block.
    #[test]
    fn every_panel_belongs_to_its_theme() {
        let light = Theme::named(Some("light"));
        for panel in [
            light.text,
            light.sidebar,
            light.activity,
            light.tab,
            light.tab_active,
        ] {
            assert!(brightness(panel.bg) > 200, "{panel:?}");
        }
        let dark = Theme::default();
        for panel in [
            dark.text,
            dark.sidebar,
            dark.activity,
            dark.tab,
            dark.tab_active,
        ] {
            assert!(brightness(panel.bg) < 80, "{panel:?}");
        }
    }

    /// One kind of picked: in a list on the frame or a sheet, in a palette or a menu, it's the
    /// blue over its ground, and what's under the pointer stays the white herdr lays, so the
    /// two can't be taken for each other (they were 1.10 to 1 apart, both white). Find's other
    /// matches are the yellow and plain to see, and the one it's on is the blue, set apart
    /// from them by its colour and from the sheet as plainly.
    #[test]
    fn picked_pointed_and_found_each_look_their_own() {
        let dark = Theme::default();
        let side = dark.on_frame();
        let blue = |c: Rgb| i16::from(c.2) - i16::from(c.0) >= 20;
        let yellow = |c: Rgb| i16::from(c.0) - i16::from(c.2) >= 10;
        for (name, picked, pointed, ground) in [
            (
                "on the sidebar",
                side.row_active,
                side.row_hover,
                side.sidebar.bg,
            ),
            ("on a sheet", dark.row_active, dark.page_hover, dark.text.bg),
            ("in a menu", dark.pick, dark.popup_hover, dark.popup),
            ("a commented span", dark.tint, dark.page_hover, dark.text.bg),
        ] {
            assert!(blue(picked), "{name}: picked {picked:?} is the blue");
            assert!(!blue(pointed), "{name}: pointed {pointed:?} isn't");
            assert!(
                contrast(picked, ground) > contrast(pointed, ground),
                "{name}: picked stands further off its ground than pointed"
            );
            let ratio = contrast(dark.text.fg, picked);
            assert!(ratio >= 4.5, "{name}: words on it {ratio:.2} to 1");
        }
        let sheet = dark.text.bg;
        let (found, on) = (dark.found, dark.current_match());
        assert!(yellow(found), "other matches {found:?} are the yellow");
        assert!(blue(on), "the one Find is on {on:?} is the blue");
        assert_ne!(on, dark.selection, "and not a selection's colour");
        for (name, color) in [("another match", found), ("the match it's on", on)] {
            let ratio = contrast(color, sheet);
            assert!(ratio >= 1.8, "{name} is {ratio:.2} to 1 on the sheet");
            let ratio = contrast(dark.text.fg, color);
            assert!(ratio >= 4.5, "words on {name} are {ratio:.2} to 1");
        }
        // Light keeps its own: the blue it picks in, and the selection's for the match.
        let light = Theme::named(Some("light"));
        assert_eq!(light.current_match(), light.selection);
        assert_eq!(light.row_active, Rgb(0xC8, 0xD6, 0xF8));
    }

    /// A text box without the keyboard shows where it is on any ground: its line reads at 1.8 to
    /// 1 or better on the frame, a sheet and a menu, where a menu's line was 1.69 on the frame
    /// and the sunk fill 1.03, and an empty Replace box all but went. A menu that opens over a
    /// card, a menu's colour, stands a step above it, and its hover and pick go up with it.
    #[test]
    fn a_box_and_a_menu_over_a_card_stand_off_their_ground() {
        let dark = Theme::default();
        for (name, ground) in [
            ("the frame", dark.activity.bg),
            ("a sheet", dark.text.bg),
            ("a menu", dark.popup),
        ] {
            let ratio = contrast(dark.field_line(), ground);
            assert!(ratio >= 1.8, "a box's line on {name} is {ratio:.2} to 1");
        }
        let light = Theme::named(Some("light"));
        let ratio = contrast(light.field_line(), light.popup);
        assert!(ratio >= 1.8, "light, on a menu {ratio:.2} to 1");
        let over = dark.over_card();
        let ratio = contrast(over.popup, dark.popup);
        assert!(
            ratio >= 1.15,
            "a menu over a card is {ratio:.2} to 1 off it"
        );
        // And above what's set on a card in a quiet button's fill, as an agent's answer is.
        let answer = dark.quiet_button(dark.popup);
        assert!(
            brightness(over.popup) > brightness(answer),
            "{:?} over {answer:?}",
            over.popup
        );
        for (name, now, was) in [
            ("its hover", over.popup_hover, dark.popup_hover),
            ("its pick", over.pick, dark.pick),
            ("its line", over.popup_border, dark.popup_border),
        ] {
            assert!(brightness(now) > brightness(was), "{name} goes up with it");
        }
        assert!(
            contrast(over.popup_dim, over.popup) >= 4.5,
            "its quiet words still read"
        );
        assert_eq!(light.over_card().popup, light.popup, "light keeps white");
    }

    /// Every action that isn't the main one is one quiet kind of button, on the frame, a sheet,
    /// a menu or a card: it stands off its ground, and under the pointer it moves a step
    /// further off as plainly. A hover that took the menu's wash was darker than the button
    /// at rest on a card, and the New Workspace button's was 1.07 to 1 from its rest.
    #[test]
    fn a_quiet_buttons_hover_shows_on_every_ground() {
        for theme in [Theme::default(), Theme::named(Some("light"))] {
            // A suggestion's card: the accent at 6% over a card.
            let suggested = mix(theme.popup, theme.accent, 0.06);
            for (name, ground) in [
                ("the frame", theme.activity.bg),
                ("a sheet", theme.text.bg),
                ("a menu or a card", theme.popup),
                ("a suggestion's card", suggested),
            ] {
                let (rest, over) = (
                    theme.secondary(ground, false),
                    theme.secondary(ground, true),
                );
                let (off, step) = (contrast(rest, ground), contrast(over, rest));
                assert!(off >= 1.2, "{name}: at rest {off:.2} to 1 off its ground");
                assert!(
                    step >= 1.2,
                    "{name}: pointed at, {step:.2} to 1 off its rest"
                );
                assert!(
                    contrast(over, ground) > off,
                    "{name}: further from its ground"
                );
            }
        }
    }

    #[test]
    fn activity_icons_stay_readable() {
        for theme in [Theme::named(Some("light")), Theme::default()] {
            let bg = brightness(theme.activity.bg) as i32;
            // The one showing is the text colour on its chip.
            let chip = brightness(theme.tint) as i32;
            assert!((brightness(theme.text.fg) as i32 - chip).abs() > 120);
            assert!((brightness(theme.activity.fg) as i32 - bg).abs() > 60);
        }
    }
}

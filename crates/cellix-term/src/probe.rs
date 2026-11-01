/// How frames reach the terminal. See the tier table in `docs/DESIGN.md`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tier {
    PixelsDirect,
    PixelsPlaceholder,
    Cells,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Multiplexer {
    None,
    Tmux,
    Zellij,
}

/// What detection found about the environment.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Env {
    pub multiplexer: Multiplexer,
    /// The terminal answered a Kitty graphics query.
    pub kitty_graphics: bool,
    /// tmux reports `allow-passthrough on`.
    pub tmux_passthrough: bool,
    /// Cell size in pixels, when the terminal answered. `TIOCGWINSZ` is the fallback.
    pub cell: Option<(usize, usize)>,
}

impl Multiplexer {
    /// Reads `$TMUX` and `$ZELLIJ` through `var`, so it can be tested without the
    /// process environment.
    pub fn detect(var: impl Fn(&str) -> Option<String>) -> Self {
        if var("TMUX").is_some_and(|v| !v.is_empty()) {
            Self::Tmux
        } else if var("ZELLIJ").is_some() {
            Self::Zellij
        } else {
            Self::None
        }
    }
}

impl std::str::FromStr for Tier {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, String> {
        match s {
            "direct" => Ok(Self::PixelsDirect),
            "placeholder" => Ok(Self::PixelsPlaceholder),
            "cells" => Ok(Self::Cells),
            _ => Err(format!(
                "unknown tier {s}: use direct, placeholder or cells"
            )),
        }
    }
}

/// Picks a tier. A user setting wins. Otherwise this is the design's table: tmux needs
/// placeholders, and anything else that speaks Kitty graphics gets direct placement.
pub fn select_tier(env: Env, forced: Option<Tier>) -> Tier {
    if let Some(tier) = forced {
        return tier;
    }
    match env.multiplexer {
        Multiplexer::Tmux if env.tmux_passthrough && env.kitty_graphics => Tier::PixelsPlaceholder,
        Multiplexer::Tmux => Tier::Cells,
        Multiplexer::Zellij | Multiplexer::None if env.kitty_graphics => Tier::PixelsDirect,
        Multiplexer::Zellij | Multiplexer::None => Tier::Cells,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env(multiplexer: Multiplexer, kitty_graphics: bool, tmux_passthrough: bool) -> Env {
        Env {
            multiplexer,
            kitty_graphics,
            tmux_passthrough,
            cell: None,
        }
    }

    #[test]
    fn tier_selection_matches_design_table() {
        use Multiplexer::*;
        assert_eq!(
            select_tier(env(None, true, false), Option::None),
            Tier::PixelsDirect
        );
        assert_eq!(
            select_tier(env(None, false, false), Option::None),
            Tier::Cells
        );
        assert_eq!(
            select_tier(env(Tmux, true, true), Option::None),
            Tier::PixelsPlaceholder
        );
        assert_eq!(
            select_tier(env(Tmux, true, false), Option::None),
            Tier::Cells
        );
        assert_eq!(
            select_tier(env(Tmux, false, true), Option::None),
            Tier::Cells
        );
        assert_eq!(
            select_tier(env(Zellij, true, false), Option::None),
            Tier::PixelsDirect
        );
        assert_eq!(
            select_tier(env(Zellij, false, false), Option::None),
            Tier::Cells
        );
    }

    #[test]
    fn a_setting_forces_the_tier() {
        let e = env(Multiplexer::None, true, false);
        assert_eq!(select_tier(e, Some(Tier::Cells)), Tier::Cells);
        assert_eq!("cells".parse(), Ok(Tier::Cells));
        assert!("nope".parse::<Tier>().is_err());
    }

    #[test]
    fn multiplexer_comes_from_the_environment() {
        let tmux = |k: &str| (k == "TMUX").then(|| "/tmp/tmux-501/default,1,0".to_string());
        assert_eq!(Multiplexer::detect(tmux), Multiplexer::Tmux);
        let zellij = |k: &str| (k == "ZELLIJ").then(|| "0".to_string());
        assert_eq!(Multiplexer::detect(zellij), Multiplexer::Zellij);
        assert_eq!(Multiplexer::detect(|_| None), Multiplexer::None);
    }
}

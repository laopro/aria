//! Colors of the interface, picked once from `theme` in the config file.

use once_cell::sync::Lazy;
use ratatui::style::Color;

use crate::consts::CONFIG;

pub struct Theme {
    pub accent: Color,
    pub accent_bg: Color,
    pub bg: Color,
    pub border: Color,
    pub text: Color,
    pub text_dim: Color,
    /// Background of the row or button under the mouse.
    pub hover: Color,
}

pub static THEME: Lazy<Theme> = Lazy::new(|| match CONFIG.player.theme.as_str() {
    "mono" => Theme {
        accent: Color::White,
        accent_bg: Color::Indexed(239),
        bg: Color::Black,
        border: Color::DarkGray,
        text: Color::White,
        text_dim: Color::Gray,
        hover: Color::Indexed(236),
    },
    "light" => Theme {
        accent: Color::Red,
        accent_bg: Color::Indexed(224),
        bg: Color::Indexed(255),
        border: Color::Indexed(248),
        text: Color::Black,
        text_dim: Color::Indexed(242),
        hover: Color::Indexed(253),
    },
    _ => Theme {
        accent: Color::LightRed,
        accent_bg: Color::Indexed(52),
        bg: Color::Black,
        border: Color::DarkGray,
        text: Color::White,
        text_dim: Color::Gray,
        hover: Color::Indexed(236),
    },
});

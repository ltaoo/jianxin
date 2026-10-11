//! Native UI values are read from the same living contract as the web reader.
use gpui_kit::{
    component::{Theme, ThemeMode},
    *,
};

pub struct Tokens {
    contract: serde_json::Value,
    dark: bool,
}

impl Tokens {
    pub fn load() -> Self {
        Self {
            contract: serde_json::from_str(include_str!("../../../webpc/.design"))
                .expect("valid design contract"),
            dark: false,
        }
    }
    pub fn color(&self, name: &str) -> Hsla {
        let value = self.color_value(name);
        rgb(u32::from_str_radix(value.trim_start_matches('#'), 16).expect("hex color")).into()
    }
    pub fn length(&self, group: &str, name: &str) -> Pixels {
        let value = self.contract["tokens"][group][name]
            .as_str()
            .expect("length token");
        if let Some(value) = value.strip_suffix("rem") {
            px(value.parse::<f32>().expect("rem value") * 16.)
        } else {
            px(value.trim_end_matches("px").parse().expect("pixel value"))
        }
    }
    fn color_value(&self, name: &str) -> &str {
        let dark = &self.contract["themes"]["desktop"]["dark"]["color"];
        (if self.dark { dark[name].as_str() } else { None })
            .or_else(|| self.contract["tokens"]["color"][name].as_str())
            .expect("color token")
    }
    pub fn set_appearance(&mut self, appearance: WindowAppearance) {
        self.dark = matches!(
            appearance,
            WindowAppearance::Dark | WindowAppearance::VibrantDark
        );
    }
    pub fn apply(&mut self, appearance: WindowAppearance, cx: &mut App) {
        self.set_appearance(appearance);
        Theme::change(
            if self.dark {
                ThemeMode::Dark
            } else {
                ThemeMode::Light
            },
            None,
            cx,
        );
        Theme::update(cx, |theme| {
            theme.colors.background = self.color("canvas");
            theme.colors.foreground = self.color("text");
            theme.colors.primary = self.color("primary");
            theme.colors.primary_hover = self.color("primaryHover");
            theme.colors.primary_foreground = self.color("primaryForeground");
            theme.colors.secondary = self.color("fill");
            theme.colors.secondary_hover = self.color("hover");
            theme.colors.muted = self.color("fill");
            theme.colors.popover = self.color("canvas");
            theme.colors.popover_foreground = self.color("text");
            theme.colors.secondary_foreground = self.color("text");
            theme.colors.muted_foreground = self.color("muted");
            theme.colors.accent = self.color("hover");
            theme.colors.accent_foreground = self.color("text");
            theme.colors.border = self.color("line");
            theme.colors.ring = self.color("focus");
            theme.radius = self.length("radius", "control");
            theme.font_size = self.length("type", "control");
        });
    }
}

#[cfg(test)]
mod tests {
    use super::Tokens;
    use gpui_kit::WindowAppearance;

    #[test]
    fn appearance_selects_desktop_colors_and_returns_to_original_light_tokens() {
        let mut tokens = Tokens::load();
        let light = tokens.color_value("canvas").to_owned();
        for appearance in [WindowAppearance::Dark, WindowAppearance::VibrantDark] {
            tokens.set_appearance(appearance);
            assert_ne!(tokens.color_value("canvas"), light);
            assert_eq!(tokens.color_value("primaryForeground"), "#ffffff");
        }
        for appearance in [WindowAppearance::Light, WindowAppearance::VibrantLight] {
            tokens.set_appearance(appearance);
            assert_eq!(tokens.color_value("canvas"), light);
        }
    }

    fn luminance(color: &str) -> f64 {
        let channels = [1, 3, 5].map(|offset| {
            let value = u8::from_str_radix(&color[offset..offset + 2], 16).unwrap() as f64 / 255.;
            if value <= 0.04045 {
                value / 12.92
            } else {
                ((value + 0.055) / 1.055).powf(2.4)
            }
        });
        channels[0] * 0.2126 + channels[1] * 0.7152 + channels[2] * 0.0722
    }

    #[test]
    fn panel_text_and_button_labels_have_readable_contrast_in_both_modes() {
        let mut tokens = Tokens::load();
        for appearance in [WindowAppearance::Light, WindowAppearance::Dark] {
            tokens.set_appearance(appearance);
            for (foreground, background) in [
                ("text", "canvas"),
                ("muted", "canvas"),
                ("danger", "canvas"),
                ("text", "hover"),
                ("primaryForeground", "primary"),
                ("primaryForeground", "primaryHover"),
            ] {
                let a = luminance(tokens.color_value(foreground));
                let b = luminance(tokens.color_value(background));
                let contrast = (a.max(b) + 0.05) / (a.min(b) + 0.05);
                assert!(
                    contrast >= 4.5,
                    "{appearance:?}: {foreground}/{background} contrast {contrast}"
                );
            }
        }
    }
}

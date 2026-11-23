//! Anime4K v4 modes and effects (`GLSL_Instructions_Advanced.md` and the two official templates,
//! "lower-end" = fast, "higher-end" = quality; shader files in `shaders/anime4k`, MIT, except the
//! two `AutoDownscalePre`, Unlicense), plus the color adjustments.
//!
//! The effects are the official shaders with only their `#define` strength changed per level.

use serde::{Deserialize, Serialize};

macro_rules! shader {
    ($name:literal) => {
        Shader { name: $name, text: include_str!(concat!("shaders/anime4k/Anime4K_", $name, ".glsl")) }
    };
}

#[derive(Clone, Copy)]
struct Shader {
    name: &'static str,
    text: &'static str,
}

const CLAMP: Shader = shader!("Clamp_Highlights");
const RESTORE_S: Shader = shader!("Restore_CNN_S");
const RESTORE_M: Shader = shader!("Restore_CNN_M");
const RESTORE_VL: Shader = shader!("Restore_CNN_VL");
const SOFT_S: Shader = shader!("Restore_CNN_Soft_S");
const SOFT_M: Shader = shader!("Restore_CNN_Soft_M");
const SOFT_VL: Shader = shader!("Restore_CNN_Soft_VL");
const UPSCALE_S: Shader = shader!("Upscale_CNN_x2_S");
const UPSCALE_M: Shader = shader!("Upscale_CNN_x2_M");
const UPSCALE_VL: Shader = shader!("Upscale_CNN_x2_VL");
const UPSCALE_DENOISE_M: Shader = shader!("Upscale_Denoise_CNN_x2_M");
const UPSCALE_DENOISE_VL: Shader = shader!("Upscale_Denoise_CNN_x2_VL");
const DOWNSCALE_X2: Shader = shader!("AutoDownscalePre_x2");
const DOWNSCALE_X4: Shader = shader!("AutoDownscalePre_x4");

struct Effect {
    shader: Shader,
    /// The line that carries the strength.
    define: &'static str,
    levels: [&'static str; 3],
}

const DENOISE: Effect = Effect { shader: shader!("Denoise_Bilateral_Mode"), define: "#define INTENSITY_SIGMA 0.1", levels: ["0.06", "0.1", "0.16"] };
const DEBLUR: Effect = Effect { shader: shader!("Deblur_DoG"), define: "#define STRENGTH 0.6", levels: ["0.6", "1.0", "1.5"] };
const THIN: Effect = Effect { shader: shader!("Thin_HQ"), define: "#define STRENGTH 0.6", levels: ["0.3", "0.6", "1.0"] };
const DARKEN: Effect = Effect { shader: shader!("Darken_HQ"), define: "#define STRENGTH 1.5", levels: ["0.8", "1.5", "2.5"] };

pub(crate) const MAX_EFFECT: i64 = 3;
const COLOR_RANGE: i64 = 50;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum AnimeMode {
    /// By the video's resolution, as the official guide: 1080p = A, 720p = B, SD = A+A (A when fast).
    #[default]
    Auto,
    A,
    B,
    C,
    Aa,
    Bb,
    Ca,
}

/// What the user fine-tunes on top of the level. Effects 0–3; colors −50–50.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Picture {
    pub mode: AnimeMode,
    pub denoise: i64,
    pub deblur: i64,
    pub thin: i64,
    pub darken: i64,
    pub deband: i64,
    pub brightness: i64,
    pub contrast: i64,
    pub saturation: i64,
    pub gamma: i64,
    pub hue: i64,
}

impl Picture {
    pub(crate) fn sanitized(mut self) -> Self {
        for value in [&mut self.denoise, &mut self.deblur, &mut self.thin, &mut self.darken, &mut self.deband] {
            *value = (*value).clamp(0, MAX_EFFECT);
        }
        for value in [&mut self.brightness, &mut self.contrast, &mut self.saturation, &mut self.gamma, &mut self.hue] {
            *value = (*value).clamp(-COLOR_RANGE, COLOR_RANGE);
        }
        self
    }

    pub(crate) fn has_effects(&self) -> bool {
        self.denoise > 0 || self.deblur > 0 || self.thin > 0 || self.darken > 0
    }

    pub(crate) fn without_effects(self) -> Self {
        Self { denoise: 0, deblur: 0, thin: 0, darken: 0, ..self }
    }

    pub(crate) fn colors(&self) -> [(&'static str, i64); 5] {
        [("brightness", self.brightness), ("contrast", self.contrast), ("saturation", self.saturation), ("gamma", self.gamma), ("hue", self.hue)]
    }

    /// mpv `deband-*` of the banding level (iterations, threshold, range, grain).
    pub(crate) fn debanding(&self) -> Option<[(&'static str, &'static str); 5]> {
        let (iterations, threshold, range, grain) = match self.deband {
            1 => ("2", "40", "16", "12"),
            2 => ("3", "56", "18", "16"),
            3 => ("4", "80", "22", "24"),
            _ => return None,
        };
        Some([("deband", "yes"), ("deband-iterations", iterations), ("deband-threshold", threshold), ("deband-range", range), ("deband-grain", grain)])
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Weight {
    Fast,
    Quality,
}

fn resolve(mode: AnimeMode, weight: Weight, lines: Option<i64>) -> AnimeMode {
    if mode != AnimeMode::Auto {
        return mode;
    }
    match lines {
        Some(lines) if lines < 700 && weight == Weight::Quality => AnimeMode::Aa,
        Some(lines) if (700..1000).contains(&lines) => AnimeMode::B,
        _ => AnimeMode::A,
    }
}

/// The templates' lists. No file appears twice (a repeated file misbehaves, says the guide).
fn mode_shaders(mode: AnimeMode, weight: Weight) -> Vec<Shader> {
    let hq = weight == Weight::Quality;
    let pick = |fast: Shader, quality: Shader| if hq { quality } else { fast };
    let (restore, soft) = (pick(RESTORE_M, RESTORE_VL), pick(SOFT_M, SOFT_VL));
    let (restore_again, soft_again) = (pick(RESTORE_S, RESTORE_M), pick(SOFT_S, SOFT_M));
    let upscale = pick(UPSCALE_M, UPSCALE_VL);
    let upscale_denoise = pick(UPSCALE_DENOISE_M, UPSCALE_DENOISE_VL);
    let last = pick(UPSCALE_S, UPSCALE_M);
    let (x2, x4) = (DOWNSCALE_X2, DOWNSCALE_X4);
    match mode {
        AnimeMode::Auto | AnimeMode::A => vec![restore, upscale, x2, x4, last],
        AnimeMode::B => vec![soft, upscale, x2, x4, last],
        AnimeMode::C => vec![upscale_denoise, x2, x4, last],
        AnimeMode::Aa => vec![restore, upscale, restore_again, x2, x4, last],
        AnimeMode::Bb => vec![soft, upscale, x2, x4, soft_again, last],
        AnimeMode::Ca => vec![upscale_denoise, x2, x4, restore_again, last],
    }
}

/// The shader list as (file name, text), in processing order. `lines`: the video's height as 16:9.
pub(crate) fn chain(picture: &Picture, anime: Option<Weight>, lines: Option<i64>) -> Vec<(String, String)> {
    let plain = |shader: Shader| (format!("Anime4K_{}.glsl", shader.name), shader.text.to_string());
    let effect = |effect: &Effect, level: i64| {
        let value = effect.levels[(level - 1).clamp(0, 2) as usize];
        let define = effect.define.rsplit_once(' ').map_or(effect.define, |(head, _)| head);
        (format!("Anime4K_{}_{level}.glsl", effect.shader.name), effect.shader.text.replace(effect.define, &format!("{define} {value}")))
    };
    let mut list = Vec::new();
    if anime.is_some() || picture.deblur > 0 {
        list.push(plain(CLAMP)); // prevents the overshoot of both
    }
    if picture.denoise > 0 {
        list.push(effect(&DENOISE, picture.denoise));
    }
    if let Some(weight) = anime {
        list.extend(mode_shaders(resolve(picture.mode, weight, lines), weight).into_iter().map(plain));
    }
    for (kind, level) in [(&DEBLUR, picture.deblur), (&DARKEN, picture.darken), (&THIN, picture.thin)] {
        if level > 0 {
            list.push(effect(kind, level));
        }
    }
    list
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_mode_follows_the_templates_without_repeating_a_file() {
        for weight in [Weight::Fast, Weight::Quality] {
            for mode in [AnimeMode::A, AnimeMode::B, AnimeMode::C, AnimeMode::Aa, AnimeMode::Bb, AnimeMode::Ca] {
                let list = chain(&Picture { mode, ..Default::default() }, Some(weight), Some(1080));
                assert_eq!(list[0].0, "Anime4K_Clamp_Highlights.glsl");
                let names: std::collections::BTreeSet<_> = list.iter().map(|(name, _)| name).collect();
                assert_eq!(names.len(), list.len(), "{mode:?} {weight:?}");
            }
        }
        let names = |list: Vec<(String, String)>| list.into_iter().map(|(n, _)| n.replace("Anime4K_", "").replace(".glsl", "")).collect::<Vec<_>>().join(" ");
        assert_eq!(
            names(chain(&Picture { mode: AnimeMode::Aa, ..Default::default() }, Some(Weight::Quality), None)),
            "Clamp_Highlights Restore_CNN_VL Upscale_CNN_x2_VL Restore_CNN_M AutoDownscalePre_x2 AutoDownscalePre_x4 Upscale_CNN_x2_M"
        );
    }

    #[test]
    fn automatic_mode_follows_the_resolution() {
        assert_eq!(resolve(AnimeMode::Auto, Weight::Quality, Some(1080)), AnimeMode::A);
        assert_eq!(resolve(AnimeMode::Auto, Weight::Fast, Some(720)), AnimeMode::B);
        assert_eq!(resolve(AnimeMode::Auto, Weight::Quality, Some(480)), AnimeMode::Aa);
        assert_eq!(resolve(AnimeMode::Auto, Weight::Fast, Some(480)), AnimeMode::A);
        assert_eq!(resolve(AnimeMode::C, Weight::Fast, Some(1080)), AnimeMode::C);
    }

    #[test]
    fn effect_levels_really_change_the_strength() {
        for effect in [&DENOISE, &DEBLUR, &THIN, &DARKEN] {
            assert!(effect.shader.text.contains(effect.define), "{}", effect.shader.name);
        }
        let weak = chain(&Picture { thin: 1, ..Default::default() }, None, None);
        let strong = chain(&Picture { thin: 3, ..Default::default() }, None, None);
        assert!(weak[0].1.contains("#define STRENGTH 0.3") && strong[0].1.contains("#define STRENGTH 1.0"));
        assert!(chain(&Picture { brightness: 20, deband: 3, ..Default::default() }, None, None).is_empty(), "colors need no shader");
    }
}

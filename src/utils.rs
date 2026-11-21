//! Free helper functions: fonts, OS integration (open / reveal / copy), drawing
//! bits, viewer zoom maths and grouping of the items.

use crate::globals::{DateGran, GroupMode};
use crate::layout::Group;
use crate::scan::Item;
use chrono::{Datelike, Local, TimeZone};
use eframe::egui::{self, pos2, vec2, Color32, Pos2, Rect, Stroke, Vec2};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

pub fn setup_fonts(ctx: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();

    fonts.font_data.insert(
        "departure_mono".to_owned(),
        egui::FontData::from_static(include_bytes!(
            "../assets/fonts/DepartureMonoNerdFontMono-Regular.otf"
        )),
    );

    fonts
        .families
        .entry(egui::FontFamily::Proportional)
        .or_default()
        .insert(0, "departure_mono".to_owned());

    fonts
        .families
        .entry(egui::FontFamily::Monospace)
        .or_default()
        .insert(0, "departure_mono".to_owned());

    ctx.set_fonts(fonts);
}

pub fn reveal_in_file_manager(path: &Path) {
    use std::process::Command;

    #[cfg(target_os = "windows")]
    {
        let _ = Command::new("explorer.exe")
            .arg(format!("/select,{}", path.display()))
            .spawn();
    }

    #[cfg(target_os = "macos")]
    {
        let _ = Command::new("open").arg("-R").arg(path).spawn();
    }

    #[cfg(target_os = "linux")]
    {
        let desktop = std::env::var("XDG_CURRENT_DESKTOP")
            .unwrap_or_default()
            .to_lowercase();

        // Prefer the file manager native to the current desktop.
        let preferred: &[&str] = if desktop.contains("kde") || desktop.contains("plasma") {
            &["dolphin", "thunar", "nemo", "nautilus", "caja"]
        } else if desktop.contains("xfce") {
            &["thunar", "dolphin", "nemo", "nautilus", "caja"]
        } else if desktop.contains("gnome") {
            &["nautilus", "dolphin", "thunar", "nemo", "caja"]
        } else if desktop.contains("cinnamon") {
            &["nemo", "dolphin", "thunar", "nautilus", "caja"]
        } else if desktop.contains("mate") {
            &["caja", "thunar", "dolphin", "nemo", "nautilus"]
        } else {
            &["dolphin", "thunar", "nemo", "nautilus", "caja"]
        };

        for manager in preferred {
            if Command::new(manager)
                .arg("--select")
                .arg(path)
                .spawn()
                .is_ok()
            {
                break;
            }
        }
    }
}

pub fn open_external(path: &Path) {
    use std::process::Command;
    #[cfg(target_os = "linux")]
    let _ = Command::new("xdg-open").arg(path).spawn();
    #[cfg(target_os = "macos")]
    let _ = Command::new("open").arg(path).spawn();
    #[cfg(target_os = "windows")]
    let _ = Command::new("cmd")
        .args(["/C", "start", ""])
        .arg(path)
        .spawn();
}

pub fn copy_full_path(ctx: &egui::Context, path: &Path) {
    let full = std::fs::canonicalize(path)
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_else(|_| path.to_string_lossy().into_owned());
    ctx.copy_text(full);
}

pub const FLASH_SECS: f32 = 1.2;

/// "Here it is": a bright frame that holds for a moment, fades out, and sends
/// a ring outwards. `t` runs 0..1 over the flash.
pub fn paint_flash(p: &egui::Painter, r: Rect, t: f32) {
    let c = crate::search::ACCENT;
    let hold = 0.6;
    let fade = if t < hold {
        1.0
    } else {
        1.0 - (t - hold) / (1.0 - hold)
    };
    p.rect_filled(r, 0.0, c.gamma_multiply(0.2 * fade));
    p.rect_stroke(r, 0.0, Stroke::new(3.0_f32, c.gamma_multiply(fade)));
    let ease = 1.0 - (1.0 - t).powi(3);
    p.rect_stroke(
        r.expand(2.0 + 22.0 * ease),
        4.0 + 8.0 * ease,
        Stroke::new(2.0_f32, c.gamma_multiply(0.7 * (1.0 - t))),
    );
}

/// Decides when overlay controls (the video transport bar) are visible: shown
/// while the pointer is moving, kept for [`AutoHide::DELAY`] after it stops, and
/// never hidden while the pointer is over them - nor while a button that was
/// pressed on them is still held (dragging a slider off the bar).
#[cfg_attr(not(feature = "video-player"), allow(dead_code))]
pub struct AutoHide {
    last_move: Instant,
    held: bool,
}

#[cfg_attr(not(feature = "video-player"), allow(dead_code))]
impl AutoHide {
    pub const DELAY: Duration = Duration::from_millis(200);

    pub fn new(now: Instant) -> Self {
        Self {
            last_move: now,
            held: false,
        }
    }

    /// Call once per frame. `moved`: the pointer moved this frame, `over`: it is
    /// over the controls, `down`: a pointer button is held. Returns whether the
    /// controls should be shown.
    pub fn update(&mut self, now: Instant, moved: bool, over: bool, down: bool) -> bool {
        if moved {
            self.last_move = now;
        }
        self.held = over || (self.held && down);
        self.held || now.saturating_duration_since(self.last_move) < Self::DELAY
    }

    /// How long until the controls hide if nothing happens (for scheduling a repaint).
    pub fn time_left(&self, now: Instant) -> Duration {
        Self::DELAY.saturating_sub(now.saturating_duration_since(self.last_move))
    }
}

/// Largest size with the aspect ratio of `size` that fits inside `avail`.
pub fn fit_within(size: Vec2, avail: Vec2) -> Vec2 {
    size * (avail.x / size.x).min(avail.y / size.y)
}

/// Keep the zoomed image from being dragged away: while it is larger than the
/// window it may only move until its edge meets the window edge, otherwise it
/// stays centred.
pub fn clamp_pan(pan: Vec2, size: Vec2, view: Vec2) -> Vec2 {
    let lim = ((size - view) * 0.5).max(Vec2::ZERO);
    vec2(pan.x.clamp(-lim.x, lim.x), pan.y.clamp(-lim.y, lim.y))
}

/// Zoom the viewer image by factor `k`, keeping the point under `anchor` fixed.
pub fn zoom_at(
    zoom: &mut f32,
    pan: &mut Vec2,
    k: f32,
    anchor: Pos2,
    screen: Rect,
    fit: Vec2,
    max_zoom: f32,
) {
    let z = (*zoom * k).clamp(1.0, max_zoom);
    let k = z / *zoom;
    let rel = anchor - screen.center();
    *pan = rel - (rel - *pan) * k;
    *zoom = z;
    *pan = clamp_pan(*pan, fit * z, screen.size());
}

pub fn gap_for(compact: bool) -> f32 {
    if compact {
        8.0
    } else {
        4.0
    }
}

/// Background shade for a group's frame in compact mode.
/// Cycles through a variety of distinct, very dark colors.
pub fn group_shade(g: u32) -> Color32 {
    const SHADES: [Color32; 8] = [
        Color32::from_rgb(35, 12, 55), // Dark purple
        Color32::from_rgb(10, 20, 55), // Dark blue
        Color32::from_rgb(8, 35, 22),  // Dark green
        Color32::from_rgb(55, 25, 8),  // Dark orange
        Color32::from_rgb(55, 10, 18), // Dark red
        Color32::from_rgb(10, 42, 48), // Dark teal
        Color32::from_rgb(48, 35, 8),  // Dark yellow / gold
        Color32::from_rgb(40, 18, 32), // Dark pink
    ];

    SHADES[g as usize % SHADES.len()]
}
pub fn draw_play(p: &egui::Painter, c: Pos2, s: f32) {
    p.add(egui::Shape::convex_polygon(
        vec![
            pos2(c.x - s * 0.5, c.y - s * 0.7),
            pos2(c.x - s * 0.5, c.y + s * 0.7),
            pos2(c.x + s * 0.8, c.y),
        ],
        Color32::from_white_alpha(230),
        Stroke::NONE,
    ));
}

pub fn build_groups(items: &[Item], mode: GroupMode, gran: DateGran, root: &Path) -> Vec<Group> {
    match mode {
        GroupMode::Folder => {
            let mut m: BTreeMap<PathBuf, Vec<u32>> = BTreeMap::new();
            for (i, it) in items.iter().enumerate() {
                m.entry(it.path.parent().unwrap_or(Path::new("")).to_path_buf())
                    .or_default()
                    .push(i as u32);
            }
            m.into_iter()
                .map(|(dir, mut ids)| {
                    ids.sort_by_cached_key(|&i| {
                        items[i as usize]
                            .path
                            .file_name()
                            .map(|n| n.to_string_lossy().to_lowercase())
                    });
                    let rel = dir.strip_prefix(root).unwrap_or(&dir);
                    let label = if rel.as_os_str().is_empty() {
                        root.file_name()
                            .map(|n| n.to_string_lossy().into_owned())
                            .unwrap_or_else(|| root.to_string_lossy().into_owned())
                    } else {
                        rel.to_string_lossy().into_owned()
                    };
                    Group { label, ids }
                })
                .collect()
        }
        GroupMode::Modified | GroupMode::Created => {
            let ts = |it: &Item| {
                if mode == GroupMode::Modified {
                    it.modified
                } else {
                    it.created
                }
            };
            let mut m: BTreeMap<i32, (String, Vec<u32>)> = BTreeMap::new();
            for (i, it) in items.iter().enumerate() {
                let Some(dt) = Local.timestamp_opt(ts(it), 0).single() else {
                    continue;
                };
                let (key, label) = match gran {
                    DateGran::Day => (
                        dt.year() * 10000 + dt.month() as i32 * 100 + dt.day() as i32,
                        dt.format("%a, %b %-d, %Y").to_string(),
                    ),
                    DateGran::Month => (
                        dt.year() * 100 + dt.month() as i32,
                        dt.format("%B %Y").to_string(),
                    ),
                };
                m.entry(key)
                    .or_insert_with(|| (label, vec![]))
                    .1
                    .push(i as u32);
            }
            m.into_values()
                .rev()
                .map(|(label, mut ids)| {
                    ids.sort_by_key(|&i| std::cmp::Reverse(ts(&items[i as usize])));
                    Group { label, ids }
                })
                .collect()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn screen() -> Rect {
        Rect::from_min_size(Pos2::ZERO, vec2(1000.0, 800.0))
    }

    #[test]
    fn controls_hide_when_the_pointer_is_still_and_show_on_movement() {
        // relative to DELAY so retuning it doesn't break the test
        let d = AutoHide::DELAY;
        let t0 = Instant::now();
        let mut h = AutoHide::new(t0);
        assert!(
            h.update(t0, false, false, false),
            "visible right after opening"
        );
        assert!(
            h.update(t0 + d / 2, false, false, false),
            "still within the delay"
        );
        assert!(
            !h.update(t0 + d * 2, false, false, false),
            "hidden once stationary"
        );
        let later = t0 + Duration::from_secs(60);
        assert!(!h.update(later, false, false, false), "stays hidden");
        assert!(
            h.update(later + Duration::from_millis(10), true, false, false),
            "moving brings it back"
        );
        assert!(h.update(later + d / 2, false, false, false));
        assert!(
            !h.update(later + d * 2, false, false, false),
            "and it hides again"
        );
    }

    #[test]
    fn controls_stay_while_the_pointer_is_over_them() {
        let t0 = Instant::now();
        let ms = |n| t0 + Duration::from_millis(n);
        let mut h = AutoHide::new(t0);
        // resting on the bar for a long time: never hides
        for i in 0..100 {
            assert!(h.update(ms(2000 + i * 1000), false, true, false));
        }
        // leaving it (a move) starts the countdown from that moment
        assert!(h.update(ms(103_000), true, false, false));
        assert!(!h.update(ms(103_000) + AutoHide::DELAY, false, false, false));
    }

    #[test]
    fn dragging_a_slider_off_the_bar_keeps_the_controls() {
        let t0 = Instant::now();
        let ms = |n| t0 + Duration::from_millis(n);
        let mut h = AutoHide::new(t0);
        assert!(h.update(ms(5000), false, true, true), "pressed on the bar");
        // pointer wanders off the bar but the button is still down, without moving much
        assert!(h.update(ms(9000), false, false, true));
        assert!(h.update(ms(20_000), false, false, true));
        // released: the countdown applies again (last move was long ago)
        assert!(!h.update(ms(20_100), false, false, false));
    }

    #[test]
    fn time_left_counts_down_to_zero() {
        let d = AutoHide::DELAY;
        let t0 = Instant::now();
        let mut h = AutoHide::new(t0);
        h.update(t0, true, false, false);
        assert_eq!(h.time_left(t0), d);
        assert_eq!(h.time_left(t0 + d / 2), d / 2);
        assert_eq!(h.time_left(t0 + Duration::from_secs(10)), Duration::ZERO);
    }

    #[test]
    fn fitted_image_stays_inside_the_screen_when_centred_on_it() {
        // Regression: sizing to the whole screen but centring on something
        // other than the screen centre pushed the picture off the top edge.
        let scr = screen();
        for size in [vec2(1920.0, 1080.0), vec2(1080.0, 1920.0), vec2(50.0, 50.0)] {
            let fit = fit_within(size, scr.size());
            assert!(
                (fit.x / fit.y - size.x / size.y).abs() < 1e-4,
                "aspect kept"
            );
            let r = Rect::from_center_size(scr.center(), fit);
            assert!(r.min.x >= scr.min.x - 1e-3 && r.min.y >= scr.min.y - 1e-3);
            assert!(r.max.x <= scr.max.x + 1e-3 && r.max.y <= scr.max.y + 1e-3);
            // and it fills the screen in at least one direction
            assert!(
                (fit.x - scr.width()).abs() < 1e-3 || (fit.y - scr.height()).abs() < 1e-3,
                "{fit:?}"
            );
        }
    }

    #[test]
    fn clamp_pan_centres_small_and_limits_large() {
        // smaller than the window: always centred
        assert_eq!(
            clamp_pan(vec2(50.0, -20.0), vec2(600.0, 400.0), vec2(1000.0, 800.0)),
            Vec2::ZERO
        );
        // 2000 wide in a 1000 window: may move 500 either way; height still centred
        let p = clamp_pan(vec2(900.0, 300.0), vec2(2000.0, 700.0), vec2(1000.0, 800.0));
        assert_eq!(p, vec2(500.0, 0.0));
    }

    #[test]
    fn zoom_keeps_the_point_under_the_cursor_fixed() {
        let fit = vec2(900.0, 700.0);
        let (mut z, mut pan) = (1.0, Vec2::ZERO);
        let anchor = pos2(650.0, 300.0);
        // image point under the cursor before zooming (relative to image centre)
        let before = (anchor - screen().center() - pan) / z;
        zoom_at(&mut z, &mut pan, 3.0, anchor, screen(), fit, 64.0);
        assert!((z - 3.0).abs() < 1e-5);
        let after = (anchor - screen().center() - pan) / z;
        assert!((before - after).length() < 1e-3, "{before:?} vs {after:?}");
    }

    #[test]
    fn zoom_is_limited_and_resets_pan_at_fit() {
        let fit = vec2(900.0, 700.0);
        let (mut z, mut pan) = (1.0, Vec2::ZERO);
        zoom_at(
            &mut z,
            &mut pan,
            1000.0,
            pos2(10.0, 10.0),
            screen(),
            fit,
            8.0,
        );
        assert_eq!(z, 8.0);
        // pan is within the image bounds
        let lim = (fit * 8.0 - screen().size()) * 0.5;
        assert!(pan.x.abs() <= lim.x + 1e-3 && pan.y.abs() <= lim.y + 1e-3);
        // zoom back out past fit: stays at 1.0 and centred
        zoom_at(
            &mut z,
            &mut pan,
            0.0001,
            pos2(10.0, 10.0),
            screen(),
            fit,
            8.0,
        );
        assert_eq!((z, pan), (1.0, Vec2::ZERO));
    }
}

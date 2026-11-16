//! Ctrl+F filename search.
//!
//! Matching works on the path *relative to the opened folder* (so `holiday/2023/img.jpg`
//! is searched as a whole, sub directories included). The query is split on
//! whitespace and every word has to occur somewhere in the path (case
//! insensitive, any order).
//!
//! This module holds the pure search state (unit tested), the floating dialog
//! and the painting helpers used to highlight hits in the grid. Scrolling to a
//! hit is left to the caller, which owns the layout.

use eframe::egui::{
    self, pos2,
    text::{CCursor, CCursorRange, LayoutJob, TextFormat},
    vec2, Align2, Color32, CursorIcon, FontId, Id, Key, Margin, Rect, Sense, Shape, Stroke,
};
use std::path::Path;

/// Amber highlight used for hits, the dialog border and matched text.
pub const ACCENT: Color32 = Color32::from_rgb(255, 196, 61);
const NO_HITS: Color32 = Color32::from_rgb(224, 98, 98);

// ------------------------------------------------------------------ matching

/// Path of `path` relative to `root`, always with `/` separators.
pub fn rel_path(root: &Path, path: &Path) -> String {
    let rel = path.strip_prefix(root).unwrap_or(path).to_string_lossy();
    if std::path::MAIN_SEPARATOR == '/' {
        rel.into_owned()
    } else {
        rel.replace(std::path::MAIN_SEPARATOR, "/")
    }
}

/// Lower-cased, whitespace separated search words.
pub fn split_words(query: &str) -> Vec<String> {
    query.split_whitespace().map(str::to_lowercase).collect()
}

/// `rel_lc` must already be lower-cased. All words have to be contained.
pub fn matches(rel_lc: &str, words: &[String]) -> bool {
    words.iter().all(|w| rel_lc.contains(w.as_str()))
}

/// Merged byte ranges of `rel` covered by any of the (lower-case) `words`.
/// Returns nothing if lower-casing changed the byte layout of the string.
pub fn highlight_ranges(rel: &str, words: &[String]) -> Vec<(usize, usize)> {
    let lc = rel.to_lowercase();
    if lc.len() != rel.len() {
        return vec![];
    }
    let mut ranges: Vec<(usize, usize)> = vec![];
    for w in words.iter().filter(|w| !w.is_empty()) {
        let mut from = 0;
        while let Some(i) = lc[from..].find(w.as_str()) {
            let (s, e) = (from + i, from + i + w.len());
            ranges.push((s, e));
            from = e;
        }
    }
    ranges.sort_unstable();
    let mut merged: Vec<(usize, usize)> = vec![];
    for (s, e) in ranges {
        match merged.last_mut() {
            Some(last) if s <= last.1 => last.1 = last.1.max(e),
            _ => merged.push((s, e)),
        }
    }
    if merged
        .iter()
        .any(|&(s, e)| !rel.is_char_boundary(s) || !rel.is_char_boundary(e))
    {
        return vec![];
    }
    merged
}

// --------------------------------------------------------------------- state

#[derive(Default)]
pub struct Search {
    pub open: bool,
    pub query: String,
    /// Hits need recomputing (query, items or ordering changed).
    pub dirty: bool,
    /// Grab keyboard focus (and select the text) on the next frame.
    focus: bool,
    words: Vec<String>,
    /// Item ids of all hits, in the order they appear in the grid.
    hits: Vec<u32>,
    is_hit: Vec<bool>,
    current: usize,
}

impl Search {
    pub fn open_dialog(&mut self) {
        self.open = true;
        self.focus = true;
        self.dirty = true;
    }

    /// Hits are only highlighted while the dialog is open and has a query.
    pub fn active(&self) -> bool {
        self.open && !self.words.is_empty()
    }

    pub fn words(&self) -> &[String] {
        &self.words
    }

    pub fn total(&self) -> usize {
        self.hits.len()
    }

    /// 1-based index of the current hit, 0 if there are none.
    pub fn position(&self) -> usize {
        if self.hits.is_empty() {
            0
        } else {
            self.current + 1
        }
    }

    pub fn is_hit(&self, id: u32) -> bool {
        self.is_hit.get(id as usize).copied().unwrap_or(false)
    }

    pub fn current_id(&self) -> Option<u32> {
        self.hits.get(self.current).copied()
    }

    /// Re-run the search. `order` is the grid order of all item ids and
    /// `rel_lc[id]` the lower-cased relative path of item `id`. The current
    /// hit is kept if it still matches, so refining a query doesn't jump away.
    pub fn recompute(&mut self, order: &[u32], rel_lc: &[String], n_items: usize) {
        self.dirty = false;
        let keep = self.current_id();
        self.words = split_words(&self.query);
        self.hits.clear();
        self.is_hit.clear();
        self.is_hit.resize(n_items, false);
        self.current = 0;
        if self.words.is_empty() {
            return;
        }
        for &id in order {
            let hit = rel_lc
                .get(id as usize)
                .is_some_and(|p| matches(p, &self.words));
            if hit {
                if let Some(f) = self.is_hit.get_mut(id as usize) {
                    *f = true;
                    self.hits.push(id);
                }
            }
        }
        if let Some(k) = keep {
            if let Some(pos) = self.hits.iter().position(|&h| h == k) {
                self.current = pos;
            }
        }
    }

    /// Move to the next (`d > 0`) or previous hit, wrapping around.
    pub fn step(&mut self, d: i32) -> Option<u32> {
        if self.hits.is_empty() {
            return None;
        }
        let n = self.hits.len() as i64;
        self.current = (self.current as i64 + d as i64).rem_euclid(n) as usize;
        self.current_id()
    }
}

// -------------------------------------------------------------------- dialog

#[derive(Default)]
pub struct Outcome {
    pub query_changed: bool,
    /// -1 = previous hit, 1 = next hit.
    pub step: i32,
    pub close: bool,
}

enum Icon {
    Up,
    Down,
    Close,
}

fn icon_button(ui: &mut egui::Ui, icon: Icon, enabled: bool, tip: &str) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(
        vec2(24.0, 24.0),
        if enabled {
            Sense::click()
        } else {
            Sense::hover()
        },
    );
    let hovered = enabled && resp.hovered();
    let p = ui.painter();
    if hovered {
        p.rect_filled(rect, 5.0, Color32::from_white_alpha(30));
    }
    let col = if !enabled {
        Color32::from_gray(80)
    } else if hovered {
        Color32::WHITE
    } else {
        Color32::from_gray(200)
    };
    let c = rect.center();
    match icon {
        Icon::Up => {
            p.add(Shape::convex_polygon(
                vec![
                    pos2(c.x - 5.5, c.y + 3.0),
                    pos2(c.x + 5.5, c.y + 3.0),
                    pos2(c.x, c.y - 4.0),
                ],
                col,
                Stroke::NONE,
            ));
        }
        Icon::Down => {
            p.add(Shape::convex_polygon(
                vec![
                    pos2(c.x - 5.5, c.y - 3.0),
                    pos2(c.x + 5.5, c.y - 3.0),
                    pos2(c.x, c.y + 4.0),
                ],
                col,
                Stroke::NONE,
            ));
        }
        Icon::Close => {
            let s = Stroke::new(1.6_f32, col);
            p.line_segment([c + vec2(-4.5, -4.5), c + vec2(4.5, 4.5)], s);
            p.line_segment([c + vec2(-4.5, 4.5), c + vec2(4.5, -4.5)], s);
        }
    }
    if enabled {
        resp.on_hover_text(tip)
            .on_hover_cursor(CursorIcon::PointingHand)
    } else {
        resp
    }
}

/// The floating search box, pinned to the top right of `avail` (the area that
/// is left once the top bar is laid out).
pub fn show(ctx: &egui::Context, avail: Rect, s: &mut Search) -> Outcome {
    let mut out = Outcome::default();
    if !s.open {
        return out;
    }
    let input_id = Id::new("search_input");
    if s.focus {
        ctx.memory_mut(|m| m.request_focus(input_id));
        if let Some(mut st) = egui::TextEdit::load_state(ctx, input_id) {
            let n = s.query.chars().count();
            st.cursor
                .set_char_range(Some(CCursorRange::two(CCursor::new(0), CCursor::new(n))));
            st.store(ctx, input_id);
        }
        s.focus = false;
    }

    let has_query = !s.words.is_empty();
    let border = if s.total() > 0 {
        ACCENT.gamma_multiply(0.85)
    } else if has_query {
        NO_HITS
    } else {
        ctx.style().visuals.window_stroke.color
    };

    egui::Area::new(Id::new("search_dialog"))
        .order(egui::Order::Foreground)
        .fixed_pos(pos2(avail.right() - 14.0, avail.top() + 8.0))
        .pivot(Align2::RIGHT_TOP)
        .show(ctx, |ui| {
            egui::Frame::popup(ui.style())
                .rounding(8.0)
                .inner_margin(Margin::symmetric(10.0, 7.0))
                .stroke(Stroke::new(1.5_f32, border))
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 6.0;
                        ui.label("🔍");

                        let te = ui.add(
                            egui::TextEdit::singleline(&mut s.query)
                                .id(input_id)
                                .hint_text("path words…")
                                .desired_width(230.0),
                        );
                        if te.changed() {
                            out.query_changed = true;
                        }
                        if te.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) {
                            out.step = if ui.input(|i| i.modifiers.shift) {
                                -1
                            } else {
                                1
                            };
                            ctx.memory_mut(|m| m.request_focus(input_id));
                        }

                        let txt = format!("{}/{}", s.position(), s.total());
                        let color = if has_query && s.total() == 0 {
                            NO_HITS
                        } else if s.total() > 0 {
                            ACCENT
                        } else {
                            Color32::from_gray(150)
                        };
                        ui.add_sized(
                            vec2(72.0, 22.0),
                            egui::Label::new(egui::RichText::new(txt).color(color)),
                        );

                        let can_step = s.total() > 0;
                        if icon_button(ui, Icon::Up, can_step, "Previous hit (Shift+Enter)")
                            .clicked()
                        {
                            out.step = -1;
                        }
                        if icon_button(ui, Icon::Down, can_step, "Next hit (Enter)").clicked() {
                            out.step = 1;
                        }
                        ui.add_space(2.0);
                        if icon_button(ui, Icon::Close, true, "Close (Esc)").clicked() {
                            out.close = true;
                        }
                    });
                });
        });
    out
}

// ------------------------------------------------------------------ painting

/// A path with the matched words highlighted (black on amber).
pub fn path_job(
    rel: &str,
    words: &[String],
    font: FontId,
    color: Color32,
    wrap_width: f32,
    max_rows: usize,
) -> LayoutJob {
    let normal = TextFormat {
        font_id: font.clone(),
        color,
        ..Default::default()
    };
    let hit = TextFormat {
        font_id: font,
        color: Color32::BLACK,
        background: ACCENT,
        ..Default::default()
    };
    let mut job = LayoutJob::default();
    let mut at = 0;
    for (s, e) in highlight_ranges(rel, words) {
        if s > at {
            job.append(&rel[at..s], 0.0, normal.clone());
        }
        job.append(&rel[s..e], 0.0, hit.clone());
        at = e;
    }
    if at < rel.len() {
        job.append(&rel[at..], 0.0, normal);
    }
    job.wrap.max_width = wrap_width;
    job.wrap.max_rows = max_rows;
    job.wrap.break_anywhere = true;
    job
}

/// Overlay for every visible tile while a search with hits is active:
/// non-matching tiles fade into the background, hits get an amber frame.
pub fn paint_tile_overlay(painter: &egui::Painter, r: Rect, hit: bool) {
    if hit {
        painter.rect_filled(r, 0.0, ACCENT.gamma_multiply(0.07));
        painter.rect_stroke(
            r.shrink(1.0),
            0.0,
            Stroke::new(2.0_f32, ACCENT.gamma_multiply(0.9)),
        );
    } else {
        painter.rect_filled(r, 0.0, Color32::from_black_alpha(175));
    }
}

/// The selected hit: a glowing double frame plus a caption strip with the
/// relative path. Paint it after all tiles so nothing covers it.
pub fn paint_current(painter: &egui::Painter, r: Rect, rel: &str, words: &[String]) {
    painter.rect_stroke(
        r.expand(6.0),
        6.0,
        Stroke::new(2.0_f32, ACCENT.gamma_multiply(0.22)),
    );
    painter.rect_stroke(
        r.expand(3.0),
        3.0,
        Stroke::new(2.0_f32, ACCENT.gamma_multiply(0.55)),
    );
    painter.rect_stroke(r, 0.0, Stroke::new(3.0_f32, ACCENT));

    let pad = 6.0;
    let galley = painter.layout_job(path_job(
        rel,
        words,
        FontId::proportional(12.0),
        Color32::from_gray(230),
        (r.width() - pad * 2.0).max(10.0),
        3,
    ));
    let h = galley.size().y + pad * 2.0 - 2.0;
    let strip = Rect::from_min_max(pos2(r.min.x, (r.max.y - h).max(r.min.y)), r.max);
    let clipped = painter.with_clip_rect(r);
    clipped.rect_filled(strip, 0.0, Color32::from_black_alpha(215));
    clipped.galley(strip.min + vec2(pad, pad - 1.0), galley, Color32::WHITE);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn lc(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_lowercase()).collect()
    }

    #[test]
    fn words_are_split_and_lowercased() {
        assert_eq!(split_words("  Cats   2023 "), vec!["cats", "2023"]);
        assert!(split_words("   ").is_empty());
    }

    #[test]
    fn every_word_must_match_in_any_order() {
        let p = "pets/cats/2023/img_001.jpg";
        assert!(matches(p, &split_words("cats 2023")));
        assert!(matches(p, &split_words("2023 CATS")));
        assert!(matches(p, &split_words("pets/cats img_0")));
        assert!(!matches(p, &split_words("cats 2024")));
        assert!(!matches(p, &split_words("dogs")));
    }

    #[test]
    fn sub_directories_are_part_of_the_haystack() {
        let root = PathBuf::from("/photos");
        let rel = rel_path(&root, &PathBuf::from("/photos/holiday/day1/a.jpg"));
        assert_eq!(rel, "holiday/day1/a.jpg");
        assert!(matches(&rel.to_lowercase(), &split_words("holiday a.jpg")));
        // the root itself is *not* searched
        assert!(!matches(&rel.to_lowercase(), &split_words("photos")));
    }

    #[test]
    fn highlight_ranges_merge_overlaps() {
        let r = highlight_ranges("Holiday/holiday.jpg", &split_words("holi day"));
        assert_eq!(r, vec![(0, 7), (8, 15)]);
        let r = highlight_ranges("abcdef", &split_words("abc cde"));
        assert_eq!(r, vec![(0, 5)]);
        assert!(highlight_ranges("abc", &split_words("x")).is_empty());
    }

    fn setup() -> (Search, Vec<String>) {
        // grid order is 2,0,1,3 (not id order)
        let rel = lc(&["a/cat.jpg", "b/dog.jpg", "a/cat2.jpg", "c/cat3.png"]);
        let mut s = Search::default();
        s.query = "cat".into();
        s.recompute(&[2, 0, 1, 3], &rel, 4);
        (s, rel)
    }

    #[test]
    fn hits_follow_grid_order_and_step_wraps() {
        let (mut s, _) = setup();
        assert_eq!(s.total(), 3);
        assert_eq!(s.position(), 1);
        assert_eq!(s.current_id(), Some(2));
        assert!(s.is_hit(0) && s.is_hit(2) && s.is_hit(3) && !s.is_hit(1));
        assert_eq!(s.step(1), Some(0));
        assert_eq!(s.step(1), Some(3));
        assert_eq!(s.step(1), Some(2)); // wrapped
        assert_eq!(s.step(-1), Some(3)); // and back
    }

    #[test]
    fn refining_keeps_the_current_hit() {
        let (mut s, rel) = setup();
        s.step(1); // on id 0
        s.query = "cat.jpg".into();
        s.recompute(&[2, 0, 1, 3], &rel, 4);
        assert_eq!(s.total(), 1);
        assert_eq!(s.current_id(), Some(0));
        // current no longer matches -> back to the first hit
        s.query = "png".into();
        s.recompute(&[2, 0, 1, 3], &rel, 4);
        assert_eq!(s.current_id(), Some(3));
    }

    #[test]
    fn empty_query_and_no_hits() {
        let (mut s, rel) = setup();
        s.query = "zzz".into();
        s.recompute(&[2, 0, 1, 3], &rel, 4);
        assert_eq!((s.total(), s.position()), (0, 0));
        assert_eq!(s.step(1), None);
        s.query = "".into();
        s.recompute(&[2, 0, 1, 3], &rel, 4);
        assert_eq!(s.total(), 0);
        assert!(!s.active());
    }
}

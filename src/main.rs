mod layout;
mod scan;
mod thumbs;
mod video;

use chrono::{Datelike, Local, TimeZone};
use eframe::egui::{
    self, pos2, vec2, Align2, Color32, FontId, Id, Key, Pos2, Rect, Sense, Stroke, TextureHandle,
    TextureOptions,
};
use layout::{Group, Layout, HEADER_H};
use scan::{Item, Kind, ScanHandle, ScanMsg};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::sync::mpsc::{channel, Receiver};
use std::time::{Duration, Instant};
use thumbs::{Req, Thumbs};

const TEX_BUDGET_BYTES: usize = 400 * 1024 * 1024;

const APP_NAME: &str = "gridview";
const SLIDER_COLUMNS_DEFAULT: usize = 12;
const SLIDER_COLUMNS_MIN: usize = 1;
const SLIDER_COLUMNS_MAX: usize = 200;
const WINDOW_WIDTH: f32 = 1280.0;
const WINDOW_HEIGHT: f32 = 800.0;

#[derive(Clone, Copy, PartialEq, Eq)]
enum GroupMode {
    Folder,
    Modified,
    Created,
}
impl GroupMode {
    fn label(self) -> &'static str {
        match self {
            GroupMode::Folder => "By folder",
            GroupMode::Modified => "By date modified",
            GroupMode::Created => "By date created",
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum DateGran {
    Day,
    Month,
}

struct Tex {
    handle: TextureHandle,
    tier: u32,
    bytes: usize,
}

struct Viewer {
    id: u32,
    tex: Option<TextureHandle>,
    fallback: Option<TextureHandle>,
    rx: Receiver<Option<egui::ColorImage>>,
}

struct App {
    root: PathBuf,
    path_input: String,
    items: Vec<Item>,
    groups: Vec<Group>,
    order: Vec<u32>,
    order_pos: Vec<u32>,
    layout: Layout,
    scan: Option<ScanHandle>,
    scanning: bool,
    dims_done: usize,

    columns: usize,
    gap: f32,
    group_mode: GroupMode,
    gran: DateGran,
    zoom_acc: f32,

    groups_dirty: bool,
    layout_dirty: bool,
    last_layout: Instant,

    thumbs: Thumbs,
    tex: HashMap<u32, Tex>,
    tex_bytes: usize,
    failed: HashSet<u32>,
    last_wanted: Vec<(u32, u32)>,

    scroll_y: f32,
    scroll_dir: f32,
    view_h: f32,
    bar_reserve: f32,
    scroll_target: Option<(f32, u8)>,

    viewer: Option<Viewer>,
}

fn open_external(path: &Path) {
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

impl App {
    fn new(cc: &eframe::CreationContext<'_>, root: PathBuf) -> Self {
        let ctx = cc.egui_ctx.clone();
        let mut app = App {
            path_input: root.to_string_lossy().into_owned(),
            root: root.clone(),
            items: vec![],
            groups: vec![],
            order: vec![],
            order_pos: vec![],
            layout: Layout::default(),
            scan: None,
            scanning: false,
            dims_done: 0,
            columns: SLIDER_COLUMNS_DEFAULT,
            gap: 4.0,
            group_mode: GroupMode::Folder,
            gran: DateGran::Day,
            zoom_acc: 1.0,
            groups_dirty: false,
            layout_dirty: false,
            last_layout: Instant::now(),
            thumbs: Thumbs::new(ctx.clone()),
            tex: HashMap::new(),
            tex_bytes: 0,
            failed: HashSet::new(),
            last_wanted: vec![],
            scroll_y: 0.0,
            scroll_dir: 0.0,
            view_h: 800.0,
            bar_reserve: 0.0,
            scroll_target: None,
            viewer: None,
        };
        app.open_dir(root, &ctx);
        app
    }

    fn open_dir(&mut self, root: PathBuf, ctx: &egui::Context) {
        self.path_input = root.to_string_lossy().into_owned();
        self.root = root.clone();
        self.items.clear();
        self.groups.clear();
        self.order.clear();
        self.order_pos.clear();
        self.layout = Layout::default();
        self.tex.clear();
        self.tex_bytes = 0;
        self.failed.clear();
        self.last_wanted.clear();
        self.thumbs.set_wanted(vec![]);
        self.viewer = None;
        self.dims_done = 0;
        self.scroll_y = 0.0;
        self.scroll_target = Some((0.0, 1));
        self.scanning = true;
        self.scan = Some(scan::spawn_scan(root, ctx.clone()));
    }

    // ---------------------------------------------------------------- scan
    fn poll_scan(&mut self) {
        let Some(h) = &self.scan else { return };
        let mut msgs = vec![];
        while let Ok(m) = h.rx.try_recv() {
            msgs.push(m);
        }
        for m in msgs {
            match m {
                ScanMsg::Walked(v) => {
                    self.items = v;
                    self.groups_dirty = true;
                }
                ScanMsg::Dims(v) => {
                    self.dims_done += v.len();
                    for (id, w, h) in v {
                        if let Some(it) = self.items.get_mut(id as usize) {
                            it.w = w;
                            it.h = h;
                        }
                    }
                    self.layout_dirty = true;
                }
                ScanMsg::Done => {
                    self.scanning = false;
                    self.layout_dirty = true;
                }
            }
        }
    }

    fn rebuild_groups(&mut self) {
        self.groups_dirty = false;
        self.groups = build_groups(&self.items, self.group_mode, self.gran, &self.root);
        self.order = self
            .groups
            .iter()
            .flat_map(|g| g.ids.iter().copied())
            .collect();
        self.order_pos = vec![0; self.items.len()];
        for (i, id) in self.order.iter().enumerate() {
            self.order_pos[*id as usize] = i as u32;
        }
        self.layout_dirty = true;
    }

    fn relayout(&mut self, width: f32) {
        // Keep the item that was at the top of the screen at the top of the screen.
        let anchor = self.layout.anchor_at(self.scroll_y);
        self.layout = Layout::compute(
            &self.items,
            &self.groups,
            width,
            self.columns,
            self.gap,
            self.group_mode != GroupMode::Folder || self.groups.len() > 1,
        );
        self.layout_dirty = false;
        self.last_layout = Instant::now();
        if let Some((id, frac)) = anchor {
            let y = self.layout.y_of[id as usize] + frac * self.layout.h_of[id as usize];
            self.scroll_target = Some((y, 2));
        }
    }

    // ------------------------------------------------------------- thumbs
    fn poll_thumbs(&mut self, ctx: &egui::Context) {
        let start = Instant::now();
        while let Ok(d) = self.thumbs.rx.try_recv() {
            match d.img {
                Some(img) => {
                    let (w, h) = img.dimensions();
                    if let Some(it) = self.items.get_mut(d.id as usize) {
                        if it.w == 0 || it.h == 0 {
                            it.w = w;
                            it.h = h;
                            self.layout_dirty = true;
                        }
                    }
                    let ci = egui::ColorImage::from_rgb([w as usize, h as usize], img.as_raw());
                    let handle = ctx.load_texture(format!("t{}", d.id), ci, TextureOptions::LINEAR);
                    let bytes = (w * h * 4) as usize;
                    if let Some(old) = self.tex.insert(
                        d.id,
                        Tex {
                            handle,
                            tier: d.tier,
                            bytes,
                        },
                    ) {
                        self.tex_bytes -= old.bytes;
                    }
                    self.tex_bytes += bytes;
                }
                None => {
                    self.failed.insert(d.id);
                }
            }
            if start.elapsed() > Duration::from_millis(6) {
                ctx.request_repaint();
                break;
            }
        }
        self.evict();
    }

    fn evict(&mut self) {
        if self.tex_bytes <= TEX_BUDGET_BYTES {
            return;
        }
        let center = self.scroll_y + self.view_h * 0.5;
        let mut v: Vec<(f32, u32)> = self
            .tex
            .keys()
            .map(|&id| {
                let y = self.layout.y_of.get(id as usize).copied().unwrap_or(0.0);
                ((y - center).abs(), id)
            })
            .collect();
        v.sort_by(|a, b| b.0.total_cmp(&a.0));
        for (dist, id) in v {
            if self.tex_bytes <= TEX_BUDGET_BYTES * 8 / 10 || dist < self.view_h * 2.0 {
                break;
            }
            if let Some(t) = self.tex.remove(&id) {
                self.tex_bytes -= t.bytes;
            }
        }
    }

    // ---------------------------------------------------------------- viewer
    fn open_viewer(&mut self, ctx: &egui::Context, id: u32) {
        let it = &self.items[id as usize];
        let (path, kind) = (it.path.clone(), it.kind);
        let screen = ctx.screen_rect().size() * ctx.pixels_per_point();
        let edge = (screen.max_elem() as u32).clamp(1024, 4096);
        let (tx, rx) = channel();
        let c = ctx.clone();
        std::thread::spawn(move || {
            let ci = thumbs::make_image(&path, kind, edge).map(|img| {
                egui::ColorImage::from_rgb(
                    [img.width() as usize, img.height() as usize],
                    img.as_raw(),
                )
            });
            let _ = tx.send(ci);
            c.request_repaint();
        });
        self.viewer = Some(Viewer {
            id,
            tex: None,
            fallback: self.tex.get(&id).map(|t| t.handle.clone()),
            rx,
        });
    }

    fn viewer_ui(&mut self, ctx: &egui::Context) {
        let Some(v) = &mut self.viewer else { return };
        if let Ok(Some(ci)) = v.rx.try_recv() {
            v.tex = Some(ctx.load_texture("viewer", ci, TextureOptions::LINEAR));
        }
        let id = v.id;
        let is_video = self.items[id as usize].kind == Kind::Video;
        let tex = v.tex.clone().or_else(|| v.fallback.clone());

        enum Act {
            None,
            Close,
            Step(i32),
            External,
        }
        let mut act = Act::None;

        egui::Area::new(Id::new("viewer"))
            .order(egui::Order::Foreground)
            .fixed_pos(Pos2::ZERO)
            .show(ctx, |ui| {
                let screen = ctx.screen_rect();
                let resp = ui.allocate_rect(screen, Sense::click());
                let p = ui.painter();
                p.rect_filled(screen, 0.0, Color32::from_black_alpha(242));
                if let Some(t) = &tex {
                    let ts = t.size_vec2();
                    let avail = screen.shrink(24.0).size();
                    let s = (avail.x / ts.x).min(avail.y / ts.y);
                    let r = Rect::from_center_size(screen.center(), ts * s);
                    p.image(
                        t.id(),
                        r,
                        Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
                        Color32::WHITE,
                    );
                } else {
                    p.text(
                        screen.center(),
                        Align2::CENTER_CENTER,
                        "Loading…",
                        FontId::proportional(18.0),
                        Color32::GRAY,
                    );
                }
                let name = self.items[id as usize].path.to_string_lossy().into_owned();
                p.text(
                    pos2(screen.min.x + 16.0, screen.max.y - 14.0),
                    Align2::LEFT_BOTTOM,
                    name,
                    FontId::proportional(13.0),
                    Color32::from_gray(180),
                );
                if is_video {
                    let c = screen.center();
                    p.circle_filled(c, 38.0, Color32::from_black_alpha(170));
                    draw_play(p, c, 18.0);
                }
                if resp.clicked() {
                    act = if is_video { Act::External } else { Act::Close };
                }
            });

        ctx.input(|i| {
            if i.key_pressed(Key::Escape) {
                act = Act::Close;
            } else if i.key_pressed(Key::ArrowRight) {
                act = Act::Step(1);
            } else if i.key_pressed(Key::ArrowLeft) {
                act = Act::Step(-1);
            } else if i.key_pressed(Key::Enter) || i.key_pressed(Key::Space) {
                act = Act::External;
            }
        });

        match act {
            Act::None => {}
            Act::Close => {
                let y = self.layout.y_of.get(id as usize).copied().unwrap_or(0.0);
                if y < self.scroll_y || y > self.scroll_y + self.view_h * 0.8 {
                    self.scroll_target = Some(((y - self.view_h * 0.3).max(0.0), 1));
                }
                self.viewer = None;
            }
            Act::Step(d) => {
                let pos = self.order_pos[id as usize] as i32 + d;
                if pos >= 0 && (pos as usize) < self.order.len() {
                    let nid = self.order[pos as usize];
                    self.open_viewer(ctx, nid);
                }
            }
            Act::External => open_external(&self.items[id as usize].path),
        }
    }

    // ------------------------------------------------------------------ grid
    fn grid_ui(&mut self, ui: &mut egui::Ui) {
        if self.groups_dirty {
            self.rebuild_groups();
        }
        let outer = ui.available_width();
        let width = (outer - self.bar_reserve).max(100.0);
        let throttled = self.scanning && self.last_layout.elapsed() < Duration::from_millis(200);
        let structural = self.layout.columns != self.columns
            || self.layout.gap != self.gap
            || (self.layout.width - width).abs() > 0.5;
        if structural || (self.layout_dirty && !throttled) {
            self.relayout(width);
        } else if self.layout_dirty {
            ui.ctx().request_repaint_after(Duration::from_millis(200));
        }

        let mut sa = egui::ScrollArea::vertical().auto_shrink([false, false]);
        if let Some((y, left)) = self.scroll_target.take() {
            sa = sa.vertical_scroll_offset(y.max(0.0));
            if left > 1 {
                self.scroll_target = Some((y, left - 1));
                ui.ctx().request_repaint();
            }
        }
        let ppp = ui.ctx().pixels_per_point();
        let out = sa.show_viewport(ui, |ui, viewport| self.paint_grid(ui, viewport, outer, ppp));
        let new_y = out.state.offset.y;
        let d = new_y - self.scroll_y;
        if d.abs() > 0.5 {
            self.scroll_dir = d.signum();
        }
        self.scroll_y = new_y;
    }

    fn paint_grid(&mut self, ui: &mut egui::Ui, viewport: Rect, outer: f32, ppp: f32) {
        let inner_w = ui.available_width();
        self.bar_reserve = (outer - inner_w).max(0.0);
        let (area, _) =
            ui.allocate_exact_size(vec2(inner_w, self.layout.total_h.max(1.0)), Sense::hover());
        let origin = area.min;
        let (top, bot) = (viewport.min.y, viewport.max.y);
        let vh = viewport.height();
        self.view_h = vh;
        let painter = ui.painter().clone();

        // one pass over visible + prefetch range
        let mut vis: Vec<(u32, Rect)> = vec![];
        let mut wanted: Vec<(f32, f32, Req)> = vec![];
        let (pre_top, pre_bot) = (top - vh, bot + vh * 3.0);
        let lay = &self.layout;
        lay.visit(pre_top, pre_bot, |c, p| {
            let r = Rect::from_min_size(origin + vec2(lay.col_x(c), p.y), vec2(lay.col_w, p.h));
            let overlaps = p.y + p.h >= top && p.y <= bot;
            if overlaps {
                vis.push((p.id, r));
            }
            let tier = thumbs::tier_for(lay.col_w.max(p.h) * ppp);
            let have = self.tex.get(&p.id).map_or(false, |t| t.tier >= tier);
            if !have && !self.failed.contains(&p.id) {
                let mut dist = if p.y + p.h < top {
                    top - (p.y + p.h)
                } else if p.y > bot {
                    p.y - bot
                } else {
                    0.0
                };
                let behind = (p.y + p.h < top && self.scroll_dir >= 0.0)
                    || (p.y > bot && self.scroll_dir < 0.0);
                if behind {
                    dist *= 2.0;
                }
                let it = &self.items[p.id as usize];
                wanted.push((
                    dist,
                    p.y,
                    Req {
                        id: p.id,
                        path: it.path.clone(),
                        kind: it.kind,
                        tier,
                        mtime: it.modified,
                        size: it.size,
                    },
                ));
            }
        });
        wanted.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.total_cmp(&b.1)));
        wanted.truncate(400);
        let key: Vec<(u32, u32)> = wanted.iter().map(|w| (w.2.id, w.2.tier)).collect();
        if key != self.last_wanted {
            self.last_wanted = key;
            self.thumbs
                .set_wanted(wanted.into_iter().map(|w| w.2).collect());
        }

        // headers
        for h in self.layout.visible_headers(top, bot) {
            let y = origin.y + h.y;
            let mid = pos2(origin.x + 12.0, y + HEADER_H * 0.5 + 2.0);
            let g = painter.text(
                mid,
                Align2::LEFT_CENTER,
                &h.label,
                FontId::proportional(19.0),
                Color32::from_gray(235),
            );
            painter.text(
                pos2(g.max.x + 10.0, mid.y + 1.0),
                Align2::LEFT_CENTER,
                format!("{}", h.count),
                FontId::proportional(13.0),
                Color32::from_gray(120),
            );
        }

        // tiles
        let ctx = ui.ctx().clone();
        let mut clicked = None;
        let pointer = ctx.pointer_hover_pos();
        for (id, r) in vis {
            let item = &self.items[id as usize];
            match self.tex.get(&id) {
                Some(t) => {
                    let ts = t.handle.size_vec2();
                    let (ta, ra) = (ts.x / ts.y, r.width() / r.height());
                    let uv = if (ta - ra).abs() < 0.01 {
                        Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0))
                    } else if ta > ra {
                        let w = ra / ta;
                        Rect::from_min_max(pos2((1.0 - w) / 2.0, 0.0), pos2((1.0 + w) / 2.0, 1.0))
                    } else {
                        let h = ta / ra;
                        Rect::from_min_max(pos2(0.0, (1.0 - h) / 2.0), pos2(1.0, (1.0 + h) / 2.0))
                    };
                    let a = ctx.animate_bool_with_time(Id::new(("fade", id)), true, 0.18);
                    painter.image(
                        t.handle.id(),
                        r,
                        uv,
                        Color32::from_white_alpha((a * 255.0) as u8),
                    );
                }
                None => {
                    let bg = if self.failed.contains(&id) {
                        Color32::from_rgb(48, 24, 24)
                    } else {
                        Color32::from_gray(30)
                    };
                    painter.rect_filled(r, 0.0, bg);
                }
            }
            if item.kind == Kind::Video {
                let c = r.center();
                painter.circle_filled(c, 20.0, Color32::from_black_alpha(140));
                draw_play(&painter, c, 10.0);
            }
            let resp = ui.interact(r, Id::new(("tile", id)), Sense::click());
            if resp.clicked() {
                clicked = Some(id);
            }
            if pointer.map_or(false, |p| r.contains(p)) {
                painter.rect_stroke(
                    r,
                    0.0,
                    Stroke::new(2.0_f32, Color32::from_rgb(90, 160, 255)),
                );
            }
        }
        if let Some(id) = clicked {
            self.open_viewer(&ctx, id);
        }
    }
}

fn draw_play(p: &egui::Painter, c: Pos2, s: f32) {
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

fn build_groups(items: &[Item], mode: GroupMode, gran: DateGran, root: &Path) -> Vec<Group> {
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

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _: &mut eframe::Frame) {
        self.poll_scan();
        self.poll_thumbs(ctx);

        // drag & drop a folder onto the window
        if let Some(p) = ctx.input(|i| i.raw.dropped_files.first().and_then(|f| f.path.clone())) {
            let dir = if p.is_dir() {
                p
            } else {
                p.parent().map(|x| x.to_path_buf()).unwrap_or(p)
            };
            self.open_dir(dir, ctx);
        }
        // pinch / ctrl+scroll changes column count
        let z = ctx.input(|i| i.zoom_delta());
        if z != 1.0 && self.viewer.is_none() {
            self.zoom_acc *= z;
            if self.zoom_acc > 1.2 {
                self.columns = (self.columns.max(2) - 1).max(1);
                self.zoom_acc = 1.0;
            } else if self.zoom_acc < 0.83 {
                self.columns = (self.columns + 1).min(SLIDER_COLUMNS_MAX);
                self.zoom_acc = 1.0;
            }
        }

        egui::TopBottomPanel::top("top").show(ctx, |ui| {
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                let te =
                    ui.add(egui::TextEdit::singleline(&mut self.path_input).desired_width(340.0));
                let go = (te.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)))
                    || ui.button("Open").clicked();
                if go {
                    let p = PathBuf::from(self.path_input.trim());
                    if p.is_dir() {
                        self.open_dir(p, ctx);
                    }
                }
                ui.separator();
                ui.label("Columns");
                ui.spacing_mut().slider_width = 500.0;
                ui.add(egui::Slider::new(
                    &mut self.columns,
                    SLIDER_COLUMNS_MIN..=SLIDER_COLUMNS_MAX,
                ));
                ui.separator();
                let mut changed = false;
                egui::ComboBox::from_id_source("group")
                    .selected_text(self.group_mode.label())
                    .show_ui(ui, |ui| {
                        for m in [GroupMode::Folder, GroupMode::Modified, GroupMode::Created] {
                            changed |= ui
                                .selectable_value(&mut self.group_mode, m, m.label())
                                .changed();
                        }
                    });
                if self.group_mode != GroupMode::Folder {
                    egui::ComboBox::from_id_source("gran")
                        .selected_text(if self.gran == DateGran::Day {
                            "Day"
                        } else {
                            "Month"
                        })
                        .show_ui(ui, |ui| {
                            changed |= ui
                                .selectable_value(&mut self.gran, DateGran::Day, "Day")
                                .changed();
                            changed |= ui
                                .selectable_value(&mut self.gran, DateGran::Month, "Month")
                                .changed();
                        });
                }
                if changed {
                    self.groups_dirty = true;
                }
            });
            ui.add_space(4.0);
        });

        egui::TopBottomPanel::bottom("status").show(ctx, |ui| {
            let found = self
                .scan
                .as_ref()
                .map_or(0, |s| s.found.load(Ordering::Relaxed));
            let txt = if self.items.is_empty() && self.scanning {
                format!("Scanning… {found} files found")
            } else if self.scanning {
                format!(
                    "{} items — reading sizes {}/{}",
                    self.items.len(),
                    self.dims_done,
                    self.items.len()
                )
            } else {
                format!(
                    "{} items  •  {} textures in memory ({} MB)",
                    self.items.len(),
                    self.tex.len(),
                    self.tex_bytes >> 20
                )
            };
            ui.label(txt);
        });

        egui::CentralPanel::default()
            .frame(egui::Frame::none().fill(Color32::from_gray(16)))
            .show(ctx, |ui| self.grid_ui(ui));

        self.viewer_ui(ctx);
    }
}

fn main() -> eframe::Result<()> {
    let root = std::env::args()
        .nth(1)
        .map(PathBuf::from)
        .or_else(|| {
            directories::UserDirs::new().and_then(|d| d.picture_dir().map(|p| p.to_path_buf()))
        })
        .unwrap_or_else(|| std::env::current_dir().unwrap());
    let opts = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([WINDOW_WIDTH, WINDOW_HEIGHT])
            .with_title(APP_NAME),
        ..Default::default()
    };
    eframe::run_native(APP_NAME, opts, Box::new(|cc| Box::new(App::new(cc, root))))
}

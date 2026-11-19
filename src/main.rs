mod globals;
mod layout;

#[cfg(feature = "video-player")]
mod player;

mod cli;
mod config;

use crate::globals::{
    DateGran, GroupMode, Tex, APP_NAME, SLIDER_COLUMNS_MAX, SLIDER_COLUMNS_MIN, TEX_BUDGET_BYTES,
};

use clap::Parser;
use cli::{Args, SortMode};
use config::AppConfig;

mod scan;
mod search;
mod thumbs;
mod utils;

#[cfg(feature = "video-thumbnails")]
mod video;

use eframe::egui::{
    self, pos2, vec2, Align2, Color32, CursorIcon, FontId, Id, Key, Modifiers, PointerButton, Pos2,
    Rect, RichText, Sense, Stroke, TextureHandle, TextureOptions, Vec2,
};
use layout::{Group, Layout, HEADER_H};
use rayon::prelude::*;
use scan::{Item, Kind, ScanHandle, ScanMsg};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::sync::mpsc::{channel, Receiver, Sender};
use std::time::{Duration, Instant};
use thumbs::{Req, Thumbs};
use utils::{
    build_groups, clamp_pan, copy_full_path, draw_play, fit_within, gap_for, group_shade,
    open_external, paint_flash, reveal_in_file_manager, setup_fonts, zoom_at, FLASH_SECS,
};

struct Viewer {
    id: u32,
    tex: Option<TextureHandle>,
    fallback: Option<TextureHandle>,
    rx: Receiver<Option<egui::ColorImage>>,
    /// Kept so a sharper decode can be requested once the user zooms in.
    tx: Sender<Option<egui::ColorImage>>,
    /// Longest side (px) of the image currently decoded for the viewer.
    edge: u32,
    /// 1.0 = fit to window. Above that the image is zoomed and can be panned.
    zoom: f32,
    /// Offset of the image centre from the screen centre, in points.
    pan: Vec2,

    #[cfg(feature = "video-player")]
    player: Option<player::Player>,

    #[cfg(feature = "video-player")]
    video_tex: Option<TextureHandle>,

    /// Show/hide logic of the video transport bar.
    #[cfg(feature = "video-player")]
    autohide: utils::AutoHide,
    /// Where the transport bar was drawn last frame (None while hidden).
    #[cfg(feature = "video-player")]
    transport_rect: Option<Rect>,
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
    compact: bool,
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

    search: search::Search,
    /// Lower-cased relative path of every item (index = item id). Built lazily
    /// the first time a search needs it, cleared when the item list changes.
    rel_lc: Vec<String>,
    /// Scroll to the current hit once the hits have been recomputed.
    search_reveal: bool,

    /// Single file mode: the file shown on its own, the folder is not scanned yet.
    single_file: Option<PathBuf>,
    /// File to scroll to (and flash) once the folder scan has produced a layout.
    pending_reveal: Option<PathBuf>,
    /// Tile that is briefly highlighted, and when that started.
    flash: Option<(u32, Instant)>,
    /// Mouse wheel delta taken away from the grid while the viewer is open.
    viewer_scroll: f32,
}

impl App {
    fn new(cc: &eframe::CreationContext<'_>, config: AppConfig) -> Self {
        setup_fonts(&cc.egui_ctx);

        let path_input = config.root.to_string_lossy().into_owned();

        let ctx = cc.egui_ctx.clone();
        let mut app = App {
            path_input,
            root: config.root.clone(),
            items: vec![],
            groups: vec![],
            order: vec![],
            order_pos: vec![],
            layout: Layout::default(),
            scan: None,
            scanning: false,
            dims_done: 0,
            columns: config.columns.clamp(SLIDER_COLUMNS_MIN, SLIDER_COLUMNS_MAX),
            gap: gap_for(config.compact),
            group_mode: config.group_mode,
            gran: config.gran,
            compact: config.compact,
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
            search: search::Search::default(),
            rel_lc: vec![],
            search_reveal: false,
            single_file: None,
            pending_reveal: None,
            flash: None,
            viewer_scroll: 0.0,
        };
        match config.single_file {
            Some(file) => app.open_single(file, &ctx),
            None => app.open_dir(config.root, &ctx),
        }
        app
    }

    /// Forget everything about the previous folder / file.
    fn reset_state(&mut self, root: PathBuf) {
        self.pending_reveal = None;
        self.flash = None;
        self.single_file = None;
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
        self.rel_lc.clear();
        self.search.dirty = true;
        self.dims_done = 0;
        self.scroll_y = 0.0;
        self.scroll_target = Some((0.0, 1));
        self.scanning = false;
        self.scan = None;
    }

    fn open_dir(&mut self, root: PathBuf, ctx: &egui::Context) {
        let title = format!("gridview - {}", root.to_string_lossy());
        ctx.send_viewport_cmd(egui::ViewportCommand::Title(title));

        self.reset_state(root.clone());
        self.scanning = true;
        self.scan = Some(scan::spawn_scan(root, ctx.clone()));
    }

    /// Single file mode: show just this file in the viewer. The folder is only
    /// scanned once the viewer is closed (see `Act::Close`).
    fn open_single(&mut self, file: PathBuf, ctx: &egui::Context) {
        let dir = file
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| self.root.clone());
        let Some(item) = scan::item_from_path(&file) else {
            return self.open_dir(dir, ctx);
        };
        self.reset_state(dir);
        self.items = vec![item];
        self.single_file = Some(file);
        self.rebuild_groups();
        self.open_viewer(ctx, 0);
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
                    self.rel_lc.clear();
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
        self.search.dirty = true;
    }

    // ---------------------------------------------------------------- search
    /// Re-run the search against the current grid order. Called from `grid_ui`
    /// right after the groups were rebuilt, so `order` is always up to date.
    fn refresh_search(&mut self) {
        if self.search.query.split_whitespace().next().is_some()
            && self.rel_lc.len() != self.items.len()
        {
            let root = &self.root;
            let cache: Vec<String> = self
                .items
                .par_iter()
                .map(|it| search::rel_path(root, &it.path).to_lowercase())
                .collect();
            self.rel_lc = cache;
        }
        self.search
            .recompute(&self.order, &self.rel_lc, self.items.len());
    }

    /// Scroll so that item `id` is on screen. With `only_if_hidden` an item
    /// that is already comfortably visible doesn't move the view.
    fn reveal_item(&mut self, id: u32, only_if_hidden: bool, ctx: &egui::Context) {
        let (Some(&y), Some(&h)) = (
            self.layout.y_of.get(id as usize),
            self.layout.h_of.get(id as usize),
        ) else {
            return;
        };
        let margin = self.view_h * 0.08;
        let visible = y >= self.scroll_y + margin && y + h <= self.scroll_y + self.view_h - margin;
        if only_if_hidden && visible {
            return;
        }
        self.scroll_target = Some(((y + h * 0.5 - self.view_h * 0.5).max(0.0), 1));
        ctx.request_repaint();
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
            self.compact,
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
        let tx_keep = tx.clone();
        let image_path = path.clone();
        let c = ctx.clone();

        #[cfg(feature = "video-player")]
        let player = if kind == Kind::Video {
            match player::Player::open(&path, ctx.clone()) {
                Ok(p) => Some(p),
                Err(e) => {
                    eprintln!("Video player error: {e}");
                    None
                }
            }
        } else {
            None
        };

        std::thread::spawn(move || {
            let ci = thumbs::make_image(&image_path, kind, edge).map(|img| {
                egui::ColorImage::from_rgb(
                    [img.width() as usize, img.height() as usize],
                    img.as_raw(),
                )
            });
            let _ = tx.send(ci);
            c.request_repaint();
        });

        let title = format!("gridview - {}", path.to_string_lossy());
        ctx.send_viewport_cmd(egui::ViewportCommand::Title(title));

        self.viewer = Some(Viewer {
            id,
            tex: None,
            fallback: self.tex.get(&id).map(|t| t.handle.clone()),
            rx,
            tx: tx_keep,
            edge,
            zoom: 1.0,
            pan: Vec2::ZERO,
            #[cfg(feature = "video-player")]
            player,

            #[cfg(feature = "video-player")]
            video_tex: None,
            #[cfg(feature = "video-player")]
            autohide: utils::AutoHide::new(Instant::now()),
            #[cfg(feature = "video-player")]
            transport_rect: None,
        });
    }

    fn viewer_ui(&mut self, ctx: &egui::Context) {
        let Some(v) = &mut self.viewer else { return };

        if let Ok(Some(ci)) = v.rx.try_recv() {
            v.tex = Some(ctx.load_texture("viewer", ci, TextureOptions::LINEAR));
        }

        // Update video frame texture.
        #[cfg(feature = "video-player")]
        if let Some(player) = v.player.as_mut() {
            if let Some(frame) = player.take_frame() {
                let image = egui::ColorImage::from_rgb([frame.width, frame.height], &frame.pixels);

                if let Some(texture) = &mut v.video_tex {
                    texture.set(image, TextureOptions::LINEAR);
                } else {
                    v.video_tex =
                        Some(ctx.load_texture("video_frame", image, TextureOptions::LINEAR));
                }
            }
        }

        let id = v.id;
        let is_video = self.items[id as usize].kind == Kind::Video;

        #[cfg(feature = "video-player")]
        let tex = v
            .video_tex
            .clone()
            .or_else(|| v.tex.clone())
            .or_else(|| v.fallback.clone());

        #[cfg(not(feature = "video-player"))]
        let tex = v.tex.clone().or_else(|| v.fallback.clone());

        // zoom: wheel / pinch / + - 0 (the grid behind is not scrolled, see `update`)
        let screen = ctx.screen_rect();
        // Image and video both fill the window and are centred on it; the video
        // transport bar floats over the bottom edge.
        let fit = tex
            .as_ref()
            .map(|t| fit_within(t.size_vec2(), screen.size()));
        if let Some(fit) = fit {
            let it = &self.items[id as usize];
            let native_w = if it.w > 0 { it.w as f32 } else { fit.x };
            let max_zoom = (native_w / fit.x * 8.0).clamp(4.0, 64.0);
            let (pinch, pointer, plus, minus, reset) = ctx.input(|i| {
                (
                    i.zoom_delta(),
                    i.pointer.hover_pos(),
                    i.key_pressed(Key::Plus) || i.key_pressed(Key::Equals),
                    i.key_pressed(Key::Minus),
                    i.key_pressed(Key::Num0),
                )
            });
            let mut k = (self.viewer_scroll * 0.0025).exp() * pinch;
            if plus {
                k *= 1.25;
            }
            if minus {
                k /= 1.25;
            }
            if reset {
                v.zoom = 1.0;
                v.pan = Vec2::ZERO;
            }
            if (k - 1.0).abs() > 1e-4 {
                zoom_at(
                    &mut v.zoom,
                    &mut v.pan,
                    k,
                    pointer.unwrap_or(screen.center()),
                    screen,
                    fit,
                    max_zoom,
                );
            }
        }

        enum Act {
            None,
            Close,
            Step(i32),
            External,
            Reveal,
        }
        let mut act = Act::None;

        #[cfg(feature = "video-player")]
        if let Some(player) = v.player.as_mut() {
            ctx.request_repaint_after(Duration::from_millis(33));

            let screen = ctx.screen_rect();

            // Controls only show while the mouse moves (or rests on them).
            let (moved, pointer, down) = ctx.input(|i| {
                (
                    i.pointer.delta() != Vec2::ZERO,
                    i.pointer.hover_pos(),
                    i.pointer.any_down(),
                )
            });
            let over = matches!((pointer, v.transport_rect), (Some(p), Some(r)) if r.contains(p));
            let now = Instant::now();
            let show = v.autohide.update(now, moved, over, down);
            if show {
                ctx.request_repaint_after(v.autohide.time_left(now));
            }
            let fade = ctx.animate_bool_with_time(Id::new("viewer_transport_fade"), show, 0.18);

            if !show && fade <= 0.0 {
                ctx.set_cursor_icon(egui::CursorIcon::None);
            }

            let area = egui::Area::new(Id::new("viewer_transport"))
                .order(egui::Order::Foreground)
                .fixed_pos(pos2(screen.center().x, screen.bottom() - 10.0))
                .pivot(Align2::CENTER_BOTTOM)
                .show(ctx, |ui| {
                    if fade <= 0.0 {
                        return; // fully hidden: no widgets, so nothing to hover or click
                    }
                    ui.set_opacity(fade);
                    egui::Frame::popup(ui.style()).show(ui, |ui| {
                        ui.horizontal(|ui| {
                            let button_size = egui::vec2(28.0, 28.0);
                            if ui
                                .add_sized(
                                    button_size,
                                    egui::Button::new(if player.is_playing() {
                                        "⏸"
                                    } else {
                                        "▶"
                                    }),
                                )
                                .clicked()
                            {
                                player.toggle_play();
                            }

                            let duration = player.duration();
                            let mut position = player.position().min(duration);

                            if duration > 0.0 {
                                let slider_width =
                                    (ui.available_width() - 500.0).clamp(50.0, f32::MAX);

                                let response = ui
                                    .allocate_ui_with_layout(
                                        egui::vec2(slider_width, 18.0),
                                        egui::Layout::left_to_right(egui::Align::Center),
                                        |ui| {
                                            ui.set_min_width(slider_width);
                                            ui.set_max_width(slider_width);
                                            ui.style_mut().spacing.slider_width = slider_width;

                                            ui.add(
                                                egui::Slider::new(&mut position, 0.0..=duration)
                                                    .show_value(false),
                                            )
                                        },
                                    )
                                    .inner;

                                if response.changed() {
                                    player.seek(position);
                                }

                                ui.label(format!(
                                    "{:02}:{:02} / {:02}:{:02}",
                                    position as u64 / 60,
                                    position as u64 % 60,
                                    duration as u64 / 60,
                                    duration as u64 % 60,
                                ));
                            } else {
                                ui.label("Loading...");
                            }

                            let mut volume = player.volume();

                            ui.label("Volume");
                            if ui
                                .add(egui::Slider::new(&mut volume, 0.0..=1.0).show_value(false))
                                .changed()
                            {
                                player.set_volume(volume);
                            }
                        });
                    });
                });
            v.transport_rect = (fade > 0.0).then_some(area.response.rect);
        }

        egui::Area::new(Id::new("viewer"))
            .order(egui::Order::Middle)
            .fixed_pos(Pos2::ZERO)
            .show(ctx, |ui| {
                let resp = ui.allocate_rect(screen, Sense::click_and_drag());
                let zoomed = v.zoom > 1.0001;
                if let Some(fit) = fit {
                    if zoomed && resp.dragged_by(PointerButton::Primary) {
                        v.pan += resp.drag_delta();
                    }
                    v.pan = clamp_pan(v.pan, fit * v.zoom, screen.size());
                }
                // normal cursor, a hand once there is something to move around
                if zoomed {
                    if resp.dragged() {
                        ctx.set_cursor_icon(CursorIcon::Grabbing);
                    } else if resp.hovered() {
                        ctx.set_cursor_icon(CursorIcon::Grab);
                    }
                    if resp.double_clicked() {
                        v.zoom = 1.0;
                        v.pan = Vec2::ZERO;
                    }
                }
                let p = ui.painter();
                p.rect_filled(screen, 0.0, Color32::from_black_alpha(242));
                if let Some(t) = &tex {
                    let size = fit.unwrap_or(t.size_vec2()) * v.zoom;
                    let r = Rect::from_center_size(screen.center() + v.pan, size);
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

                #[cfg(feature = "video-player")]
                let show_play = is_video
                    && v.player
                        .as_ref()
                        .map_or(true, |player| !player.is_playing());

                #[cfg(not(feature = "video-player"))]
                let show_play = is_video;

                if show_play {
                    let c = screen.center();
                    p.circle_filled(c, 38.0, Color32::from_black_alpha(170));
                    draw_play(p, c, 18.0);
                }

                let menu_open = resp.context_menu_opened();
                resp.context_menu(|ui| {
                    if ui.button("🚀 Open externally").clicked() {
                        act = Act::External;
                        ui.close_menu();
                    }
                    if ui.button("📂 Show in folder").clicked() {
                        act = Act::Reveal;
                        ui.close_menu();
                    }
                    if ui.button("📋 Copy path").clicked() {
                        copy_full_path(ui.ctx(), &self.items[id as usize].path);
                        ui.close_menu();
                    }
                });
                if resp.middle_clicked() {
                    reveal_in_file_manager(&self.items[id as usize].path);
                } else if resp.clicked() && !menu_open && !zoomed {
                    // (zoomed: clicks are ignored so a stray click doesn't throw the zoom away)
                    act = Act::Close;
                }
            });

        // Zoomed in on an image: swap the preview for a sharper decode (once).
        if v.zoom > 1.1 && !is_video {
            let it = &self.items[id as usize];
            let max_side = ctx.input(|i| i.max_texture_side) as u32;
            let want = max_side.min(8192);
            let src = if it.w > 0 { it.w.max(it.h) } else { u32::MAX };
            if want > v.edge && src > v.edge {
                v.edge = want;
                let (path, kind, tx, c) = (it.path.clone(), it.kind, v.tx.clone(), ctx.clone());
                std::thread::spawn(move || {
                    let ci = thumbs::make_image(&path, kind, want).map(|img| {
                        egui::ColorImage::from_rgb(
                            [img.width() as usize, img.height() as usize],
                            img.as_raw(),
                        )
                    });
                    let _ = tx.send(ci);
                    c.request_repaint();
                });
            }
        }

        ctx.input(|i| {
            if i.key_pressed(Key::Escape) {
                act = Act::Close;
            } else if i.key_pressed(Key::ArrowRight) {
                act = Act::Step(1);
            } else if i.key_pressed(Key::ArrowLeft) {
                act = Act::Step(-1);
            } else if i.key_pressed(Key::Enter) {
                act = Act::External;
            } else if i.key_pressed(Key::Space) {
                #[cfg(feature = "video-player")]
                {
                    if let Some(player) = v.player.as_mut() {
                        player.toggle_play();
                    } else {
                        act = Act::External;
                    }
                }

                #[cfg(not(feature = "video-player"))]
                {
                    act = Act::External;
                }
            }
        });

        match act {
            Act::None => {}
            Act::Close if self.single_file.is_some() => {
                // Single file mode: now load the folder, scroll to the file, flash it.
                if let Some(file) = self.single_file.take() {
                    let dir = file.parent().map(Path::to_path_buf).unwrap_or_default();
                    self.open_dir(dir, ctx);
                    self.pending_reveal = Some(file);
                }
                ctx.send_viewport_cmd(egui::ViewportCommand::Title(format!(
                    "gridview - {}",
                    self.root.to_string_lossy()
                )));
            }
            Act::Close => {
                let y = self.layout.y_of.get(id as usize).copied().unwrap_or(0.0);
                if y < self.scroll_y || y > self.scroll_y + self.view_h * 0.8 {
                    self.scroll_target = Some(((y - self.view_h * 0.3).max(0.0), 1));
                }
                self.viewer = None;
                ctx.send_viewport_cmd(egui::ViewportCommand::Title(format!(
                    "gridview - {}",
                    self.root.to_string_lossy()
                )));
            }
            Act::Step(d) => {
                let pos = self.order_pos[id as usize] as i32 + d;
                if pos >= 0 && (pos as usize) < self.order.len() {
                    let nid = self.order[pos as usize];
                    self.open_viewer(ctx, nid);
                }
            }
            Act::Reveal => reveal_in_file_manager(&self.items[id as usize].path),
            Act::External => open_external(&self.items[id as usize].path),
        }
    }

    // ------------------------------------------------------------------ grid
    fn grid_ui(&mut self, ui: &mut egui::Ui) {
        if self.groups_dirty {
            self.rebuild_groups();
        }
        if self.search.dirty && self.search.open {
            self.refresh_search();
        }
        let outer = ui.available_width();
        let width = (outer - self.bar_reserve).max(100.0);
        let throttled = self.scanning && self.last_layout.elapsed() < Duration::from_millis(200);
        let structural = self.layout.columns != self.columns
            || self.layout.gap != self.gap
            || self.layout.compact != self.compact
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
        // Dim non-matching tiles only when there is something to show instead.
        let search_on = self.search.active() && self.search.total() > 0;
        let current_hit = if search_on {
            self.search.current_id()
        } else {
            None
        };
        let mut current_rect = None;
        let mut flash = None;
        if let Some((fid, t0)) = self.flash {
            let el = t0.elapsed().as_secs_f32();
            if el >= FLASH_SECS {
                self.flash = None;
            } else {
                flash = Some((fid, el / FLASH_SECS, None::<Rect>));
                ctx.request_repaint();
            }
        }
        for (id, r) in vis {
            if let Some(f) = flash.as_mut().filter(|f| f.0 == id) {
                f.2 = Some(r);
            }
            let item = &self.items[id as usize];
            let group = self.layout.group_of.get(id as usize).copied().unwrap_or(0);
            if self.layout.compact {
                painter.rect_filled(r.expand(self.layout.gap * 0.5), 0.0, group_shade(group));
            }
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
            if search_on {
                search::paint_tile_overlay(&painter, r, self.search.is_hit(id));
                if current_hit == Some(id) {
                    current_rect = Some((r, id));
                }
            }
            let resp = ui.interact(r, Id::new(("tile", id)), Sense::click());
            let resp = resp.on_hover_ui(|ui| {
                ui.set_max_width(460.0);
                let rel = search::rel_path(&self.root, &item.path);
                let words: &[String] = if self.search.active() {
                    self.search.words()
                } else {
                    &[]
                };
                ui.label(search::path_job(
                    &rel,
                    words,
                    FontId::proportional(13.0),
                    ui.visuals().text_color(),
                    440.0,
                    usize::MAX,
                ));
                if self.layout.compact {
                    if let Some(g) = self.groups.get(group as usize) {
                        ui.label(
                            RichText::new(format!("{}  ({} items)", g.label, g.ids.len()))
                                .small()
                                .weak(),
                        );
                    }
                }
            });
            let menu_open = resp.context_menu_opened();
            resp.context_menu(|ui| {
                if ui.button("🚀 Open externally").clicked() {
                    open_external(&item.path);
                    ui.close_menu();
                }
                if ui.button("📂 Show in folder").clicked() {
                    reveal_in_file_manager(&item.path);
                    ui.close_menu();
                }
                if ui.button("📋 Copy path").clicked() {
                    copy_full_path(ui.ctx(), &item.path);
                    ui.close_menu();
                }
            });
            if resp.middle_clicked() {
                reveal_in_file_manager(&item.path);
            } else if resp.clicked() && !menu_open {
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
        // Painted last so no neighbouring tile can cover the glow.
        if let Some((r, id)) = current_rect {
            let rel = search::rel_path(&self.root, &self.items[id as usize].path);
            search::paint_current(&painter, r, &rel, self.search.words());
        }
        if let Some((_, t, Some(r))) = flash {
            paint_flash(&painter, r, t);
        }
        if let Some(id) = clicked {
            self.open_viewer(&ctx, id);
        }
    }
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _: &mut eframe::Frame) {
        self.poll_scan();
        self.poll_thumbs(ctx);

        // handle fullscreen f11
        if ctx.input(|i| i.key_pressed(egui::Key::F11)) {
            let fullscreen = ctx.input(|i| i.viewport().fullscreen.unwrap_or(false));
            ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(!fullscreen));
        }

        // Ctrl+F opens the search, Esc closes it, F3 / Shift+F3 jump between hits.
        // (Ignored while the viewer is open: it owns Esc and the arrow keys.)
        let mut key_step = 0;
        if self.viewer.is_none() {
            let was_open = self.search.open;
            let (find, esc, prev, next) = ctx.input_mut(|i| {
                (
                    i.consume_key(Modifiers::COMMAND, Key::F),
                    was_open && i.consume_key(Modifiers::NONE, Key::Escape),
                    was_open && i.consume_key(Modifiers::SHIFT, Key::F3),
                    was_open && i.consume_key(Modifiers::NONE, Key::F3),
                )
            });
            if find {
                self.search.open_dialog();
            } else if esc {
                self.search.open = false;
            }
            key_step = if prev {
                -1
            } else if next {
                1
            } else {
                0
            };
        }

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

        // The viewer zooms with the wheel; keep the grid underneath from scrolling too.
        self.viewer_scroll = 0.0;
        if self.viewer.is_some() {
            self.viewer_scroll = ctx.input_mut(|i| {
                let s = i.smooth_scroll_delta.y;
                i.smooth_scroll_delta = Vec2::ZERO;
                i.raw_scroll_delta = Vec2::ZERO;
                s
            });
        }

        egui::TopBottomPanel::top("top").show(ctx, |ui| {
            ui.add_space(3.0);
            ui.horizontal(|ui| {
                let te =
                    ui.add(egui::TextEdit::singleline(&mut self.path_input).desired_width(340.0));
                if ui.button("...").clicked() {
                    if let Some(path) = rfd::FileDialog::new()
                        .set_directory(&self.path_input)
                        .pick_folder()
                    {
                        self.path_input = path.to_string_lossy().into_owned();
                    }
                }
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
                ui.separator();
                if ui
                    .checkbox(&mut self.compact, "Compact")
                    .on_hover_text(
                        "Pack small groups side by side to use the full width.\n\
                         Groups get different background shades; hover a tile to see its group.",
                    )
                    .changed()
                {
                    self.gap = gap_for(self.compact);
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui
                        .selectable_label(self.search.open, "🔍 Find")
                        .on_hover_text("Find files by path (Ctrl+F)")
                        .clicked()
                    {
                        if self.search.open {
                            self.search.open = false;
                        } else {
                            self.search.open_dialog();
                        }
                    }
                });
            });
            ui.add_space(3.0);
        });

        egui::TopBottomPanel::bottom("status").show(ctx, |ui| {
            ui.add_space(3.0);

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
            ui.add_space(3.0);
        });

        // What is left below the top bar: the search dialog hangs off its top right.
        let avail = ctx.available_rect();

        egui::CentralPanel::default()
            .frame(egui::Frame::none().fill(Color32::from_gray(16)))
            .show(ctx, |ui| self.grid_ui(ui));

        if self.viewer.is_none() {
            let out = search::show(ctx, avail, &mut self.search);
            if out.query_changed {
                // Hits are recomputed next frame (they need the grid order);
                // then we scroll to the current one.
                self.search.dirty = true;
                self.search_reveal = true;
                ctx.request_repaint();
            }
            if out.close {
                self.search.open = false;
            }
            let step = if out.step != 0 { out.step } else { key_step };
            if step != 0 {
                if let Some(id) = self.search.step(step) {
                    self.reveal_item(id, true, ctx);
                }
            }
            if self.search_reveal && !self.search.dirty {
                self.search_reveal = false;
                if let Some(id) = self.search.current_id() {
                    self.reveal_item(id, true, ctx);
                }
            }
        }

        // Came here from single file mode: once the folder has a layout, jump to the file.
        if self.pending_reveal.is_some()
            && !self.groups_dirty
            && !self.items.is_empty()
            && self.layout.y_of.len() == self.items.len()
        {
            if let Some(file) = self.pending_reveal.take() {
                if let Some(id) = self.items.iter().position(|it| it.path == file) {
                    self.reveal_item(id as u32, false, ctx);
                    self.flash = Some((id as u32, Instant::now()));
                }
            }
        }

        self.viewer_ui(ctx);
    }
}

fn main() -> eframe::Result<()> {
    let args = Args::parse();

    // First argument that is a folder, or a supported image/video file (single file mode).
    let target = args.paths.iter().find_map(|p| {
        if p.is_dir() {
            Some((p.clone(), None))
        } else if p.is_file() && scan::classify(p).is_some() {
            let abs = std::path::absolute(p).unwrap_or_else(|_| p.clone());
            let dir = abs.parent().map(Path::to_path_buf).unwrap_or_default();
            Some((dir, Some(abs)))
        } else {
            None
        }
    });
    let (root, single_file) = target.unwrap_or_else(|| {
        let dir = directories::UserDirs::new()
            .and_then(|d| d.picture_dir().map(|p| p.to_path_buf()))
            .unwrap_or_else(|| std::env::current_dir().unwrap());
        (dir, None)
    });

    let (group_mode, gran) = match args.sort {
        SortMode::Folder => (GroupMode::Folder, DateGran::Month),
        SortMode::ModifiedDay => (GroupMode::Modified, DateGran::Day),
        SortMode::ModifiedMonth => (GroupMode::Modified, DateGran::Month),
        SortMode::CreatedDay => (GroupMode::Created, DateGran::Day),
        SortMode::CreatedMonth => (GroupMode::Created, DateGran::Month),
    };

    let config = AppConfig {
        root,
        single_file,
        width: args.width,
        height: args.height,
        fullscreen: args.fullscreen,
        columns: args.columns,
        group_mode,
        gran,
        compact: args.compact,
    };

    // load app icon
    use std::sync::Arc;
    let icon_bytes = include_bytes!("../assets/icon.png");
    let icon_image = image::load_from_memory(icon_bytes).expect("Failed to load application icon");
    let icon_rgba = icon_image.to_rgba8();
    let icon = egui::IconData {
        rgba: icon_rgba.into_raw(),
        width: icon_image.width(),
        height: icon_image.height(),
    };

    let opts = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([config.width as f32, config.height as f32])
            .with_fullscreen(config.fullscreen)
            .with_title(APP_NAME)
            .with_icon(Arc::new(icon)),
        ..Default::default()
    };

    eframe::run_native(
        APP_NAME,
        opts,
        Box::new(move |cc| Box::new(App::new(cc, config))),
    )
}

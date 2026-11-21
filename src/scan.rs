//! Recursive directory walk + (cached, parallel) header-only dimension probing.
//! The UI gets the file list as soon as the walk ends; dimensions stream in
//! afterwards so the layout can refine itself without blocking anything.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs::{self, File};
use std::io::BufReader;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{channel, Receiver};
use std::sync::{Arc, Mutex};
use std::time::UNIX_EPOCH;
use walkdir::WalkDir;

use rayon::prelude::*;

use crate::cache;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    Image,
    Video,
}

#[derive(Clone)]
pub struct Item {
    pub path: PathBuf,
    pub kind: Kind,
    pub modified: i64,
    pub created: i64,
    pub size: u64,
    pub w: u32,
    pub h: u32,
}

impl Item {
    pub fn aspect(&self) -> f32 {
        if self.w == 0 || self.h == 0 {
            return if self.kind == Kind::Video {
                16.0 / 9.0
            } else {
                4.0 / 3.0
            };
        }
        (self.w as f32 / self.h as f32).clamp(0.1, 10.0)
    }
}

pub enum ScanMsg {
    Walked(Vec<Item>),
    Dims(Vec<(u32, u32, u32)>),
    Done,
}

pub struct ScanHandle {
    pub rx: Receiver<ScanMsg>,
    pub found: Arc<AtomicUsize>,
}

const IMAGE_EXT: &[&str] = &["jpg", "jpeg", "png", "gif", "bmp", "webp", "tif", "tiff"];
const VIDEO_EXT: &[&str] = &[
    "mp4", "mkv", "webm", "mov", "avi", "m4v", "wmv", "flv", "mpg", "mpeg", "ts", "3gp",
];

pub fn classify(p: &Path) -> Option<Kind> {
    let ext = p.extension()?.to_str()?.to_ascii_lowercase();
    if IMAGE_EXT.contains(&ext.as_str()) {
        Some(Kind::Image)
    } else if VIDEO_EXT.contains(&ext.as_str()) {
        Some(Kind::Video)
    } else {
        None
    }
}

/// Build an `Item` for one file (single file mode). Dimensions stay 0 until probed.
pub fn item_from_path(path: &Path) -> Option<Item> {
    let kind = classify(path)?;
    let md = fs::metadata(path).ok()?;
    if !md.is_file() {
        return None;
    }
    let modified = secs(md.modified()).unwrap_or(0);
    let created = secs(md.created()).unwrap_or(modified);
    Some(Item {
        path: path.to_path_buf(),
        kind,
        modified,
        created,
        size: md.len(),
        w: 0,
        h: 0,
    })
}

fn secs(t: std::io::Result<std::time::SystemTime>) -> Option<i64> {
    let d = t.ok()?.duration_since(UNIX_EPOCH).ok()?;
    Some(d.as_secs() as i64)
}

#[derive(Serialize, Deserialize, Clone)]
struct CacheEntry {
    mtime: i64,
    size: u64,
    w: u32,
    h: u32,
}

/// The size cache of one cache location (see `cache`): files on an encrypted
/// drive are cached on that drive, so each location has its own.
fn dims_cache_path(dir: &Path) -> PathBuf {
    dir.join("dims.bin")
}

fn load_dims_cache(dir: &Path) -> HashMap<String, CacheEntry> {
    fs::read(dims_cache_path(dir))
        .ok()
        .and_then(|b| bincode::deserialize(&b).ok())
        .unwrap_or_default()
}

fn save_dims_cache(dir: &Path, map: &HashMap<String, CacheEntry>) {
    let _ = fs::create_dir_all(dir);
    if let Ok(b) = bincode::serialize(map) {
        let _ = fs::write(dims_cache_path(dir), b);
    }
}

/// EXIF orientation (1..=8). Only JPEGs matter in practice.
pub fn orientation(path: &Path) -> u32 {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    if ext != "jpg" && ext != "jpeg" {
        return 1;
    }
    let Ok(f) = File::open(path) else { return 1 };
    let mut r = BufReader::new(f);
    exif::Reader::new()
        .read_from_container(&mut r)
        .ok()
        .and_then(|e| {
            e.get_field(exif::Tag::Orientation, exif::In::PRIMARY)
                .and_then(|f| f.value.get_uint(0))
        })
        .unwrap_or(1)
}

/// Displayed (orientation-corrected) size, reading only file headers.
fn probe(path: &Path, kind: Kind) -> (u32, u32) {
    match kind {
        Kind::Image => {
            let (w, h) = match imagesize::size(path) {
                Ok(s) => (s.width as u32, s.height as u32),
                Err(_) => image::image_dimensions(path).unwrap_or((0, 0)),
            };
            if (5..=8).contains(&orientation(path)) {
                (h, w)
            } else {
                (w, h)
            }
        }
        #[cfg(feature = "video-thumbnails")]
        Kind::Video => crate::video::probe(path).unwrap_or((0, 0)),

        #[cfg(not(feature = "video-thumbnails"))]
        Kind::Video => (0, 0),
    }
}

pub fn spawn_scan(root: PathBuf, ctx: eframe::egui::Context) -> ScanHandle {
    let (tx, rx) = channel();
    let found = Arc::new(AtomicUsize::new(0));
    let found2 = found.clone();

    std::thread::spawn(move || {
        // ---- 1. walk ----
        let mut items: Vec<Item> = Vec::new();
        let walker = WalkDir::new(&root)
            .follow_links(false)
            .into_iter()
            .filter_entry(|e| {
                // skip hidden directories (.git, .cache, ...) but never the root itself
                e.depth() == 0 || !e.file_name().to_string_lossy().starts_with('.')
            });
        for e in walker.filter_map(|e| e.ok()) {
            if !e.file_type().is_file() {
                continue;
            }
            let Some(kind) = classify(e.path()) else {
                continue;
            };
            let Ok(md) = e.metadata() else { continue };
            let modified = secs(md.modified()).unwrap_or(0);
            let created = secs(md.created()).unwrap_or(modified);
            items.push(Item {
                path: e.into_path(),
                kind,
                modified,
                created,
                size: md.len(),
                w: 0,
                h: 0,
            });
            found2.store(items.len(), Ordering::Relaxed);
            if items.len() % 2000 == 0 {
                ctx.request_repaint();
            }
        }
        let probe_list: Vec<(u32, PathBuf, Kind, i64, u64)> = items
            .iter()
            .enumerate()
            .map(|(i, it)| (i as u32, it.path.clone(), it.kind, it.modified, it.size))
            .collect();
        let _ = tx.send(ScanMsg::Walked(items));
        ctx.request_repaint();

        // ---- 2. probe dimensions in parallel, using the on-disk caches ----
        // Every file belongs to a cache location (usually the one in ~/.cache,
        // but files on an encrypted drive are cached on that drive).
        let mut loc_dirs: Vec<PathBuf> = Vec::new();
        let mut loc_index: HashMap<PathBuf, usize> = HashMap::new();
        let probe_list: Vec<(u32, PathBuf, Kind, i64, u64, Option<usize>)> = probe_list
            .into_iter()
            .map(|(id, p, kind, mt, sz)| {
                let loc = cache::dir_for(&p).map(|d| {
                    *loc_index.entry(d.clone()).or_insert_with(|| {
                        loc_dirs.push(d);
                        loc_dirs.len() - 1
                    })
                });
                (id, p, kind, mt, sz, loc)
            })
            .collect();
        let caches: Vec<HashMap<String, CacheEntry>> =
            loc_dirs.iter().map(|d| load_dims_cache(d)).collect();
        let fresh: Vec<Mutex<HashMap<String, CacheEntry>>> = loc_dirs
            .iter()
            .map(|_| Mutex::new(HashMap::new()))
            .collect();
        let threads = (std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(4)
            * 2)
        .max(4);
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(threads)
            .build()
            .unwrap();
        pool.install(|| {
            probe_list.par_chunks(128).for_each(|chunk| {
                let mut out = Vec::with_capacity(chunk.len());
                let mut local = Vec::new();
                for (id, p, kind, mt, sz, loc) in chunk {
                    let key = p.to_string_lossy().into_owned();
                    let hit = loc
                        .and_then(|l| caches[l].get(&key))
                        .filter(|e| e.mtime == *mt && e.size == *sz);
                    let (w, h) = match hit {
                        Some(e) => (e.w, e.h),
                        None => probe(p, *kind),
                    };
                    if let Some(l) = loc {
                        local.push((
                            *l,
                            key,
                            CacheEntry {
                                mtime: *mt,
                                size: *sz,
                                w,
                                h,
                            },
                        ));
                    }
                    out.push((*id, w, h));
                }
                for (l, key, entry) in local {
                    fresh[l].lock().unwrap().insert(key, entry);
                }
                let _ = tx.send(ScanMsg::Dims(out));
                ctx.request_repaint();
            });
        });
        for (dir, map) in loc_dirs.iter().zip(fresh) {
            save_dims_cache(dir, &map.into_inner().unwrap());
        }
        let _ = tx.send(ScanMsg::Done);
        ctx.request_repaint();
    });

    ScanHandle { rx, found }
}

//! Thumbnail pipeline: a pool of worker threads that always serve the most
//! relevant request first. The UI replaces the "wanted" list every time the
//! viewport moves, so anything that scrolled away is simply never decoded.
//! Results are cached on disk as small JPEGs keyed by (path, mtime, size, tier).

use image::{DynamicImage, RgbImage};
use std::collections::HashSet;
use std::fs::{self, File};
use std::io::BufReader;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Condvar, Mutex};

use crate::scan::{self, Kind};

pub const TIERS: [u32; 3] = [256, 512, 1024];

pub fn tier_for(px: f32) -> u32 {
    for t in TIERS {
        if px <= t as f32 {
            return t;
        }
    }
    *TIERS.last().unwrap()
}

#[derive(Clone)]
pub struct Req {
    pub id: u32,
    pub path: PathBuf,
    pub kind: Kind,
    pub tier: u32,
    pub mtime: i64,
    pub size: u64,
}

pub struct Done {
    pub id: u32,
    pub tier: u32,
    pub img: Option<RgbImage>,
}

#[derive(Default)]
struct Queue {
    wanted: Vec<Req>,
    in_flight: HashSet<(u32, u32)>,
    stop: bool,
}

struct Shared {
    q: Mutex<Queue>,
    cv: Condvar,
}

pub struct Thumbs {
    shared: Arc<Shared>,
    pub rx: Receiver<Done>,
}

impl Thumbs {
    pub fn new(ctx: eframe::egui::Context) -> Self {
        let shared = Arc::new(Shared {
            q: Mutex::new(Queue::default()),
            cv: Condvar::new(),
        });
        let (tx, rx) = channel();
        let n = std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(4)
            .max(2);
        for _ in 0..n {
            let shared = shared.clone();
            let tx = tx.clone();
            let ctx = ctx.clone();
            std::thread::spawn(move || worker(shared, tx, ctx));
        }
        Thumbs { shared, rx }
    }

    /// Replace the pending work list (already sorted by priority).
    pub fn set_wanted(&self, reqs: Vec<Req>) {
        let mut q = self.shared.q.lock().unwrap();
        q.wanted = reqs;
        drop(q);
        self.shared.cv.notify_all();
    }
}

impl Drop for Thumbs {
    fn drop(&mut self) {
        self.shared.q.lock().unwrap().stop = true;
        self.shared.cv.notify_all();
    }
}

fn worker(shared: Arc<Shared>, tx: Sender<Done>, ctx: eframe::egui::Context) {
    loop {
        let req = {
            let mut q = shared.q.lock().unwrap();
            loop {
                if q.stop {
                    return;
                }
                let pos = q
                    .wanted
                    .iter()
                    .position(|r| !q.in_flight.contains(&(r.id, r.tier)));
                if let Some(pos) = pos {
                    let r = q.wanted.remove(pos);
                    q.in_flight.insert((r.id, r.tier));
                    break r;
                }
                q = shared.cv.wait(q).unwrap();
            }
        };
        let img = load_thumb(&req);
        let done = Done {
            id: req.id,
            tier: req.tier,
            img,
        };
        if tx.send(done).is_err() {
            return;
        }
        shared
            .q
            .lock()
            .unwrap()
            .in_flight
            .remove(&(req.id, req.tier));
        ctx.request_repaint();
    }
}

fn fnv(bytes: &[u8], mut h: u64) -> u64 {
    for b in bytes {
        h ^= *b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    h
}

fn cache_path(r: &Req) -> PathBuf {
    let mut h = 0xcbf29ce484222325u64;
    h = fnv(r.path.to_string_lossy().as_bytes(), h);
    h = fnv(&r.mtime.to_le_bytes(), h);
    h = fnv(&r.size.to_le_bytes(), h);
    scan::cache_dir()
        .join("thumbs")
        .join(format!("{:016x}_{}.jpg", h, r.tier))
}

fn load_thumb(r: &Req) -> Option<RgbImage> {
    let cp = cache_path(r);
    if let Some(img) = decode_jpeg(&cp, u32::MAX) {
        return Some(img);
    }
    let img = make_image(&r.path, r.kind, r.tier)?;
    if let Some(dir) = cp.parent() {
        let _ = fs::create_dir_all(dir);
    }
    let tmp = cp.with_extension("tmp");
    if let Ok(mut f) = File::create(&tmp) {
        let mut enc = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut f, 85);
        if enc
            .encode(
                img.as_raw(),
                img.width(),
                img.height(),
                image::ColorType::Rgb8,
            )
            .is_ok()
        {
            let _ = fs::rename(&tmp, &cp);
        } else {
            let _ = fs::remove_file(&tmp);
        }
    }
    Some(img)
}

/// Decode a JPEG, letting libjpeg-style DCT scaling shrink it by 1/2, 1/4 or
/// 1/8 *during* decoding. Pass `u32::MAX` for full size.
fn decode_jpeg(path: &Path, max_edge: u32) -> Option<RgbImage> {
    let f = File::open(path).ok()?;
    let mut d = jpeg_decoder::Decoder::new(BufReader::new(f));
    d.read_info().ok()?;
    let info = d.info()?;
    let (w, h) = (info.width as u32, info.height as u32);
    if max_edge != u32::MAX && w.max(h) > max_edge {
        let s = max_edge as f32 / w.max(h) as f32;
        let tw = ((w as f32 * s).ceil() as u32).clamp(1, 65535) as u16;
        let th = ((h as f32 * s).ceil() as u32).clamp(1, 65535) as u16;
        d.scale(tw, th).ok()?;
    }
    let px = d.decode().ok()?;
    let info = d.info()?;
    let (w, h) = (info.width as u32, info.height as u32);
    match info.pixel_format {
        jpeg_decoder::PixelFormat::RGB24 => RgbImage::from_raw(w, h, px),
        jpeg_decoder::PixelFormat::L8 => {
            let mut rgb = Vec::with_capacity(px.len() * 3);
            for g in px {
                rgb.extend_from_slice(&[g, g, g]);
            }
            RgbImage::from_raw(w, h, rgb)
        }
        _ => None,
    }
}

fn apply_orientation(img: DynamicImage, o: u32) -> DynamicImage {
    match o {
        2 => img.fliph(),
        3 => img.rotate180(),
        4 => img.flipv(),
        5 => img.rotate90().fliph(),
        6 => img.rotate90(),
        7 => img.rotate270().fliph(),
        8 => img.rotate270(),
        _ => img,
    }
}

/// Produce an RGB image whose longest side is at most `max_edge`.
pub fn make_image(path: &Path, kind: Kind, max_edge: u32) -> Option<RgbImage> {
    match kind {
        Kind::Video => crate::video::thumbnail(path, max_edge),
        Kind::Image => {
            let ext = path
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or("")
                .to_ascii_lowercase();
            let mut img = None;
            if ext == "jpg" || ext == "jpeg" {
                img = decode_jpeg(path, max_edge).map(DynamicImage::ImageRgb8);
            }
            let img = match img {
                Some(i) => i,
                None => image::open(path).ok()?,
            };
            let img = apply_orientation(img, scan::orientation(path));
            let img = if img.width().max(img.height()) > max_edge {
                img.thumbnail(max_edge, max_edge)
            } else {
                img
            };
            Some(img.to_rgb8())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scaled_jpeg_decode_and_exif_free_path() {
        let dir = std::env::temp_dir().join("gv_test");
        let _ = fs::create_dir_all(&dir);
        let p = dir.join("a.jpg");
        let img = RgbImage::from_fn(3000, 2000, |x, y| {
            image::Rgb([(x % 255) as u8, (y % 255) as u8, 90])
        });
        img.save(&p).unwrap();
        let t = std::time::Instant::now();
        let out = make_image(&p, Kind::Image, 512).unwrap();
        eprintln!(
            "3000x2000 -> {}x{} in {:?}",
            out.width(),
            out.height(),
            t.elapsed()
        );
        assert!(out.width().max(out.height()) <= 512 && out.width().max(out.height()) >= 400);
    }
}

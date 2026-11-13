//! Video thumbnails through libav* linked into the process (no `ffmpeg`
//! subprocess). Each worker thread opens the file, seeks to ~10 %, decodes
//! the first frame and scales it with swscale.

use image::RgbImage;
use std::path::Path;

#[cfg(feature = "video")]
mod imp {
    use super::*;
    use ffmpeg::format::Pixel;
    use ffmpeg::media::Type;
    use ffmpeg::software::scaling::{Context as Scaler, Flags};
    use ffmpeg_next as ffmpeg;
    use std::sync::Once;

    fn init() {
        static INIT: Once = Once::new();
        INIT.call_once(|| {
            let _ = ffmpeg::init();
            ffmpeg::util::log::set_level(ffmpeg::util::log::Level::Quiet);
        });
    }

    /// Clockwise rotation (0/90/180/270) from the display-matrix side data.
    fn rotation(stream: &ffmpeg::format::stream::Stream) -> i32 {
        for sd in stream.side_data() {
            if sd.kind() == ffmpeg::packet::side_data::Type::DisplayMatrix {
                let d = sd.data();
                if d.len() >= 36 {
                    let mut m = [0i32; 9];
                    for (i, c) in d.chunks_exact(4).take(9).enumerate() {
                        m[i] = i32::from_ne_bytes([c[0], c[1], c[2], c[3]]);
                    }
                    let angle = unsafe { ffmpeg::ffi::av_display_rotation_get(m.as_ptr()) };
                    if angle.is_finite() {
                        return ((-angle).round() as i32).rem_euclid(360);
                    }
                }
            }
        }
        0
    }

    pub fn probe(path: &Path) -> Option<(u32, u32)> {
        init();
        let ictx = ffmpeg::format::input(&path).ok()?;
        let s = ictx.streams().best(Type::Video)?;
        let (w, h) = unsafe {
            let p = s.parameters().as_ptr();
            ((*p).width as u32, (*p).height as u32)
        };
        if w == 0 || h == 0 {
            return None;
        }
        Some(if matches!(rotation(&s), 90 | 270) {
            (h, w)
        } else {
            (w, h)
        })
    }

    fn grab(path: &Path, max_edge: u32, seek: bool) -> Option<RgbImage> {
        let mut ictx = ffmpeg::format::input(&path).ok()?;
        let (idx, rot, params) = {
            let s = ictx.streams().best(Type::Video)?;
            (s.index(), rotation(&s), s.parameters())
        };
        let mut dec = ffmpeg::codec::context::Context::from_parameters(params)
            .ok()?
            .decoder()
            .video()
            .ok()?;
        if seek {
            let dur = ictx.duration();
            if dur > 0 {
                let ts = dur / 10;
                ictx.seek(ts, ..ts).ok()?;
            }
        }
        let mut frame = ffmpeg::util::frame::Video::empty();
        let mut got = false;
        for (stream, packet) in ictx.packets() {
            if stream.index() != idx {
                continue;
            }
            if dec.send_packet(&packet).is_err() {
                continue;
            }
            if dec.receive_frame(&mut frame).is_ok() {
                got = true;
                break;
            }
        }
        if !got {
            let _ = dec.send_eof();
            got = dec.receive_frame(&mut frame).is_ok();
        }
        if !got {
            return None;
        }

        let (w, h) = (frame.width(), frame.height());
        let scale = (max_edge as f32 / w.max(h) as f32).min(1.0);
        let tw = (((w as f32 * scale).round() as u32).max(2)) & !1;
        let th = (((h as f32 * scale).round() as u32).max(2)) & !1;
        let mut scaler =
            Scaler::get(frame.format(), w, h, Pixel::RGB24, tw, th, Flags::AREA).ok()?;
        let mut rgb = ffmpeg::util::frame::Video::empty();
        scaler.run(&frame, &mut rgb).ok()?;

        let stride = rgb.stride(0);
        let data = rgb.data(0);
        let mut buf = Vec::with_capacity((tw * th * 3) as usize);
        for y in 0..th as usize {
            buf.extend_from_slice(&data[y * stride..y * stride + tw as usize * 3]);
        }
        let img = RgbImage::from_raw(tw, th, buf)?;
        Some(match rot {
            90 => image::imageops::rotate90(&img),
            180 => image::imageops::rotate180(&img),
            270 => image::imageops::rotate270(&img),
            _ => img,
        })
    }

    pub fn thumbnail(path: &Path, max_edge: u32) -> Option<RgbImage> {
        init();
        grab(path, max_edge, true).or_else(|| grab(path, max_edge, false))
    }
}

#[cfg(not(feature = "video"))]
mod imp {
    use super::*;
    pub fn probe(_: &Path) -> Option<(u32, u32)> {
        None
    }
    pub fn thumbnail(_: &Path, _: u32) -> Option<RgbImage> {
        None
    }
}

pub fn probe(path: &Path) -> Option<(u32, u32)> {
    imp::probe(path)
}
pub fn thumbnail(path: &Path, max_edge: u32) -> Option<RgbImage> {
    imp::thumbnail(path, max_edge)
}

#[cfg(all(test, feature = "video"))]
mod tests {
    #[test]
    fn thumbnail_from_generated_clip() {
        let p = std::path::Path::new("/tmp/gv_vid/t.mp4");
        if !p.exists() {
            return;
        }
        assert_eq!(super::probe(p), Some((1280, 720)));
        let t = std::time::Instant::now();
        let img = super::thumbnail(p, 512).expect("frame");
        eprintln!(
            "video thumb {}x{} in {:?}",
            img.width(),
            img.height(),
            t.elapsed()
        );
        assert_eq!(img.width(), 512);
    }
}

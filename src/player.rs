//! GStreamer-backed video playback for the egui viewer.
//!
//! Video frames are delivered through a one-frame channel so the UI never
//! accumulates an unbounded queue when rendering falls behind.

use std::path::Path;
use std::sync::mpsc::{sync_channel, Receiver, SyncSender, TryRecvError, TrySendError};
use std::time::Duration;

use eframe::egui;

use gstreamer as gst;
use gstreamer::prelude::*;
use gstreamer_app as gst_app;
use gstreamer_video as gst_video;
use gstreamer_video::prelude::*;

#[derive(Debug)]
pub struct VideoFrame {
    pub width: usize,
    pub height: usize,
    /// Tightly packed RGB pixels, three bytes per pixel.
    pub pixels: Vec<u8>,
}

pub struct Player {
    playbin: gst::Element,
    frames: Receiver<VideoFrame>,
    playing: bool,
    volume: f64,
}

impl Player {
    /// Open a local video, start playback, and route audio to GStreamer's
    /// default audio sink.
    pub fn open(path: &Path, ctx: egui::Context) -> Result<Self, String> {
        gst::init().map_err(|e| format!("GStreamer initialization failed: {e}"))?;

        let absolute_path = path
            .canonicalize()
            .map_err(|e| format!("Could not resolve video path: {e}"))?;
        let uri = gst::glib::filename_to_uri(&absolute_path, None)
            .map_err(|e| format!("Could not create video URI: {e}"))?;

        let caps = gst_video::VideoCapsBuilder::new()
            .format(gst_video::VideoFormat::Rgb)
            .build();

        let appsink = gst_app::AppSink::builder().caps(&caps).build();
        // Keep at most one decoded frame waiting for the UI.
        appsink.set_property("max-buffers", 1u32);
        appsink.set_property("drop", true);
        appsink.set_property("sync", true);

        let (tx, rx) = sync_channel::<VideoFrame>(1);
        install_frame_callback(&appsink, tx, ctx);

        let playbin = gst::ElementFactory::make("playbin")
            .build()
            .map_err(|e| format!("Could not create GStreamer playbin: {e}"))?;

        playbin.set_property("uri", uri.as_str());
        playbin.set_property("video-sink", &appsink);
        // playbin's default audio sink is selected automatically.
        playbin.set_property("volume", 1.0f64);

        playbin
            .set_state(gst::State::Playing)
            .map_err(|e| format!("Could not start video playback: {e}"))?;

        Ok(Self {
            playbin,
            frames: rx,
            playing: true,
            volume: 1.0,
        })
    }

    /// Return the newest decoded frame, if one is waiting.
    pub fn take_frame(&mut self) -> Option<VideoFrame> {
        let mut newest = None;
        loop {
            match self.frames.try_recv() {
                Ok(frame) => newest = Some(frame),
                Err(TryRecvError::Empty | TryRecvError::Disconnected) => break,
            }
        }
        newest
    }

    pub fn toggle_play(&mut self) {
        self.playing = !self.playing;
        let state = if self.playing {
            gst::State::Playing
        } else {
            gst::State::Paused
        };
        if let Err(e) = self.playbin.set_state(state) {
            eprintln!("GStreamer state change failed: {e}");
        }
    }

    pub fn is_playing(&self) -> bool {
        self.playing
    }

    pub fn position(&self) -> f64 {
        self.playbin
            .query_position::<gst::ClockTime>()
            .map(|t| t.seconds() as f64)
            .unwrap_or(0.0)
    }

    pub fn duration(&self) -> f64 {
        self.playbin
            .query_duration::<gst::ClockTime>()
            .map(|t| t.seconds() as f64)
            .unwrap_or(0.0)
    }

    pub fn seek(&self, seconds: f64) {
        let target = gst::ClockTime::from_seconds(seconds.max(0.0) as u64);
        if let Err(e) = self
            .playbin
            .seek_simple(gst::SeekFlags::FLUSH | gst::SeekFlags::KEY_UNIT, target)
        {
            eprintln!("GStreamer seek failed: {e}");
        }
    }

    pub fn volume(&self) -> f64 {
        self.volume
    }

    pub fn set_volume(&mut self, volume: f64) {
        self.volume = volume.clamp(0.0, 1.0);
        self.playbin.set_property("volume", self.volume);
    }
}

impl Drop for Player {
    fn drop(&mut self) {
        // Setting NULL stops streaming threads and releases decoder/audio
        // resources before another file is opened.
        let _ = self.playbin.set_state(gst::State::Null);
    }
}

fn install_frame_callback(
    appsink: &gst_app::AppSink,
    tx: SyncSender<VideoFrame>,
    ctx: egui::Context,
) {
    appsink.set_callbacks(
        gst_app::AppSinkCallbacks::builder()
            .new_sample(move |sink| {
                let sample = sink.pull_sample().map_err(|_| gst::FlowError::Eos)?;
                let caps = sample.caps().ok_or(gst::FlowError::NotNegotiated)?;
                let info = gst_video::VideoInfo::from_caps(caps)
                    .map_err(|_| gst::FlowError::NotNegotiated)?;
                let buffer = sample.buffer().ok_or(gst::FlowError::Error)?;

                let frame = gst_video::VideoFrame::from_buffer_readable(
                    buffer.to_owned(),
                    &info,
                )
                .map_err(|_| gst::FlowError::Error)?;

                let width = frame.width() as usize;
                let height = frame.height() as usize;
                let stride = frame.plane_stride()[0].unsigned_abs() as usize;
                let row_bytes = width * 3;
                let source = frame.plane_data(0).map_err(|_| gst::FlowError::Error)?;

                if stride < row_bytes || source.len() < stride.saturating_mul(height) {
                    return Err(gst::FlowError::Error);
                }

                let mut pixels = Vec::with_capacity(row_bytes * height);
                for y in 0..height {
                    let start = y * stride;
                    pixels.extend_from_slice(&source[start..start + row_bytes]);
                }

                let video_frame = VideoFrame {
                    width,
                    height,
                    pixels,
                };

                // If the UI has not consumed the previous frame yet, drop
                // this one rather than blocking GStreamer's streaming thread.
                match tx.try_send(video_frame) {
                    Ok(()) | Err(TrySendError::Full(_)) => {}
                    Err(TrySendError::Disconnected(_)) => return Err(gst::FlowError::Flushing),
                }

                ctx.request_repaint_after(Duration::from_millis(16));
                Ok(gst::FlowSuccess::Ok)
            })
            .build(),
    );
}

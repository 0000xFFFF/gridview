use clap::{Parser, ValueEnum};
use std::path::PathBuf;

use crate::globals::{
    APP_NAME,
    SLIDER_COLUMNS_DEFAULT,
    WINDOW_HEIGHT,
    WINDOW_WIDTH,
};

#[derive(Parser, Debug)]
#[command(
    name = APP_NAME,
    version,
    about = "Image and video viewer"
)]
pub struct Args {
    /// Window width
    #[arg(long, default_value_t = WINDOW_WIDTH as u32)]
    pub width: u32,

    /// Window height
    #[arg(long, default_value_t = WINDOW_HEIGHT as u32)]
    pub height: u32,

    /// Start fullscreen
    #[arg(short = 'f', long)]
    pub fullscreen: bool,

    /// Number of columns
    #[arg(short = 'c', long, default_value_t = SLIDER_COLUMNS_DEFAULT)]
    pub columns: usize,

    /// Sort/group files
    #[arg(short = 's', long, value_enum, default_value_t = SortMode::CreatedDay)]
    pub sort: SortMode,

    /// Compact layout: pack small groups side by side instead of one group per row block
    #[arg(short = 'C', long)]
    pub compact: bool,

    /// Where thumbnails and image sizes are cached. `auto`: on the drive itself
    /// (`<mount>/.gridview-cache`) when the files are on an encrypted drive, so
    /// nothing leaks onto the unencrypted system drive, else in the user cache
    /// dir. `central`: always the user cache dir. `drive`: always on the files' drive
    /// (encryption is only detected on Linux, use this elsewhere)
    #[arg(long, value_enum, default_value_t = CacheMode::Auto)]
    pub cache: CacheMode,

    /// Directory to open, or a single image/video file to view
    /// (Esc then loads the file's folder and scrolls to it)
    #[arg(value_name = "PATH", num_args = 0..)]
    pub paths: Vec<PathBuf>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum CacheMode {
    Auto,
    Central,
    Drive,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum SortMode {
    Folder,
    ModifiedDay,
    ModifiedMonth,
    CreatedDay,
    CreatedMonth,
}

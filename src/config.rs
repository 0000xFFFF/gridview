use crate::globals::{GroupMode, DateGran};

use std::path::PathBuf;

pub struct AppConfig {
    pub root: PathBuf,
    pub width: u32,
    pub height: u32,
    pub fullscreen: bool,
    pub columns: usize,
    pub group_mode: GroupMode,
    pub gran: DateGran,
    pub compact: bool,
}


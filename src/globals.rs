use eframe::egui::TextureHandle;

pub const TEX_BUDGET_BYTES: usize = 400 * 1024 * 1024;

pub const APP_NAME: &str = "gridview";
pub const SLIDER_COLUMNS_DEFAULT: usize = 5;
pub const SLIDER_COLUMNS_MIN: usize = 1;
pub const SLIDER_COLUMNS_MAX: usize = 45;
pub const WINDOW_WIDTH: f32 = 1280.0;
pub const WINDOW_HEIGHT: f32 = 800.0;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum GroupMode {
    Folder,
    Modified,
    Created,
}
impl GroupMode {
    pub fn label(self) -> &'static str {
        match self {
            GroupMode::Folder => "By folder",
            GroupMode::Modified => "By date modified",
            GroupMode::Created => "By date created",
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum DateGran {
    Day,
    Month,
}


pub struct Tex {
    pub handle: TextureHandle,
    pub tier: u32,
    pub bytes: usize,
}

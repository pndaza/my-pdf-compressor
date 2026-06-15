use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    Bw,
    Color,
}

#[derive(Debug, Clone, Serialize)]
pub struct ImageInfo {
    pub index: usize,
    pub page: u32,
    pub width: u32,
    pub height: u32,
    pub is_color: bool,
    pub original_size: usize,
    pub thumbnail: String,
    pub mode: Mode,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ImageChoice {
    pub index: usize,
    pub mode: Mode,
}

#[derive(Debug, Serialize)]
pub struct CompressResult {
    pub output_path: String,
    pub original_size: u64,
    pub compressed_size: u64,
    pub image_count: usize,
}

pub struct DecodedImage {
    pub width: u32,
    pub height: u32,
    pub data: Vec<u8>,
    pub is_color: bool,
    pub dpi: f64,
}

#[derive(Clone)]
pub struct CompressedImage {
    pub data: Vec<u8>,
    pub width: u32,
    pub height: u32,
    pub color_space: &'static str,
    pub bits_per_component: u8,
    pub filter: &'static str,
    pub is_ccitt: bool,
    pub dpi: f64,
}
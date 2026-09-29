//! Image decoding helpers. Everything here is safe to call from a worker
//! thread: `gdk::Texture` is immutable and thread-safe.

use gtk::{gdk, gdk_pixbuf, gio, glib, prelude::*};

/// Longest thumbnail edge in pixels. Media cards are about 330 logical px
/// wide, so this stays sharp at 125 % scaling and reasonable at 150 %.
pub const THUMB_SIZE: i32 = 512;
/// Longest edge for the preview pane and details view.
pub const PREVIEW_SIZE: i32 = 2048;

pub struct Decoded {
    pub width: i32,
    pub height: i32,
    /// blake3 of the RGBA pixels: equal for the same picture whatever encoder
    /// produced the PNG, unlike a hash of the file bytes.
    pub pixel_hash: [u8; 32],
    pub thumbnail: gdk::Texture,
}

pub fn decode(bytes: &[u8]) -> Result<Decoded, glib::Error> {
    let stream = gio::MemoryInputStream::from_bytes(&glib::Bytes::from(bytes));
    let full = gdk_pixbuf::Pixbuf::from_stream(&stream, gio::Cancellable::NONE)?;
    Ok(Decoded {
        width: full.width(),
        height: full.height(),
        pixel_hash: pixel_hash(&full),
        thumbnail: texture_from_pixbuf(&flatten(&scale_down(&full, THUMB_SIZE))),
    })
}

/// Hash the image as 8-bit RGBA rows, skipping rowstride padding, so RGB vs
/// RGBA sources and different strides of the same picture agree.
fn pixel_hash(pb: &gdk_pixbuf::Pixbuf) -> [u8; 32] {
    let rgba = if pb.has_alpha() {
        pb.clone()
    } else {
        pb.add_alpha(false, 0, 0, 0).unwrap_or_else(|_| pb.clone())
    };
    let (w, h) = (rgba.width().max(0) as usize, rgba.height().max(0) as usize);
    let stride = rgba.rowstride().max(0) as usize;
    let row = w * rgba.n_channels().max(0) as usize;
    let bytes = rgba.read_pixel_bytes();
    let mut hasher = blake3::Hasher::new();
    hasher.update(&(w as u64).to_le_bytes());
    hasher.update(&(h as u64).to_le_bytes());
    for y in 0..h {
        let start = y * stride;
        if let Some(line) = bytes.get(start..start + row) {
            hasher.update(line);
        }
    }
    *hasher.finalize().as_bytes()
}

/// A thumbnail of the image file, transparency shown as a checkerboard.
pub fn thumbnail_from_file(path: &str) -> Result<gdk::Texture, glib::Error> {
    load_scaled(path, THUMB_SIZE)
}

/// The image at up to `PREVIEW_SIZE`, for the preview pane and details.
pub fn preview_from_file(path: &str) -> Result<gdk::Texture, glib::Error> {
    load_scaled(path, PREVIEW_SIZE)
}

fn load_scaled(path: &str, max: i32) -> Result<gdk::Texture, glib::Error> {
    let (_, w, h) = gdk_pixbuf::Pixbuf::file_info(path)
        .ok_or_else(|| glib::Error::new(glib::FileError::Inval, "not an image"))?;
    // Never upscale: small images keep their pixels.
    let pb = if w <= max && h <= max {
        gdk_pixbuf::Pixbuf::from_file(path)?
    } else {
        gdk_pixbuf::Pixbuf::from_file_at_scale(path, max, max, true)?
    };
    Ok(texture_from_pixbuf(&flatten(&pb)))
}

fn scale_down(full: &gdk_pixbuf::Pixbuf, max: i32) -> gdk_pixbuf::Pixbuf {
    let (w, h) = (full.width(), full.height());
    let scale = max as f64 / w.max(h).max(1) as f64;
    if scale >= 1.0 {
        return full.clone();
    }
    let tw = ((w as f64 * scale).round() as i32).max(1);
    let th = ((h as f64 * scale).round() as i32).max(1);
    full.scale_simple(tw, th, gdk_pixbuf::InterpType::Bilinear)
        .unwrap_or_else(|| full.clone())
}

/// Checker cell size in pixels, and its two greys (neutral in light and dark
/// themes, like an image editor's transparency grid).
const CHECK_SIZE: i32 = 8;
const CHECK_LIGHT: u32 = 0xe8e8e8;
const CHECK_DARK: u32 = 0xc4c4c4;

/// Composite transparent pixels over a checkerboard, so a dark logo is
/// visible on a dark card and transparent areas read as transparent.
/// Opaque images come back unchanged.
fn flatten(pb: &gdk_pixbuf::Pixbuf) -> gdk_pixbuf::Pixbuf {
    if !has_transparency(pb) {
        return pb.clone();
    }
    pb.composite_color_simple(
        pb.width(),
        pb.height(),
        gdk_pixbuf::InterpType::Nearest,
        255,
        CHECK_SIZE,
        CHECK_LIGHT,
        CHECK_DARK,
    )
    .unwrap_or_else(|| pb.clone())
}

fn has_transparency(pb: &gdk_pixbuf::Pixbuf) -> bool {
    if !pb.has_alpha() || pb.n_channels() != 4 {
        return false;
    }
    let (w, h) = (pb.width().max(0) as usize, pb.height().max(0) as usize);
    let stride = pb.rowstride().max(0) as usize;
    let bytes = pb.read_pixel_bytes();
    (0..h).any(|y| {
        bytes
            .get(y * stride..y * stride + w * 4)
            .is_some_and(|row| row.as_chunks::<4>().0.iter().any(|px| px[3] < 255))
    })
}

/// `gdk::Texture::for_pixbuf` is deprecated since GTK 4.20; build the texture
/// from the pixel rows directly.
fn texture_from_pixbuf(pb: &gdk_pixbuf::Pixbuf) -> gdk::Texture {
    let format = if pb.has_alpha() {
        gdk::MemoryFormat::R8g8b8a8
    } else {
        gdk::MemoryFormat::R8g8b8
    };
    let bytes = pb.read_pixel_bytes();
    gdk::MemoryTexture::new(
        pb.width(),
        pb.height(),
        format,
        &bytes,
        pb.rowstride() as usize,
    )
    .upcast()
}

/// A small PNG (for menu icons) rendered from the image file on disk.
pub fn menu_icon_png(path: &str, size: i32) -> Option<Vec<u8>> {
    let pb = gdk_pixbuf::Pixbuf::from_file_at_scale(path, size, size, true).ok()?;
    flatten(&pb).save_to_bufferv("png", &[]).ok()
}

/// A 1 × 1 texture of `rgba`, stretched into a colour swatch.
pub fn swatch(rgba: [f32; 4]) -> gdk::Texture {
    let px = rgba.map(|c| (c.clamp(0.0, 1.0) * 255.0).round() as u8);
    gdk::MemoryTexture::new(
        1,
        1,
        gdk::MemoryFormat::R8g8b8a8,
        &glib::Bytes::from(&px),
        4,
    )
    .upcast()
}

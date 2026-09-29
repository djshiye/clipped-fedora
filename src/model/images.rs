//! Image decoding helpers. Everything here is safe to call from a worker
//! thread: `gdk::Texture` is immutable and thread-safe.

use gtk::{gdk, gdk_pixbuf, gio, glib, prelude::*};

/// Longest thumbnail edge in logical pixels; rows show it at 48 px, so this
/// stays crisp up to 4× scaling.
pub const THUMB_SIZE: i32 = 192;

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
        thumbnail: thumbnail_of(&full),
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

pub fn thumbnail_from_file(path: &str) -> Result<gdk::Texture, glib::Error> {
    let pb = gdk_pixbuf::Pixbuf::from_file_at_scale(path, THUMB_SIZE, THUMB_SIZE, true)?;
    Ok(texture_from_pixbuf(&pb))
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

fn thumbnail_of(full: &gdk_pixbuf::Pixbuf) -> gdk::Texture {
    let (w, h) = (full.width(), full.height());
    let scale = THUMB_SIZE as f64 / w.max(h).max(1) as f64;
    if scale >= 1.0 {
        return texture_from_pixbuf(full);
    }
    let tw = ((w as f64 * scale).round() as i32).max(1);
    let th = ((h as f64 * scale).round() as i32).max(1);
    let small = full
        .scale_simple(tw, th, gdk_pixbuf::InterpType::Bilinear)
        .unwrap_or_else(|| full.clone());
    texture_from_pixbuf(&small)
}

/// A small PNG (for menu icons) rendered from the image file on disk.
pub fn menu_icon_png(path: &str, size: i32) -> Option<Vec<u8>> {
    let pb = gdk_pixbuf::Pixbuf::from_file_at_scale(path, size, size, true).ok()?;
    pb.save_to_bufferv("png", &[]).ok()
}

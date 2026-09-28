//! Image decoding helpers. Everything here is safe to call from a worker
//! thread: `gdk::Texture` is immutable and thread-safe.

use gtk::{gdk, gdk_pixbuf, gio, glib, prelude::*};

/// Longest thumbnail edge in logical pixels; rows show it at 48 px, so this
/// stays crisp up to 4× scaling.
pub const THUMB_SIZE: i32 = 192;

pub struct Decoded {
    pub width: i32,
    pub height: i32,
    pub thumbnail: gdk::Texture,
}

pub fn decode(bytes: &[u8]) -> Result<Decoded, glib::Error> {
    let stream = gio::MemoryInputStream::from_bytes(&glib::Bytes::from(bytes));
    let full = gdk_pixbuf::Pixbuf::from_stream(&stream, gio::Cancellable::NONE)?;
    Ok(Decoded {
        width: full.width(),
        height: full.height(),
        thumbnail: thumbnail_of(&full),
    })
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

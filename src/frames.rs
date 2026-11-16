//! Frame hand-off between CEF and GTK: the view geometry CEF needs, and dmabuf frames.

use cef::*;
use gtk::gdk;

/// Everything the render handler needs to know about the view. CEF may call it from its own UI
/// thread (`multi_threaded_message_loop`), so it lives behind a mutex, not in GTK-thread state.
#[derive(Clone, Copy)]
pub struct View {
    pub size: (i32, i32),
    pub scale: f32,
    /// Render at device resolution (see `k`).
    pub backing: bool,
}

impl View {
    /// Factor between logical size and the pixel size CEF renders at. With "backing scale" CEF is
    /// told the view is device-sized at 1x, so frames match the screen's physical pixels.
    pub fn k(&self) -> f32 {
        if self.backing && self.scale > 1.0 { self.scale } else { 1.0 }
    }

    /// Size CEF renders at: logical size times `k`.
    pub fn device_size(&self) -> (i32, i32) {
        let k = self.k();
        (((self.size.0 as f32 * k).round() as i32).max(1), ((self.size.1 as f32 * k).round() as i32).max(1))
    }
}


/// Plain-data description of a dmabuf frame that owns (dup'd) fds, so it can be sent from CEF's
/// UI thread to the GTK thread.
pub struct DmabufFrame {
    fourcc: u32,
    modifier: u64,
    width: u32,
    height: u32,
    /// (fd, offset, stride) per plane.
    planes: Vec<(i32, u32, u32)>,
}

impl DmabufFrame {
    /// CEF closes its fds when the paint callback returns, so take our own duplicates.
    pub fn from_info(info: &AcceleratedPaintInfo) -> Option<Self> {
        let fourcc = match info.format.as_ref() {
            sys::cef_color_type_t::CEF_COLOR_TYPE_BGRA_8888 => u32::from_le_bytes(*b"AR24"), // ARGB8888
            sys::cef_color_type_t::CEF_COLOR_TYPE_RGBA_8888 => u32::from_le_bytes(*b"AB24"), // ABGR8888
            _ => return None,
        };
        let n = (info.plane_count.max(0) as usize).min(info.planes.len());
        if n == 0 {
            return None;
        }
        let mut planes = Vec::with_capacity(n);
        for p in &info.planes[..n] {
            let fd = unsafe { libc::dup(p.fd) };
            if fd < 0 {
                planes.iter().for_each(|&(f, _, _)| unsafe { libc::close(f); });
                return None;
            }
            planes.push((fd, p.offset as u32, p.stride));
        }
        Some(Self {
            fourcc,
            modifier: info.modifier,
            width: info.extra.coded_size.width as u32,
            height: info.extra.coded_size.height as u32,
            planes,
        })
    }
}

/// GTK thread only. Consumes the frame; its fds are closed when GDK releases the texture.
pub fn import_frame(display: &gdk::Display, frame: DmabufFrame) -> Option<gdk::Texture> {
    let fds: Vec<i32> = frame.planes.iter().map(|p| p.0).collect();
    let mut b = gdk::DmabufTextureBuilder::new()
        .set_display(display)
        .set_fourcc(frame.fourcc)
        .set_modifier(frame.modifier)
        .set_width(frame.width)
        .set_height(frame.height)
        .set_n_planes(frame.planes.len() as u32);
    for (i, &(fd, offset, stride)) in frame.planes.iter().enumerate() {
        b = unsafe { b.set_fd(i as u32, fd) }.set_offset(i as u32, offset).set_stride(i as u32, stride);
    }
    let to_close = fds.clone();
    let tex = unsafe { b.build_with_release_func(move || to_close.iter().for_each(|&f| { libc::close(f); })) };
    match tex {
        Ok(t) => Some(t),
        Err(_) => {
            // Not taken over by GDK; the release function is not called on failure.
            fds.iter().for_each(|&f| unsafe { libc::close(f); });
            None
        }
    }
}

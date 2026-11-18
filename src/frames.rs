//! Frame hand-off between CEF and GTK: the view geometry CEF needs, and dmabuf frames.

use cef::*;
use gtk::gdk;

/// Everything the render handler needs to know about the view. CEF may call it from its own UI
/// thread (`multi_threaded_message_loop`), so it lives behind a mutex, not in GTK-thread state.
#[derive(Clone, Copy)]
pub struct View {
    pub size: (i32, i32),
    pub scale: f32,
    /// The desktop's text scaling factor (GNOME's "Large Text" is 1.25), an extra scale on top of `scale`: 1 when unset.
    /// It is applied like a display scale (see `dsf`), so text, layout and scrollbars all grow and stay sharp.
    pub text: f32,
    /// Render at device resolution (see `dsf`).
    pub backing: bool,
}

/// The range of text scaling factors honoured (GNOME offers 0.5 to 3).
const TEXT_RANGE: (f32, f32) = (0.5, 3.0);

/// A text scaling factor from a setting, or 1 when it is missing or not a sensible number.
pub fn sane_text_scale(v: f64) -> f32 {
    if v.is_finite() && v > 0.0 { (v as f32).clamp(TEXT_RANGE.0, TEXT_RANGE.1) } else { 1.0 }
}

/// The desktop's text scaling factor: `BROWSAH_FORCE_TEXT_SCALE` (to test), else GNOME's `text-scaling-factor` (what the
/// accessibility "Large Text" switch and the Tweaks slider set), else 1. Cheap to call often (dconf caches the value).
pub fn text_scale() -> f32 {
    use gtk::prelude::SettingsExt;
    thread_local! {
        static SETTINGS: Option<gtk::gio::Settings> = {
            let source = gtk::gio::SettingsSchemaSource::default();
            source.and_then(|s| s.lookup("org.gnome.desktop.interface", true)).filter(|schema| schema.has_key("text-scaling-factor")).map(|_| gtk::gio::Settings::new("org.gnome.desktop.interface"))
        };
    }
    if let Some(v) = std::env::var("BROWSAH_FORCE_TEXT_SCALE").ok().and_then(|s| s.parse::<f64>().ok()) {
        return sane_text_scale(v);
    }
    SETTINGS.with(|s| s.as_ref().map_or(1.0, |s| sane_text_scale(s.double("text-scaling-factor"))))
}

impl View {
    /// Factor between the window's logical pixels and the units CEF uses for input coordinates, popup rectangles and
    /// menu positions. CEF works in its own logical (DIP) pixels, which are the window's divided by the text scale, so
    /// this is `1 / text` (1 without text scaling: nothing needs converting).
    pub fn k(&self) -> f32 {
        1.0 / self.text
    }

    /// The view's size as CEF is told it (CEF's logical pixels: the window's divided by the text scale; the frames are
    /// this times `dsf`, which is the window's size times the display scale).
    pub fn device_size(&self) -> (i32, i32) {
        (((self.size.0 as f32 / self.text).round() as i32).max(1), ((self.size.1 as f32 / self.text).round() as i32).max(1))
    }

    /// The device scale factor CEF is told: the screen's times the text scale, when rendering at device resolution
    /// (sharp), else 1 (CEF renders at 1x and the picture is stretched, which looks blurry).
    pub fn dsf(&self) -> f32 {
        let total = self.scale * self.text;
        if self.backing && total > 1.0 { total } else { 1.0 }
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

#[cfg(test)]
mod tests {
    use super::*;

    fn view(scale: f32, text: f32, backing: bool) -> View {
        View { size: (1000, 600), scale, text, backing }
    }

    #[test]
    fn without_text_scaling_nothing_changes() {
        let v = view(2.0, 1.0, true);
        assert_eq!((v.k(), v.device_size(), v.dsf()), (1.0, (1000, 600), 2.0));
        let v = view(1.0, 1.0, true);
        assert_eq!((v.k(), v.device_size(), v.dsf()), (1.0, (1000, 600), 1.0));
    }

    #[test]
    fn text_scaling_is_an_extra_scale_factor() {
        // Large Text (1.25) on a 1x screen: CEF gets a smaller logical view and a 1.25 scale, so the page is 1.25 times
        // bigger on screen; the frame is still the window's size in device pixels (800 * 1.25 = 1000).
        let v = view(1.0, 1.25, true);
        assert_eq!(v.device_size(), (800, 480));
        assert_eq!(v.dsf(), 1.25);
        assert_eq!(v.k(), 0.8, "a window position times k is CEF's position");
        let (w, h) = v.device_size();
        assert_eq!(((w as f32 * v.dsf()).round() as i32, (h as f32 * v.dsf()).round() as i32), (1000, 600));
        // On a 2x screen the scales multiply: frame 2000 device pixels wide.
        let v = view(2.0, 1.5, true);
        // (the logical size is rounded to a whole pixel, so the frame can be a pixel or two off the window)
        assert!(((v.device_size().0 as f32 * v.dsf()).round() as i32 - 2000).abs() <= 2);
        assert_eq!(v.dsf(), 3.0);
    }

    #[test]
    fn the_logical_size_never_reaches_zero() {
        let v = View { size: (0, 1), scale: 1.0, text: 3.0, backing: true };
        assert_eq!(v.device_size(), (1, 1));
    }

    #[test]
    fn text_scales_are_made_sane() {
        assert_eq!(sane_text_scale(1.25), 1.25);
        assert_eq!(sane_text_scale(0.0), 1.0);
        assert_eq!(sane_text_scale(-2.0), 1.0);
        assert_eq!(sane_text_scale(f64::NAN), 1.0);
        assert_eq!(sane_text_scale(10.0), 3.0);
        assert_eq!(sane_text_scale(0.1), 0.5);
    }
}

use base64::Engine;
use image::codecs::jpeg::JpegEncoder;
use image::imageops::FilterType;
#[cfg(not(target_os = "windows"))]
use image::DynamicImage;
use image::{ExtendedColorType, RgbImage};
use std::sync::{Mutex, OnceLock};
use std::time::Instant;

#[cfg(target_os = "windows")]
use std::sync::Once;

#[cfg(target_os = "windows")]
static DPI_INIT: Once = Once::new();

static LAST_SCREEN_TRANSFORM: OnceLock<Mutex<Option<ScreenTransform>>> = OnceLock::new();

/// Virtual screen bounding rectangle in physical display pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VirtualScreenRect {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

/// Optional capture region in physical virtual-screen pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CaptureRegion {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScreenFormat {
    Png,
    Jpeg,
}

/// Options controlling screenshot capture, crop, resize, and encoding.
#[derive(Debug, Clone)]
pub struct ScreenCaptureOptions {
    pub format: ScreenFormat,
    pub quality: u8,
    pub max_width: Option<u32>,
    pub max_height: Option<u32>,
    pub scale: Option<f64>,
    pub region: Option<CaptureRegion>,
}

impl Default for ScreenCaptureOptions {
    fn default() -> Self {
        Self {
            format: ScreenFormat::Png,
            quality: 80,
            max_width: None,
            max_height: None,
            scale: None,
            region: None,
        }
    }
}

impl ScreenCaptureOptions {
    pub fn from_json(params: &serde_json::Value) -> anyhow::Result<Self> {
        let mut opts = Self::default();

        if let Some(preset) = params.get("preset").and_then(|v| v.as_str()) {
            match preset.trim().to_ascii_lowercase().as_str() {
                "fast" | "cua" | "llm" => {
                    opts.format = ScreenFormat::Jpeg;
                    opts.quality = 80;
                    opts.max_width = Some(1280);
                }
                "hd" => {
                    opts.format = ScreenFormat::Jpeg;
                    opts.quality = 85;
                    opts.max_width = Some(1920);
                }
                "lossless" | "full" | "raw" => {
                    opts.format = ScreenFormat::Png;
                    opts.max_width = None;
                    opts.max_height = None;
                    opts.scale = None;
                }
                other => {
                    return Err(anyhow::anyhow!("Unknown screen preset '{}'", other));
                }
            }
        }

        if let Some(fmt) = params.get("format").and_then(|v| v.as_str()) {
            opts.format = match fmt.trim().to_ascii_lowercase().as_str() {
                "png" => ScreenFormat::Png,
                "jpeg" | "jpg" => ScreenFormat::Jpeg,
                other => return Err(anyhow::anyhow!("Unsupported screenshot format '{}'", other)),
            };
        }

        if let Some(q) = params.get("quality").and_then(|v| v.as_u64()) {
            opts.quality = (q as u8).clamp(1, 100);
        }
        if let Some(mw) = params.get("max_width").and_then(|v| v.as_u64()) {
            if mw > 0 {
                opts.max_width = Some(mw as u32);
            }
        }
        if let Some(mh) = params.get("max_height").and_then(|v| v.as_u64()) {
            if mh > 0 {
                opts.max_height = Some(mh as u32);
            }
        }
        if let Some(sc) = params.get("scale").and_then(|v| v.as_f64()) {
            if sc > 0.0 && sc <= 1.0 {
                opts.scale = Some(sc);
            }
        }

        if let Some(reg) = params.get("region").and_then(|v| v.as_object()) {
            let x = reg.get("x").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
            let y = reg.get("y").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
            let w = reg
                .get("width")
                .or_else(|| reg.get("w"))
                .and_then(|v| v.as_u64())
                .unwrap_or(0) as u32;
            let h = reg
                .get("height")
                .or_else(|| reg.get("h"))
                .and_then(|v| v.as_u64())
                .unwrap_or(0) as u32;
            if w == 0 || h == 0 {
                return Err(anyhow::anyhow!("Invalid screen region dimensions {}x{}", w, h));
            }
            opts.region = Some(CaptureRegion {
                x,
                y,
                width: w,
                height: h,
            });
        }

        Ok(opts)
    }

    pub fn format_str(&self) -> &'static str {
        match self.format {
            ScreenFormat::Png => "png",
            ScreenFormat::Jpeg => "jpeg",
        }
    }

    #[cfg(not(target_os = "windows"))]
    pub fn needs_transform(&self) -> bool {
        self.format == ScreenFormat::Jpeg
            || self.max_width.is_some()
            || self.max_height.is_some()
            || self.scale.is_some()
            || self.region.is_some()
    }
}

/// Describes the mapping from captured/resized screenshot pixels back to physical desktop pixels.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ScreenTransform {
    pub origin_x: i32,
    pub origin_y: i32,
    pub original_width: u32,
    pub original_height: u32,
    pub width: u32,
    pub height: u32,
    pub scale_x: f64,
    pub scale_y: f64,
}

impl ScreenTransform {
    pub fn from_dims(
        origin_x: i32,
        origin_y: i32,
        original_width: u32,
        original_height: u32,
        width: u32,
        height: u32,
    ) -> Self {
        let scale_x = if width > 0 {
            original_width as f64 / width as f64
        } else {
            1.0
        };
        let scale_y = if height > 0 {
            original_height as f64 / height as f64
        } else {
            1.0
        };
        Self {
            origin_x,
            origin_y,
            original_width,
            original_height,
            width,
            height,
            scale_x,
            scale_y,
        }
    }
}

pub fn get_last_screen_transform() -> Option<ScreenTransform> {
    LAST_SCREEN_TRANSFORM
        .get_or_init(|| Mutex::new(None))
        .lock()
        .ok()
        .and_then(|g| *g)
}

pub fn set_last_screen_transform(t: ScreenTransform) {
    if let Ok(mut guard) = LAST_SCREEN_TRANSFORM.get_or_init(|| Mutex::new(None)).lock() {
        *guard = Some(t);
    }
}

/// Maps coordinates from `physical`, `last_screen` (or explicit source dimensions),
/// `normalized` (`0..1000`), or `ratio` (`0.0..1.0`) to physical virtual-screen pixels.
pub fn map_cua_coords(
    x: f64,
    y: f64,
    coord_space: &str,
    params: &serde_json::Value,
    virtual_bounds: Option<VirtualScreenRect>,
    last_screen: Option<ScreenTransform>,
) -> anyhow::Result<(f64, f64)> {
    match coord_space.trim().to_ascii_lowercase().as_str() {
        "" | "physical" | "screen_px" => Ok((x, y)),
        "last_screen" | "screen" | "image" => {
            if let (Some(sw), Some(sh)) = (
                params.get("source_width").and_then(|v| v.as_f64()),
                params.get("source_height").and_then(|v| v.as_f64()),
            ) {
                let bounds = virtual_bounds
                    .or_else(|| {
                        last_screen.map(|t| VirtualScreenRect {
                            x: t.origin_x,
                            y: t.origin_y,
                            width: t.original_width as i32,
                            height: t.original_height as i32,
                        })
                    })
                    .ok_or_else(|| anyhow::anyhow!("Cannot resolve target screen bounds for coord_space='image'"))?;
                let sx = bounds.width as f64 / sw.max(1.0);
                let sy = bounds.height as f64 / sh.max(1.0);
                return Ok((bounds.x as f64 + x * sx, bounds.y as f64 + y * sy));
            }
            let t = last_screen.ok_or_else(|| {
                anyhow::anyhow!("coord_space='last_screen' requested before any screen capture")
            })?;
            Ok((
                t.origin_x as f64 + x * t.scale_x,
                t.origin_y as f64 + y * t.scale_y,
            ))
        }
        "normalized" | "norm" | "norm_1000" => {
            let norm_max = params
                .get("norm_max")
                .and_then(|v| v.as_f64())
                .filter(|v| *v > 0.0)
                .unwrap_or(1000.0);
            let rect = resolve_target_rect(virtual_bounds, last_screen)?;
            let px = rect.x as f64 + (x / norm_max) * (rect.width.max(1) as f64);
            let py = rect.y as f64 + (y / norm_max) * (rect.height.max(1) as f64);
            Ok((px, py))
        }
        "ratio" | "unit" => {
            let rect = resolve_target_rect(virtual_bounds, last_screen)?;
            let px = rect.x as f64 + x * (rect.width.max(1) as f64);
            let py = rect.y as f64 + y * (rect.height.max(1) as f64);
            Ok((px, py))
        }
        other => Err(anyhow::anyhow!("Unsupported coord_space '{}'", other)),
    }
}

fn resolve_target_rect(
    virtual_bounds: Option<VirtualScreenRect>,
    last_screen: Option<ScreenTransform>,
) -> anyhow::Result<VirtualScreenRect> {
    if let Some(t) = last_screen {
        return Ok(VirtualScreenRect {
            x: t.origin_x,
            y: t.origin_y,
            width: t.original_width as i32,
            height: t.original_height as i32,
        });
    }
    virtual_bounds.ok_or_else(|| anyhow::anyhow!("Target display bounds unavailable"))
}

/// Full encoded screenshot payload + CUA coordinate metadata.
pub struct ScreenCaptureOutput {
    pub base64: String,
    pub format: &'static str,
    pub width: u32,
    pub height: u32,
    pub original_width: u32,
    pub original_height: u32,
    pub origin_x: i32,
    pub origin_y: i32,
    pub scale_x: f64,
    pub scale_y: f64,
    pub bytes: usize,
    pub capture_ms: u64,
    pub method: &'static str,
}

impl ScreenCaptureOutput {
    pub fn to_json(&self) -> serde_json::Value {
        serde_json::json!({
            "base64": &self.base64,
            "image_base64": &self.base64,
            "format": self.format,
            "width": self.width,
            "height": self.height,
            "original_width": self.original_width,
            "original_height": self.original_height,
            "origin_x": self.origin_x,
            "origin_y": self.origin_y,
            "scale_x": self.scale_x,
            "scale_y": self.scale_y,
            "bytes": self.bytes,
            "capture_ms": self.capture_ms,
            "method": self.method,
        })
    }
}

fn compute_target_dimensions(src_w: u32, src_h: u32, opts: &ScreenCaptureOptions) -> (u32, u32) {
    if src_w == 0 || src_h == 0 {
        return (src_w, src_h);
    }

    let mut ratio = opts.scale.unwrap_or(1.0);
    if let Some(mw) = opts.max_width {
        if src_w > mw {
            ratio = ratio.min(mw as f64 / src_w as f64);
        }
    }
    if let Some(mh) = opts.max_height {
        if src_h > mh {
            ratio = ratio.min(mh as f64 / src_h as f64);
        }
    }

    if ratio >= 0.9999 {
        return (src_w, src_h);
    }

    let dst_w = ((src_w as f64) * ratio).round().max(1.0) as u32;
    let dst_h = ((src_h as f64) * ratio).round().max(1.0) as u32;
    (dst_w, dst_h)
}

fn encode_rgb_image(
    rgb: RgbImage,
    origin_x: i32,
    origin_y: i32,
    opts: &ScreenCaptureOptions,
    method: &'static str,
    started: Instant,
) -> anyhow::Result<ScreenCaptureOutput> {
    let orig_w = rgb.width();
    let orig_h = rgb.height();
    let (dst_w, dst_h) = compute_target_dimensions(orig_w, orig_h, opts);

    let final_rgb = if (dst_w, dst_h) != (orig_w, orig_h) {
        image::imageops::resize(&rgb, dst_w, dst_h, FilterType::Triangle)
    } else {
        rgb
    };

    let mut encoded = Vec::with_capacity((dst_w as usize * dst_h as usize) / 4);
    match opts.format {
        ScreenFormat::Png => {
            let mut encoder = png::Encoder::new(&mut encoded, dst_w, dst_h);
            encoder.set_color(png::ColorType::Rgb);
            encoder.set_depth(png::BitDepth::Eight);
            encoder.set_compression(png::Compression::Fast);
            let mut writer = encoder.write_header()?;
            writer.write_image_data(final_rgb.as_raw())?;
        }
        ScreenFormat::Jpeg => {
            let mut encoder = JpegEncoder::new_with_quality(&mut encoded, opts.quality);
            encoder.encode(final_rgb.as_raw(), dst_w, dst_h, ExtendedColorType::Rgb8)?;
        }
    }

    let transform = ScreenTransform::from_dims(origin_x, origin_y, orig_w, orig_h, dst_w, dst_h);
    set_last_screen_transform(transform);

    let b64 = base64::engine::general_purpose::STANDARD.encode(&encoded);
    Ok(ScreenCaptureOutput {
        base64: b64,
        format: opts.format_str(),
        width: dst_w,
        height: dst_h,
        original_width: orig_w,
        original_height: orig_h,
        origin_x,
        origin_y,
        scale_x: transform.scale_x,
        scale_y: transform.scale_y,
        bytes: encoded.len(),
        capture_ms: started.elapsed().as_millis() as u64,
        method,
    })
}

/// Ensures Per-Monitor DPI Awareness v2 is enabled once so `BitBlt`, `GetSystemMetrics`,
/// and `SendInput` all share the exact same physical pixel coordinate space.
#[cfg(target_os = "windows")]
pub fn ensure_dpi_awareness() {
    DPI_INIT.call_once(|| unsafe {
        use windows::Win32::UI::HiDpi::{
            SetProcessDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
        };
        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
    });
}

/// Returns the bounding rectangle of the Windows virtual desktop across all monitors.
#[cfg(target_os = "windows")]
pub fn virtual_screen_bounds() -> anyhow::Result<VirtualScreenRect> {
    use windows::Win32::UI::WindowsAndMessaging::{
        GetSystemMetrics, SM_CXVIRTUALSCREEN, SM_CYVIRTUALSCREEN, SM_XVIRTUALSCREEN,
        SM_YVIRTUALSCREEN,
    };

    ensure_dpi_awareness();

    let (x, y, width, height) = unsafe {
        (
            GetSystemMetrics(SM_XVIRTUALSCREEN),
            GetSystemMetrics(SM_YVIRTUALSCREEN),
            GetSystemMetrics(SM_CXVIRTUALSCREEN),
            GetSystemMetrics(SM_CYVIRTUALSCREEN),
        )
    };

    if width <= 0 || height <= 0 {
        return Err(anyhow::anyhow!(
            "Invalid virtual screen dimensions: {}x{}",
            width,
            height
        ));
    }

    Ok(VirtualScreenRect {
        x,
        y,
        width,
        height,
    })
}

pub fn current_virtual_bounds() -> Option<VirtualScreenRect> {
    #[cfg(target_os = "windows")]
    {
        virtual_screen_bounds().ok()
    }
    #[cfg(not(target_os = "windows"))]
    {
        get_last_screen_transform().map(|t| VirtualScreenRect {
            x: t.origin_x,
            y: t.origin_y,
            width: t.original_width as i32,
            height: t.original_height as i32,
        })
    }
}

#[cfg(target_os = "windows")]
struct ScreenDcGuard(windows::Win32::Graphics::Gdi::HDC);

#[cfg(target_os = "windows")]
impl Drop for ScreenDcGuard {
    fn drop(&mut self) {
        unsafe {
            let _ = windows::Win32::Graphics::Gdi::ReleaseDC(None, self.0);
        }
    }
}

#[cfg(target_os = "windows")]
struct MemDcGuard(windows::Win32::Graphics::Gdi::HDC);

#[cfg(target_os = "windows")]
impl Drop for MemDcGuard {
    fn drop(&mut self) {
        unsafe {
            let _ = windows::Win32::Graphics::Gdi::DeleteDC(self.0);
        }
    }
}

#[cfg(target_os = "windows")]
struct BitmapGuard(windows::Win32::Graphics::Gdi::HBITMAP);

#[cfg(target_os = "windows")]
impl Drop for BitmapGuard {
    fn drop(&mut self) {
        unsafe {
            let _ = windows::Win32::Graphics::Gdi::DeleteObject(self.0.into());
        }
    }
}

#[cfg(target_os = "windows")]
struct SelectObjectGuard {
    hdc: windows::Win32::Graphics::Gdi::HDC,
    prev: windows::Win32::Graphics::Gdi::HGDIOBJ,
}

#[cfg(target_os = "windows")]
impl Drop for SelectObjectGuard {
    fn drop(&mut self) {
        unsafe {
            let _ = windows::Win32::Graphics::Gdi::SelectObject(self.hdc, self.prev);
        }
    }
}

/// Captures the Windows virtual screen (or requested region) directly via Win32 GDI `BitBlt`
/// and encodes it according to `opts`.
#[cfg(target_os = "windows")]
pub fn capture_screen_sync(opts: &ScreenCaptureOptions) -> anyhow::Result<ScreenCaptureOutput> {
    use windows::Win32::Graphics::Gdi::{
        BitBlt, CreateCompatibleDC, CreateDIBSection, GdiFlush, GetDC, SelectObject, BITMAPINFO,
        BITMAPINFOHEADER, BI_RGB, CAPTUREBLT, DIB_RGB_COLORS, RGBQUAD, SRCCOPY,
    };

    let started = Instant::now();
    let vbounds = virtual_screen_bounds()?;

    let (cap_x, cap_y, width, height) = if let Some(reg) = opts.region {
        let x1 = reg.x.max(vbounds.x);
        let y1 = reg.y.max(vbounds.y);
        let x2 = (reg.x as i64 + reg.width as i64).min(vbounds.x as i64 + vbounds.width as i64);
        let y2 = (reg.y as i64 + reg.height as i64).min(vbounds.y as i64 + vbounds.height as i64);
        let w = (x2 - x1 as i64) as i32;
        let h = (y2 - y1 as i64) as i32;
        if w <= 0 || h <= 0 {
            return Err(anyhow::anyhow!(
                "Requested capture region is outside virtual screen bounds"
            ));
        }
        (x1, y1, w, h)
    } else {
        (vbounds.x, vbounds.y, vbounds.width, vbounds.height)
    };

    let pixel_count = (width as usize)
        .checked_mul(height as usize)
        .ok_or_else(|| anyhow::anyhow!("Screen pixel count overflow"))?;
    let byte_len = pixel_count
        .checked_mul(4)
        .ok_or_else(|| anyhow::anyhow!("Screen buffer size overflow"))?;

    let rgb_vec = unsafe {
        let hdc_screen = GetDC(None);
        if hdc_screen.is_invalid() {
            return Err(anyhow::anyhow!("Win32 GetDC(None) failed"));
        }
        let screen_dc = ScreenDcGuard(hdc_screen);

        let hdc_mem = CreateCompatibleDC(Some(screen_dc.0));
        if hdc_mem.is_invalid() {
            return Err(anyhow::anyhow!("Win32 CreateCompatibleDC failed"));
        }
        let mem_dc = MemDcGuard(hdc_mem);

        let bmi = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: width,
                biHeight: -height,
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                biSizeImage: 0,
                biXPelsPerMeter: 0,
                biYPelsPerMeter: 0,
                biClrUsed: 0,
                biClrImportant: 0,
            },
            bmiColors: [RGBQUAD::default(); 1],
        };

        let mut bits_ptr: *mut core::ffi::c_void = std::ptr::null_mut();
        let hbitmap = CreateDIBSection(
            Some(screen_dc.0),
            &bmi,
            DIB_RGB_COLORS,
            &mut bits_ptr,
            None,
            0,
        )?;
        if hbitmap.is_invalid() || bits_ptr.is_null() {
            return Err(anyhow::anyhow!("Win32 CreateDIBSection returned invalid bitmap"));
        }
        let bitmap = BitmapGuard(hbitmap);

        let prev_obj = SelectObject(mem_dc.0, bitmap.0.into());
        if prev_obj.is_invalid() {
            return Err(anyhow::anyhow!("Win32 SelectObject failed"));
        }
        let select_guard = SelectObjectGuard {
            hdc: mem_dc.0,
            prev: prev_obj,
        };

        if BitBlt(
            mem_dc.0,
            0,
            0,
            width,
            height,
            Some(screen_dc.0),
            cap_x,
            cap_y,
            SRCCOPY | CAPTUREBLT,
        )
        .is_err()
        {
            BitBlt(
                mem_dc.0,
                0,
                0,
                width,
                height,
                Some(screen_dc.0),
                cap_x,
                cap_y,
                SRCCOPY,
            )?;
        }

        let _ = GdiFlush();

        let bgra = std::slice::from_raw_parts(bits_ptr as *const u8, byte_len);
        let mut rgb = vec![0u8; pixel_count * 3];
        for (src, dst) in bgra.chunks_exact(4).zip(rgb.chunks_exact_mut(3)) {
            dst[0] = src[2];
            dst[1] = src[1];
            dst[2] = src[0];
        }

        drop(select_guard);
        drop(bitmap);
        drop(mem_dc);
        drop(screen_dc);

        rgb
    };

    let rgb_img = RgbImage::from_raw(width as u32, height as u32, rgb_vec)
        .ok_or_else(|| anyhow::anyhow!("Failed to build RGB image buffer"))?;

    encode_rgb_image(rgb_img, cap_x, cap_y, opts, "win32_bitblt", started)
}

#[cfg(not(target_os = "windows"))]
fn parse_png_ihdr_dims(png_bytes: &[u8]) -> Option<(u32, u32)> {
    const PNG_SIG: &[u8; 8] = b"\x89PNG\r\n\x1a\n";
    if png_bytes.len() < 24 || &png_bytes[0..8] != PNG_SIG || &png_bytes[12..16] != b"IHDR" {
        return None;
    }
    let w = u32::from_be_bytes([png_bytes[16], png_bytes[17], png_bytes[18], png_bytes[19]]);
    let h = u32::from_be_bytes([png_bytes[20], png_bytes[21], png_bytes[22], png_bytes[23]]);
    Some((w, h))
}

#[cfg(not(target_os = "windows"))]
pub fn capture_screen_sync(opts: &ScreenCaptureOptions) -> anyhow::Result<ScreenCaptureOutput> {
    let started = Instant::now();
    let mut cmd = std::process::Command::new("grim");
    if std::env::var("WAYLAND_DISPLAY").is_err() {
        let wayland_display = if std::path::Path::new("/run/user/1000/wayland-1").exists() {
            "wayland-1"
        } else if std::path::Path::new("/run/user/1000/wayland-0").exists() {
            "wayland-0"
        } else {
            "wayland-1"
        };
        cmd.env("WAYLAND_DISPLAY", wayland_display);
    }
    if std::env::var("XDG_RUNTIME_DIR").is_err() {
        let uid = dirs::data_dir()
            .and_then(|p| p.to_str().map(|s| s.to_string()))
            .unwrap_or_default();
        let uid_str = if uid.contains("1000") { "1000" } else { "1000" };
        cmd.env("XDG_RUNTIME_DIR", format!("/run/user/{}", uid_str));
    }

    let output = cmd.arg("-").output()?;
    if !output.status.success() {
        let err_msg = String::from_utf8_lossy(&output.stderr);
        return Err(anyhow::anyhow!("grim screenshot failed: {}", err_msg.trim()));
    }

    let raw_png = output.stdout;

    // Fast path: raw PNG without crop/resize/re-encode.
    if !opts.needs_transform() {
        let (w, h) = parse_png_ihdr_dims(&raw_png).unwrap_or((0, 0));
        let transform = ScreenTransform::from_dims(0, 0, w, h, w, h);
        if w > 0 && h > 0 {
            set_last_screen_transform(transform);
        }
        let b64 = base64::engine::general_purpose::STANDARD.encode(&raw_png);
        return Ok(ScreenCaptureOutput {
            base64: b64,
            format: "png",
            width: w,
            height: h,
            original_width: w,
            original_height: h,
            origin_x: 0,
            origin_y: 0,
            scale_x: 1.0,
            scale_y: 1.0,
            bytes: raw_png.len(),
            capture_ms: started.elapsed().as_millis() as u64,
            method: "grim",
        });
    }

    let img: DynamicImage = image::load_from_memory(&raw_png)?;
    let (cropped, origin_x, origin_y) = if let Some(reg) = opts.region {
        let full_w = img.width();
        let full_h = img.height();
        let x = reg.x.max(0) as u32;
        let y = reg.y.max(0) as u32;
        if x >= full_w || y >= full_h {
            return Err(anyhow::anyhow!(
                "Requested capture region ({}, {}) is outside screen {}x{}",
                reg.x,
                reg.y,
                full_w,
                full_h
            ));
        }
        let cw = reg.width.min(full_w - x);
        let ch = reg.height.min(full_h - y);
        (img.crop_imm(x, y, cw, ch), x as i32, y as i32)
    } else {
        (img, 0, 0)
    };

    encode_rgb_image(cropped.to_rgb8(), origin_x, origin_y, opts, "grim", started)
}

pub async fn capture_screen(params: &serde_json::Value) -> anyhow::Result<ScreenCaptureOutput> {
    let opts = ScreenCaptureOptions::from_json(params)?;
    tokio::task::spawn_blocking(move || capture_screen_sync(&opts))
        .await
        .map_err(|e| anyhow::anyhow!("Screen capture task failed: {}", e))?
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parses_presets_and_overrides() {
        let opts = ScreenCaptureOptions::from_json(&json!({ "preset": "fast" })).unwrap();
        assert_eq!(opts.format, ScreenFormat::Jpeg);
        assert_eq!(opts.quality, 80);
        assert_eq!(opts.max_width, Some(1280));

        let custom = ScreenCaptureOptions::from_json(&json!({
            "preset": "fast",
            "format": "png",
            "max_width": 960,
        }))
        .unwrap();
        assert_eq!(custom.format, ScreenFormat::Png);
        assert_eq!(custom.max_width, Some(960));
    }

    #[test]
    fn maps_cua_coordinates_across_spaces() {
        let t = ScreenTransform::from_dims(-1920, 0, 2560, 1440, 1280, 720);
        let vbounds = VirtualScreenRect {
            x: -1920,
            y: 0,
            width: 2560,
            height: 1440,
        };

        let (px, py) = map_cua_coords(
            640.0,
            360.0,
            "last_screen",
            &json!({}),
            Some(vbounds),
            Some(t),
        )
        .unwrap();
        assert!((px - (-640.0)).abs() < 1e-6);
        assert!((py - 720.0).abs() < 1e-6);

        let (nx, ny) = map_cua_coords(
            500.0,
            250.0,
            "normalized",
            &json!({}),
            Some(vbounds),
            Some(t),
        )
        .unwrap();
        assert!((nx - (-640.0)).abs() < 1e-6);
        assert!((ny - 360.0).abs() < 1e-6);

        let (rx, ry) =
            map_cua_coords(0.5, 0.5, "ratio", &json!({}), Some(vbounds), Some(t)).unwrap();
        assert!((rx - (-640.0)).abs() < 1e-6);
        assert!((ry - 720.0).abs() < 1e-6);
    }

    #[test]
    fn target_dimensions_preserve_aspect_and_never_upscale() {
        let opts = ScreenCaptureOptions {
            max_width: Some(1280),
            ..Default::default()
        };
        assert_eq!(compute_target_dimensions(2560, 1440, &opts), (1280, 720));
        assert_eq!(compute_target_dimensions(800, 600, &opts), (800, 600));
    }
}

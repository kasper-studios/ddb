#[cfg(target_os = "linux")]
use evdev::uinput::VirtualDeviceBuilder;
#[cfg(target_os = "linux")]
use evdev::{AttributeSet, BusType, EventType, InputEvent, InputId, Key, RelativeAxisType};
use std::sync::Mutex;
use std::thread::sleep;
use std::time::Duration;

#[allow(unused_variables)]
pub struct DiromPointer {
    #[cfg(target_os = "linux")]
    device: Mutex<evdev::uinput::VirtualDevice>,
    current_x: Mutex<f64>,
    current_y: Mutex<f64>,
}

/// Converts a physical virtual-screen pixel coordinate along one axis into Win32
/// `SendInput` normalized `[0, 65535]` coordinates for `MOUSEEVENTF_VIRTUALDESK`.
#[cfg(any(target_os = "windows", test))]
fn to_virtual_desktop_coord(coord: i32, origin: i32, size: i32) -> i32 {
    if size <= 1 {
        return 0;
    }
    let rel = (coord as i64 - origin as i64).clamp(0, (size - 1) as i64);
    (((rel * 65536) + 32768) / size as i64).clamp(0, 65535) as i32
}

#[cfg(target_os = "windows")]
fn send_mouse_input(
    dx: i32,
    dy: i32,
    mouse_data: u32,
    flags: windows::Win32::UI::Input::KeyboardAndMouse::MOUSE_EVENT_FLAGS,
) -> anyhow::Result<()> {
    use windows::Win32::UI::Input::KeyboardAndMouse::{
        SendInput, INPUT, INPUT_0, INPUT_MOUSE, MOUSEINPUT,
    };

    let input = INPUT {
        r#type: INPUT_MOUSE,
        Anonymous: INPUT_0 {
            mi: MOUSEINPUT {
                dx,
                dy,
                mouseData: mouse_data,
                dwFlags: flags,
                time: 0,
                dwExtraInfo: 0,
            },
        },
    };

    let sent = unsafe { SendInput(&[input], std::mem::size_of::<INPUT>() as i32) };
    if sent != 1 {
        return Err(anyhow::anyhow!(
            "Win32 SendInput mouse event failed (sent={}, error={})",
            sent,
            windows::core::Error::from_thread()
        ));
    }
    Ok(())
}

#[cfg(target_os = "windows")]
fn win_button_flags(
    button: &str,
) -> (
    windows::Win32::UI::Input::KeyboardAndMouse::MOUSE_EVENT_FLAGS,
    windows::Win32::UI::Input::KeyboardAndMouse::MOUSE_EVENT_FLAGS,
) {
    use windows::Win32::UI::Input::KeyboardAndMouse::{
        MOUSEEVENTF_LEFTDOWN, MOUSEEVENTF_LEFTUP, MOUSEEVENTF_MIDDLEDOWN, MOUSEEVENTF_MIDDLEUP,
        MOUSEEVENTF_RIGHTDOWN, MOUSEEVENTF_RIGHTUP,
    };

    match button.trim().to_ascii_lowercase().as_str() {
        "right" => (MOUSEEVENTF_RIGHTDOWN, MOUSEEVENTF_RIGHTUP),
        "middle" => (MOUSEEVENTF_MIDDLEDOWN, MOUSEEVENTF_MIDDLEUP),
        _ => (MOUSEEVENTF_LEFTDOWN, MOUSEEVENTF_LEFTUP),
    }
}

/// Moves the Windows cursor directly to virtual-screen pixel `(x, y)` via Win32 `SendInput`.
#[cfg(target_os = "windows")]
pub fn windows_move_to(x: i32, y: i32) -> anyhow::Result<()> {
    use windows::Win32::UI::Input::KeyboardAndMouse::{
        MOUSEEVENTF_ABSOLUTE, MOUSEEVENTF_MOVE, MOUSEEVENTF_VIRTUALDESK,
    };

    let bounds = crate::screen::virtual_screen_bounds()?;
    let dx = to_virtual_desktop_coord(x, bounds.x, bounds.width);
    let dy = to_virtual_desktop_coord(y, bounds.y, bounds.height);
    send_mouse_input(
        dx,
        dy,
        0,
        MOUSEEVENTF_MOVE | MOUSEEVENTF_ABSOLUTE | MOUSEEVENTF_VIRTUALDESK,
    )
}

/// Presses or releases a mouse button, optionally moving to `(x, y)` first.
#[cfg(target_os = "windows")]
pub fn windows_mouse_button(
    x: Option<i32>,
    y: Option<i32>,
    button: &str,
    down: bool,
) -> anyhow::Result<()> {
    if let (Some(px), Some(py)) = (x, y) {
        windows_move_to(px, py)?;
    }
    let (down_flag, up_flag) = win_button_flags(button);
    send_mouse_input(0, 0, 0, if down { down_flag } else { up_flag })
}

/// Clicks `button` (`clicks` times) at the current cursor position via `SendInput`.
#[cfg(target_os = "windows")]
pub fn windows_click_current(button: &str, clicks: u8) -> anyhow::Result<()> {
    let (down_flag, up_flag) = win_button_flags(button);
    let count = clicks.max(1);

    for i in 0..count {
        send_mouse_input(0, 0, 0, down_flag)?;
        sleep(Duration::from_millis(25));
        send_mouse_input(0, 0, 0, up_flag)?;
        if i + 1 < count {
            sleep(Duration::from_millis(45));
        }
    }
    Ok(())
}

/// Moves to `(x, y)` and clicks `button` (`clicks` times) directly via Win32 `SendInput`.
#[cfg(target_os = "windows")]
pub fn windows_click(x: i32, y: i32, button: &str, clicks: u8) -> anyhow::Result<()> {
    windows_move_to(x, y)?;
    windows_click_current(button, clicks)
}

/// Performs a smooth drag from `(from_x, from_y)` to `(to_x, to_y)` while holding `button`.
#[cfg(target_os = "windows")]
pub fn windows_drag(
    from_x: i32,
    from_y: i32,
    to_x: i32,
    to_y: i32,
    button: &str,
    duration_ms: u64,
) -> anyhow::Result<()> {
    windows_move_to(from_x, from_y)?;
    sleep(Duration::from_millis(20));
    windows_mouse_button(None, None, button, true)?;
    sleep(Duration::from_millis(30));

    let steps = (duration_ms / 15).clamp(8, 40) as usize;
    let step_delay = Duration::from_millis((duration_ms / steps as u64).max(5));

    for i in 1..=steps {
        let t = i as f64 / steps as f64;
        let ease = t * t * (3.0 - 2.0 * t);
        let cur_x = (from_x as f64 + (to_x - from_x) as f64 * ease).round() as i32;
        let cur_y = (from_y as f64 + (to_y - from_y) as f64 * ease).round() as i32;
        windows_move_to(cur_x, cur_y)?;
        sleep(step_delay);
    }

    sleep(Duration::from_millis(20));
    windows_mouse_button(None, None, button, false)?;
    Ok(())
}

/// Scrolls vertically (`dy` wheel ticks: positive = up, negative = down) and horizontally
/// (`dx` wheel ticks: positive = right, negative = left), optionally moving to `(x, y)` first.
#[cfg(target_os = "windows")]
pub fn windows_scroll(x: Option<i32>, y: Option<i32>, dx: i32, dy: i32) -> anyhow::Result<()> {
    use windows::Win32::UI::Input::KeyboardAndMouse::{MOUSEEVENTF_HWHEEL, MOUSEEVENTF_WHEEL};
    use windows::Win32::UI::WindowsAndMessaging::WHEEL_DELTA;

    if let (Some(px), Some(py)) = (x, y) {
        windows_move_to(px, py)?;
    }

    let delta = WHEEL_DELTA as i32;
    if dy != 0 {
        let amount = dy.saturating_mul(delta) as u32;
        send_mouse_input(0, 0, amount, MOUSEEVENTF_WHEEL)?;
    }
    if dx != 0 {
        let amount = dx.saturating_mul(delta) as u32;
        send_mouse_input(0, 0, amount, MOUSEEVENTF_HWHEEL)?;
    }
    Ok(())
}

#[allow(dead_code)]
impl DiromPointer {
    #[cfg(target_os = "linux")]
    pub fn new() -> anyhow::Result<Self> {
        let mut keys = AttributeSet::<Key>::new();
        keys.insert(Key::BTN_LEFT);
        keys.insert(Key::BTN_RIGHT);
        keys.insert(Key::BTN_MIDDLE);

        let mut rels = AttributeSet::<RelativeAxisType>::new();
        rels.insert(RelativeAxisType::REL_X);
        rels.insert(RelativeAxisType::REL_Y);
        rels.insert(RelativeAxisType::REL_WHEEL);
        rels.insert(RelativeAxisType::REL_HWHEEL);

        let device = VirtualDeviceBuilder::new()?
            .name("Dirom Virtual Pointer")
            .input_id(InputId::new(BusType::BUS_USB, 0x1337, 0x1488, 1))
            .with_keys(&keys)?
            .with_relative_axes(&rels)?
            .build()?;

        // Center on screen
        Ok(Self {
            device: Mutex::new(device),
            current_x: Mutex::new(960.0),
            current_y: Mutex::new(540.0),
        })
    }

    #[cfg(target_os = "windows")]
    pub fn new() -> anyhow::Result<Self> {
        crate::screen::ensure_dpi_awareness();
        let mut pt = windows::Win32::Foundation::POINT::default();
        let (x, y) = unsafe {
            if windows::Win32::UI::WindowsAndMessaging::GetCursorPos(&mut pt).is_ok() {
                (pt.x as f64, pt.y as f64)
            } else {
                (0.0, 0.0)
            }
        };
        Ok(Self {
            current_x: Mutex::new(x),
            current_y: Mutex::new(y),
        })
    }

    #[cfg(not(any(target_os = "linux", target_os = "windows")))]
    pub fn new() -> anyhow::Result<Self> {
        Ok(Self {
            current_x: Mutex::new(960.0),
            current_y: Mutex::new(540.0),
        })
    }

    /// Smooth Bezier curve movement from current position to target (target_x, target_y) on Linux,
    /// or direct Win32 `SendInput` absolute positioning on Windows.
    #[cfg(target_os = "linux")]
    pub fn move_to(&self, target_x: f64, target_y: f64, duration_ms: u64) -> anyhow::Result<()> {
        let start_x = *self.current_x.lock().unwrap();
        let start_y = *self.current_y.lock().unwrap();

        let steps = (duration_ms / 10).clamp(10, 50) as usize;
        let dt = Duration::from_millis((duration_ms / steps as u64).max(1));

        // Control points for cubic Bezier curve (adds subtle human arc)
        let dx = target_x - start_x;
        let dy = target_y - start_y;
        let ctrl_x1 = start_x + dx * 0.25 - dy * 0.1;
        let ctrl_y1 = start_y + dy * 0.25 + dx * 0.1;
        let ctrl_x2 = start_x + dx * 0.75 + dy * 0.05;
        let ctrl_y2 = start_y + dy * 0.75 - dx * 0.05;

        let mut last_px = start_x;
        let mut last_py = start_y;

        for i in 1..=steps {
            let t = i as f64 / steps as f64;
            // Ease-in-out easing function
            let ease = t * t * (3.0 - 2.0 * t);

            // Cubic Bezier interpolation
            let u = 1.0 - ease;
            let px = u * u * u * start_x
                + 3.0 * u * u * ease * ctrl_x1
                + 3.0 * u * ease * ease * ctrl_x2
                + ease * ease * ease * target_x;
            let py = u * u * u * start_y
                + 3.0 * u * u * ease * ctrl_y1
                + 3.0 * u * ease * ease * ctrl_y2
                + ease * ease * ease * target_y;

            let rel_x = (px - last_px).round() as i32;
            let rel_y = (py - last_py).round() as i32;

            if rel_x != 0 || rel_y != 0 {
                self.emit_relative(rel_x, rel_y)?;
                last_px += rel_x as f64;
                last_py += rel_y as f64;
            }

            sleep(dt);
        }

        *self.current_x.lock().unwrap() = target_x;
        *self.current_y.lock().unwrap() = target_y;
        Ok(())
    }

    #[cfg(target_os = "windows")]
    pub fn move_to(&self, target_x: f64, target_y: f64, _duration_ms: u64) -> anyhow::Result<()> {
        windows_move_to(target_x.round() as i32, target_y.round() as i32)?;
        *self.current_x.lock().unwrap() = target_x;
        *self.current_y.lock().unwrap() = target_y;
        Ok(())
    }

    #[cfg(not(any(target_os = "linux", target_os = "windows")))]
    pub fn move_to(&self, target_x: f64, target_y: f64, _duration_ms: u64) -> anyhow::Result<()> {
        *self.current_x.lock().unwrap() = target_x;
        *self.current_y.lock().unwrap() = target_y;
        Ok(())
    }

    #[cfg(target_os = "linux")]
    fn linux_key_for_button(button: &str) -> Key {
        match button.trim().to_ascii_lowercase().as_str() {
            "right" => Key::BTN_RIGHT,
            "middle" => Key::BTN_MIDDLE,
            _ => Key::BTN_LEFT,
        }
    }

    #[cfg(target_os = "linux")]
    pub fn mouse_button(&self, button: &str, down: bool) -> anyhow::Result<()> {
        let key = Self::linux_key_for_button(button);
        let mut dev = self.device.lock().unwrap();
        let ev = InputEvent::new(EventType::KEY, key.code(), if down { 1 } else { 0 });
        let syn = InputEvent::new(EventType::SYNCHRONIZATION, 0, 0);
        dev.emit(&[ev, syn])?;
        Ok(())
    }

    #[cfg(target_os = "linux")]
    pub fn click(&self, button: &str) -> anyhow::Result<()> {
        self.click_count(button, 1)
    }

    #[cfg(target_os = "linux")]
    pub fn click_count(&self, button: &str, clicks: u8) -> anyhow::Result<()> {
        let count = clicks.max(1);
        for i in 0..count {
            self.mouse_button(button, true)?;
            sleep(Duration::from_millis(35));
            self.mouse_button(button, false)?;
            if i + 1 < count {
                sleep(Duration::from_millis(45));
            }
        }
        Ok(())
    }

    #[cfg(target_os = "linux")]
    pub fn scroll(&self, dx: i32, dy: i32) -> anyhow::Result<()> {
        let mut dev = self.device.lock().unwrap();
        let syn = InputEvent::new(EventType::SYNCHRONIZATION, 0, 0);
        if dy != 0 {
            let ev_v = InputEvent::new(EventType::RELATIVE, RelativeAxisType::REL_WHEEL.0, dy);
            dev.emit(&[ev_v, syn])?;
        }
        if dx != 0 {
            let ev_h = InputEvent::new(EventType::RELATIVE, RelativeAxisType::REL_HWHEEL.0, dx);
            dev.emit(&[ev_h, syn])?;
        }
        Ok(())
    }

    #[cfg(target_os = "linux")]
    pub fn drag(
        &self,
        from_x: f64,
        from_y: f64,
        to_x: f64,
        to_y: f64,
        button: &str,
        duration_ms: u64,
    ) -> anyhow::Result<()> {
        self.move_to(from_x, from_y, 120)?;
        sleep(Duration::from_millis(20));
        self.mouse_button(button, true)?;
        sleep(Duration::from_millis(30));
        self.move_to(to_x, to_y, duration_ms.max(120))?;
        sleep(Duration::from_millis(20));
        self.mouse_button(button, false)?;
        Ok(())
    }

    #[cfg(target_os = "windows")]
    pub fn click(&self, button: &str) -> anyhow::Result<()> {
        windows_click_current(button, 1)
    }

    #[cfg(not(any(target_os = "linux", target_os = "windows")))]
    pub fn click(&self, _button: &str) -> anyhow::Result<()> {
        Ok(())
    }

    #[cfg(target_os = "linux")]
    fn emit_relative(&self, dx: i32, dy: i32) -> anyhow::Result<()> {
        let mut dev = self.device.lock().unwrap();
        let ev_x = InputEvent::new(EventType::RELATIVE, RelativeAxisType::REL_X.0, dx);
        let ev_y = InputEvent::new(EventType::RELATIVE, RelativeAxisType::REL_Y.0, dy);
        let syn = InputEvent::new(EventType::SYNCHRONIZATION, 0, 0);
        dev.emit(&[ev_x, ev_y, syn])?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::to_virtual_desktop_coord;

    #[test]
    fn maps_single_monitor_corners_and_pixels() {
        assert_eq!(to_virtual_desktop_coord(0, 0, 1920), 17);
        assert_eq!(to_virtual_desktop_coord(1919, 0, 1920), 65518);
        assert_eq!(to_virtual_desktop_coord(960, 0, 1920), 32785);

        // Verify every pixel round-trips accurately under both Win32 mapping formulas.
        for x in 0..1920 {
            let dx = to_virtual_desktop_coord(x, 0, 1920);
            assert_eq!((dx * 1920) / 65536, x);
            assert_eq!((dx * 1919 + 32767) / 65535, x);
        }
    }

    #[test]
    fn maps_multi_monitor_with_negative_origin() {
        // Left monitor 1920px wide starting at -1920 + primary 1920px wide = 3840 total
        assert_eq!(to_virtual_desktop_coord(-1920, -1920, 3840), 8);
        assert_eq!(to_virtual_desktop_coord(0, -1920, 3840), 32776);
        assert_eq!(to_virtual_desktop_coord(1919, -1920, 3840), 65527);

        for x in -1920..1920 {
            let dx = to_virtual_desktop_coord(x, -1920, 3840);
            assert_eq!(-1920 + (dx * 3840) / 65536, x);
            assert_eq!(-1920 + (dx * 3839 + 32767) / 65535, x);
        }
    }
}

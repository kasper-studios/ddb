#[cfg(target_os = "linux")]
use evdev::uinput::VirtualDeviceBuilder;
#[cfg(target_os = "linux")]
use evdev::{AttributeSet, BusType, EventType, InputEvent, InputId, Key, RelativeAxisType};
use std::sync::Mutex;
use std::thread::sleep;
use std::time::Duration;

pub struct DiromPointer {
    #[cfg(target_os = "linux")]
    device: Mutex<evdev::uinput::VirtualDevice>,
    current_x: Mutex<f64>,
    current_y: Mutex<f64>,
}

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

        let device = VirtualDeviceBuilder::new()?
            .name("Dirom Virtual Pointer")
            .input_id(InputId::new(BusType::BUS_USB, 0x1337, 0x1488, 1))
            .with_keys(&keys)?
            .with_relative_axes(&rels)?
            .build()?;

        Ok(Self {
            device: Mutex::new(device),
            current_x: Mutex::new(960.0), // Center of 1920x1080
            current_y: Mutex::new(540.0),
        })
    }

    #[cfg(not(target_os = "linux"))]
    pub fn new() -> anyhow::Result<Self> {
        Ok(Self {
            current_x: Mutex::new(960.0),
            current_y: Mutex::new(540.0),
        })
    }

    /// Smooth Bezier curve movement from current position to target (target_x, target_y)
    pub fn move_to(&self, target_x: f64, target_y: f64, duration_ms: u64) -> anyhow::Result<()> {
        let start_x = *self.current_x.lock().unwrap();
        let start_y = *self.current_y.lock().unwrap();

        let steps = (duration_ms / 10).clamp(10, 50) as usize;
        let dt = Duration::from_millis(duration_ms / steps as u64);

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

    #[cfg(target_os = "linux")]
    pub fn click(&self, button: &str) -> anyhow::Result<()> {
        let key = match button {
            "right" => Key::BTN_RIGHT,
            "middle" => Key::BTN_MIDDLE,
            _ => Key::BTN_LEFT,
        };

        let mut dev = self.device.lock().unwrap();
        // Down
        let ev_down = InputEvent::new(EventType::KEY, key.code(), 1);
        let syn = InputEvent::new(EventType::SYNCHRONIZATION, 0, 0);
        dev.emit(&[ev_down, syn])?;

        sleep(Duration::from_millis(50));

        // Up
        let ev_up = InputEvent::new(EventType::KEY, key.code(), 0);
        dev.emit(&[ev_up, syn])?;

        Ok(())
    }

    #[cfg(not(target_os = "linux"))]
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

    #[cfg(not(target_os = "linux"))]
    fn emit_relative(&self, _dx: i32, _dy: i32) -> anyhow::Result<()> {
        Ok(())
    }
}

//! Keeps the mouse where you left it when you touch the panel.
//!
//! Windows moves the cursor to every touch, so each tap on the small display
//! dragged the mouse off the main one. A low-level mouse hook remembers where
//! the real mouse last was (events Windows makes from touch carry a documented
//! tag, and are skipped), and when a touch ends the page asks for it back.

#[cfg(windows)]
mod imp {
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::Duration;
    use windows::Win32::Foundation::{LPARAM, LRESULT, POINT, WPARAM};
    use windows::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows::Win32::UI::WindowsAndMessaging::*;

    /// The tag in `dwExtraInfo` of mouse events Windows makes from touch or pen.
    const MI_WP_SIGNATURE: usize = 0xFF51_5700;
    const SIGNATURE_MASK: usize = 0xFFFF_FF00;
    const UNKNOWN: u64 = u64::MAX;

    /// Where the real mouse last was, packed as two i32s.
    static LAST: AtomicU64 = AtomicU64::new(UNKNOWN);

    pub(super) fn pack(p: POINT) -> u64 {
        ((p.x as u32 as u64) << 32) | p.y as u32 as u64
    }

    pub(super) fn unpack(v: u64) -> POINT {
        POINT { x: (v >> 32) as u32 as i32, y: v as u32 as i32 }
    }

    pub(super) fn from_touch_or_pen(extra: usize) -> bool {
        extra & SIGNATURE_MASK == MI_WP_SIGNATURE
    }

    /// Runs for every mouse event on the system, so it only records and returns.
    unsafe extern "system" fn hook(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
        if code >= 0 {
            let info = unsafe { &*(lparam.0 as *const MSLLHOOKSTRUCT) };
            if !from_touch_or_pen(info.dwExtraInfo) {
                LAST.store(pack(info.pt), Ordering::Relaxed);
            }
        }
        unsafe { CallNextHookEx(None, code, wparam, lparam) }
    }

    pub fn start() {
        std::thread::spawn(|| unsafe {
            let mut p = POINT::default();
            if GetCursorPos(&mut p).is_ok() {
                LAST.store(pack(p), Ordering::Relaxed);
            }
            let module = GetModuleHandleW(None).ok().map(Into::into);
            let Ok(_hook) = SetWindowsHookExW(WH_MOUSE_LL, Some(hook), module, 0) else {
                return;
            };
            // A low-level hook is called on the thread that set it, so this
            // thread must keep pumping messages or Windows drops the hook.
            let mut msg = MSG::default();
            while GetMessageW(&mut msg, None, 0, 0).as_bool() {}
        });
    }

    pub fn restore() {
        if LAST.load(Ordering::Relaxed) == UNKNOWN {
            return;
        }
        std::thread::spawn(|| {
            // Windows can still move the cursor as the finger lifts.
            std::thread::sleep(Duration::from_millis(40));
            let p = unpack(LAST.load(Ordering::Relaxed));
            let _ = unsafe { SetCursorPos(p.x, p.y) };
        });
    }
}

#[cfg(windows)]
pub use imp::{restore, start};

#[cfg(not(windows))]
pub fn start() {}
#[cfg(not(windows))]
pub fn restore() {}

#[cfg(all(test, windows))]
mod tests {
    use super::imp::*;
    use windows::Win32::Foundation::POINT;

    #[test]
    fn tells_touch_from_mouse() {
        assert!(from_touch_or_pen(0xFF51_5780)); // touch
        assert!(from_touch_or_pen(0xFF51_5700)); // pen
        assert!(!from_touch_or_pen(0)); // a real mouse
        assert!(!from_touch_or_pen(0x1234_5678));
    }

    #[test]
    fn positions_survive_packing() {
        // monitors left of or above the primary have negative coordinates
        for (x, y) in [(0, 0), (1919, 1079), (-800, 240), (-1, -1), (i32::MAX, i32::MIN)] {
            let p = unpack(pack(POINT { x, y }));
            assert_eq!((p.x, p.y), (x, y));
        }
    }
}

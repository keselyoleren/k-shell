//! Copying secrets (passwords, private keys) to the clipboard: the copy is marked as
//! concealed so clipboard managers and the OS clipboard history skip it, and it is cleared
//! again after a timeout if the clipboard still holds it.

use parking_lot::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::OnceLock;
use std::time::Duration;

use crate::error::{Ctx, Res};

/// One clipboard handle for the life of the app: on Linux (X11) the data is served by the
/// process that set it, so the handle must not be dropped right after copying.
fn clipboard() -> Res<&'static Mutex<arboard::Clipboard>> {
    static CLIP: OnceLock<Mutex<arboard::Clipboard>> = OnceLock::new();
    if let Some(c) = CLIP.get() {
        return Ok(c);
    }
    let c = arboard::Clipboard::new().s()?;
    Ok(CLIP.get_or_init(|| Mutex::new(c)))
}

/// Bumped on every secret copy, so only the latest copy's timer clears the clipboard.
static GENERATION: AtomicU64 = AtomicU64::new(0);

/// Put `text` on the clipboard as a concealed item; clear it after `clear_after` seconds
/// (0 = never) unless something else has been copied since.
pub fn copy_secret(text: String, clear_after: u32) -> Res<()> {
    {
        let mut cb = clipboard()?.lock();
        let set = cb.set();
        #[cfg(target_os = "macos")]
        let set = {
            use arboard::SetExtApple;
            set.exclude_from_history()
        };
        #[cfg(windows)]
        let set = {
            use arboard::SetExtWindows;
            set.exclude_from_monitoring()
        };
        #[cfg(all(unix, not(target_os = "macos")))]
        let set = {
            use arboard::SetExtLinux;
            set.exclude_from_history()
        };
        set.text(text.clone()).s()?;
    }
    let generation = GENERATION.fetch_add(1, Ordering::SeqCst) + 1;
    if clear_after > 0 {
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_secs(clear_after.into()));
            if GENERATION.load(Ordering::SeqCst) == generation {
                let _ = clear_if_holds(&text);
            }
        });
    }
    Ok(())
}

fn clear_if_holds(text: &str) -> Res<()> {
    let mut cb = clipboard()?.lock();
    if cb.get_text().ok().as_deref() == Some(text) {
        cb.clear().s()?;
    }
    Ok(())
}

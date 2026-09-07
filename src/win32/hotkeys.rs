//! Global hotkeys. Registered with a NULL window so WM_HOTKEY arrives as a thread message.

use windows::Win32::UI::Input::KeyboardAndMouse::{
    RegisterHotKey, UnregisterHotKey, MOD_NOREPEAT, MOD_SHIFT, MOD_WIN, VIRTUAL_KEY, VK_B, VK_F, VK_K, VK_L, VK_N, VK_Q,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Hotkey {
    Compact,
    Retile,
    ToggleFloat,
    StackNext,
    StackPrev,
    Quit,
}

impl Hotkey {
    pub const ALL: [Hotkey; 6] =
        [Hotkey::Compact, Hotkey::Retile, Hotkey::ToggleFloat, Hotkey::StackNext, Hotkey::StackPrev, Hotkey::Quit];

    pub fn id(self) -> i32 {
        match self {
            Hotkey::Compact => 1,
            Hotkey::Retile => 2,
            Hotkey::ToggleFloat => 3,
            Hotkey::StackNext => 4,
            Hotkey::StackPrev => 5,
            Hotkey::Quit => 9,
        }
    }

    pub fn from_id(id: i32) -> Option<Hotkey> {
        Hotkey::ALL.into_iter().find(|h| h.id() == id)
    }

    pub fn key(self) -> VIRTUAL_KEY {
        match self {
            Hotkey::Compact => VK_K,
            Hotkey::Retile => VK_L,
            Hotkey::ToggleFloat => VK_F,
            Hotkey::StackNext => VK_N,
            Hotkey::StackPrev => VK_B,
            Hotkey::Quit => VK_Q,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Hotkey::Compact => "Win+Shift+K  compact the monitor under the cursor",
            Hotkey::Retile => "Win+Shift+L  re-read monitors and re-apply layout",
            Hotkey::ToggleFloat => "Win+Shift+F  toggle floating for the foreground window",
            Hotkey::StackNext => "Win+Shift+N  next window in the focused stack",
            Hotkey::StackPrev => "Win+Shift+B  previous window in the focused stack",
            Hotkey::Quit => "Win+Shift+Q  quit",
        }
    }
}

/// Register every hotkey. A failure is logged, not fatal: another program owns that chord.
pub fn register_all() {
    for hk in Hotkey::ALL {
        // SAFETY: no pointers; NULL hwnd routes WM_HOTKEY to this thread's queue.
        match unsafe { RegisterHotKey(None, hk.id(), MOD_WIN | MOD_SHIFT | MOD_NOREPEAT, hk.key().0 as u32) } {
            Ok(()) => log::info!("{}", hk.label()),
            Err(e) => {
                log::warn!("{} — NOT registered ({e}); another program owns this chord", hk.label())
            }
        }
    }
}

pub fn unregister_all() {
    for hk in Hotkey::ALL {
        // SAFETY: mirrors register_all.
        unsafe {
            let _ = UnregisterHotKey(None, hk.id());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_round_trip_and_are_unique() {
        let mut seen = std::collections::HashSet::new();
        for hk in Hotkey::ALL {
            assert_eq!(Hotkey::from_id(hk.id()), Some(hk));
            assert!(seen.insert(hk.id()), "duplicate id for {hk:?}");
            assert!(hk.label().starts_with("Win+Shift+"));
        }
        assert_eq!(Hotkey::from_id(12345), None);
    }

    #[test]
    fn keys_are_distinct() {
        let mut seen = std::collections::HashSet::new();
        for hk in Hotkey::ALL {
            assert!(seen.insert(hk.key().0), "duplicate key for {hk:?}");
        }
    }
}

#![forbid(unsafe_code)]

use serde::{Deserialize, Serialize};

pub const COOLDOWN_MS: u64 = 3_000;

pub fn content_scale(width: i32, height: i32, dpi: u32) -> f64 {
    (dpi as f64 / 96.0)
        .min(width as f64 / 760.0)
        .min(height as f64 / 650.0)
        .max(0.25)
}

#[derive(Clone, Copy)]
pub enum InputKind {
    Keyboard,
    Mouse,
}

#[derive(Clone, Copy)]
pub enum SessionEvent {
    Lock,
    Unlock,
    Disconnect,
    Connect,
}

#[derive(Default)]
pub struct WaitingState {
    pub keyboard_events: u64,
    pub mouse_events: u64,
    pub prompts: u64,
    pub session_events: u64,
    locked: bool,
    disconnected: bool,
    last_prompt_ms: Option<u64>,
}

impl WaitingState {
    pub fn visible(&self) -> bool {
        !self.locked && !self.disconnected
    }

    // Receives only a category and monotonic elapsed time. Key values, characters,
    // mouse coordinates and window titles never enter the state or persistence model.
    pub fn touch(&mut self, kind: InputKind, now_ms: u64) {
        if !self.visible() {
            return;
        }
        let count = match kind {
            InputKind::Keyboard => &mut self.keyboard_events,
            InputKind::Mouse => &mut self.mouse_events,
        };
        *count = count.saturating_add(1);
        if self.remaining_ms(now_ms) == 0 {
            self.prompts = self.prompts.saturating_add(1);
            self.last_prompt_ms = Some(now_ms);
        }
    }

    pub fn remaining_ms(&self, now_ms: u64) -> u64 {
        self.last_prompt_ms
            .map(|last| COOLDOWN_MS.saturating_sub(now_ms.saturating_sub(last)))
            .unwrap_or(0)
    }

    pub fn session(&mut self, event: SessionEvent) {
        self.session_events = self.session_events.saturating_add(1);
        match event {
            SessionEvent::Lock => self.locked = true,
            SessionEvent::Unlock => self.locked = false,
            SessionEvent::Disconnect => self.disconnected = true,
            SessionEvent::Connect => self.disconnected = false,
        }
        self.last_prompt_ms = None;
    }
}

// Bounded aggregate snapshot. Identity and input payloads have no fields here.
#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Snapshot {
    pub schema_version: u32,
    pub process_id: u32,
    pub started_at_unix_ms: u128,
    pub elapsed_ms: u64,
    pub clean_exit: bool,
    pub recovered_previous_run: bool,
    pub keyboard_events: u64,
    pub mouse_events: u64,
    pub prompts: u64,
    pub session_events: u64,
    pub visible: bool,
    pub monitor_count: usize,
    pub monitor_dpi: Vec<u32>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn content_fits_small_large_portrait_and_scaled_displays() {
        for (width, height) in [
            (640, 480),
            (800, 600),
            (1080, 1920),
            (1920, 1080),
            (3840, 2160),
        ] {
            for dpi in [96, 120, 144, 192, 288] {
                let scale = content_scale(width, height, dpi);
                assert!(680.0 * scale <= width as f64);
                assert!(580.0 * scale <= height as f64);
                assert!(scale <= dpi as f64 / 96.0);
                assert!(23.0 * scale >= 12.0);
            }
        }
    }

    #[test]
    fn cooldown_counts_every_message_without_extending_prompt() {
        let mut state = WaitingState::default();
        state.touch(InputKind::Keyboard, 10);
        state.touch(InputKind::Mouse, 11);
        state.touch(InputKind::Keyboard, 3009);
        assert_eq!(
            (state.keyboard_events, state.mouse_events, state.prompts),
            (2, 1, 1)
        );
        assert_eq!(state.remaining_ms(3009), 1);
        state.touch(InputKind::Mouse, 3010);
        assert_eq!(state.prompts, 2);
        assert_eq!(state.remaining_ms(3010), COOLDOWN_MS);
    }

    #[test]
    fn overlapping_lock_disconnect_requires_both_recoveries() {
        let mut state = WaitingState::default();
        state.session(SessionEvent::Lock);
        state.session(SessionEvent::Disconnect);
        state.session(SessionEvent::Unlock);
        assert!(!state.visible());
        state.touch(InputKind::Keyboard, 0);
        assert_eq!(state.keyboard_events, 0);
        state.session(SessionEvent::Connect);
        assert!(state.visible());
        state.touch(InputKind::Mouse, 1);
        assert_eq!(state.prompts, 1);
        assert_eq!(state.session_events, 4);
    }

    #[test]
    fn counter_saturation_and_clock_regression_are_bounded() {
        let mut state = WaitingState {
            keyboard_events: u64::MAX,
            ..Default::default()
        };
        state.touch(InputKind::Keyboard, 100);
        state.touch(InputKind::Keyboard, 99);
        assert_eq!(state.keyboard_events, u64::MAX);
        assert_eq!(state.prompts, 1);
        assert_eq!(state.remaining_ms(99), COOLDOWN_MS);
    }
}

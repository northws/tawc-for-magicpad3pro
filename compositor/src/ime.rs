// SPDX-License-Identifier: MIT

//! Minimal `zwp_input_method_v2` support.
//!
//! TAWC's own IME is Android's, bridged through the custom
//! `zwp_text_input_v3` implementation in [`crate::text_input`]. This module
//! adds the *other* side of the protocol: a Wayland input method client —
//! fcitx5 running inside the chroot — can grab the seat keyboard, compose
//! with the physical keyboard, and have its commit/preedit/delete requests
//! delivered to the focused client's text-input object.
//!
//! Scope: enough for a real IME to work. The candidate popup surface is
//! accepted and tracked but not rendered yet; until it is, candidate
//! selection works blind (space/number keys).

use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};

use log::info;

use smithay::backend::input::KeyState;
use smithay::input::keyboard::{
    GrabStartData, KeymapFile, KeyboardGrab, KeyboardInnerHandle, Keycode, ModifiersState,
};
use wayland_protocols::wp::text_input::zv3::server::zwp_text_input_v3::{
    ChangeCause, ContentHint, ContentPurpose,
};
use smithay::reexports::wayland_server::{
    Client, DataInit, Dispatch, DisplayHandle, GlobalDispatch, New, Resource,
    protocol::wl_keyboard::KeymapFormat,
    protocol::wl_surface::WlSurface,
};
use smithay::utils::Serial;
use smithay::reexports::wayland_protocols_misc::zwp_input_method_v2::server::{
    zwp_input_method_keyboard_grab_v2::{self, ZwpInputMethodKeyboardGrabV2},
    zwp_input_method_manager_v2::{self, ZwpInputMethodManagerV2},
    zwp_input_method_v2::{self, ZwpInputMethodV2},
    zwp_input_popup_surface_v2::{self, ZwpInputPopupSurfaceV2},
};

use crate::compositor::TawcState;

/// Whether a Wayland input method currently owns text input. Consulted by
/// the Android soft-keyboard path: when fcitx is composing, showing Gboard
/// would both cover the screen and let taps commit text behind fcitx's back.
static WAYLAND_IME_ACTIVE: AtomicBool = AtomicBool::new(false);
/// Number of key events forwarded to the IM so far. The first few are echoed
/// as toasts: they show whether TAWC sends the right (evdev + 8) codes and
/// whether releases arrive at all.
static IME_KEY_EVENTS: AtomicU32 = AtomicU32::new(0);
/// Number of text batches the IM has produced; the first few are echoed.
static IME_COMMITS: AtomicU32 = AtomicU32::new(0);

/// Set only once the IM actually holds the keyboard grab. Soft-keyboard
/// suppression keys off this, so an IM that connects but never grabs cannot
/// leave the user without any working keyboard.
static WAYLAND_IME_GRABBED: AtomicBool = AtomicBool::new(false);

/// Whether a Wayland input method is active *and* owns the keyboard.
pub fn wayland_ime_active() -> bool {
    WAYLAND_IME_ACTIVE.load(Ordering::Relaxed) && WAYLAND_IME_GRABBED.load(Ordering::Relaxed)
}

/// Text the input method produced since the last `commit`.
#[derive(Default)]
struct PendingText {
    /// `commit_string` payload (may arrive in several requests).
    commit: String,
    /// `set_preedit_string(text, cursor_begin, cursor_end)`.
    preedit: Option<(String, i32, i32)>,
    /// `delete_surrounding_text(before, after)` in UTF-8 bytes.
    delete_before: u32,
    delete_after: u32,
}

impl PendingText {
    fn is_empty(&self) -> bool {
        self.commit.is_empty()
            && self.preedit.is_none()
            && self.delete_before == 0
            && self.delete_after == 0
    }
}

/// All `zwp_input_method_v2` state. Lives in [`TawcState::ime`].
#[derive(Default)]
pub struct ImeState {
    instance: Option<ZwpInputMethodV2>,
    keyboard_grab: Option<ZwpInputMethodKeyboardGrabV2>,
    popup: Option<ZwpInputPopupSurfaceV2>,
    /// Surface the IM drew its candidate window into, if any.
    pub popup_surface: Option<WlSurface>,
    /// Whether the IM currently believes a text field is active. Drives
    /// activate/deactivate edges — never resend activate while active, or
    /// fcitx resets its composition on every keystroke.
    active: bool,
    /// What the IM was last told, so client-side text changes that don't
    /// actually change anything don't get echoed back (a feedback loop:
    /// preedit -> client surrounding update -> IM -> preedit).
    last_surrounding: Option<(String, i32, i32)>,
    last_content: Option<(u32, u32)>,
    /// Last preedit text forwarded to the client, so identical updates are
    /// not re-sent (fcitx re-emits them on every panel update).
    last_preedit: Option<String>,
    pending: PendingText,
}

impl ImeState {
    pub fn new() -> Self {
        Self::default()
    }

    /// Whether an input method client is connected.
    pub fn has_instance(&self) -> bool {
        self.instance.is_some()
    }

    /// Whether the input method currently owns the keyboard.
    pub fn keyboard_grabbed(&self) -> bool {
        self.keyboard_grab.is_some()
    }

    fn unregister(&mut self) {
        self.instance = None;
        self.keyboard_grab = None;
        self.popup = None;
        self.popup_surface = None;
        self.active = false;
        self.last_surrounding = None;
        self.last_content = None;
        self.last_preedit = None;
        WAYLAND_IME_ACTIVE.store(false, Ordering::Relaxed);
        self.pending = PendingText::default();
    }

    /// Tell the input method whether a text field has focus, and hand it the
    /// surrounding text when it does.
    fn on_text_focus_changed(
        &mut self,
        focused: bool,
        surrounding: Option<(String, i32, i32)>,
        content_type: Option<(u32, u32)>,
    ) {
        let Some(im) = self.instance.as_ref() else {
            return;
        };
        info!("ime: text focus changed, focused={focused}");
        if !focused {
            if self.active {
                im.deactivate();
                im.done();
                self.active = false;
                self.last_surrounding = None;
                self.last_content = None;
                WAYLAND_IME_ACTIVE.store(false, Ordering::Relaxed);
                info!("ime: deactivated");
            }
            return;
        }
        let mut sent = false;
        if !self.active {
            im.activate();
            self.active = true;
            self.last_surrounding = None;
            self.last_content = None;
            WAYLAND_IME_ACTIVE.store(true, Ordering::Relaxed);
            info!("ime: activated");
            crate::toast_from_native("IME: activated");
            sent = true;
        }
        // Temporary: never push surrounding/content updates after activation.
        // Firefox re-sends surrounding text on every preedit tick; echoing it
        // back keeps fcitx re-emitting preedit and can saturate the socket.
        if !sent {
            return;
        }
        if let Some(surrounding) = surrounding {
            if self.last_surrounding.as_ref() != Some(&surrounding) {
                let (text, cursor, anchor) = surrounding.clone();
                im.surrounding_text(text, cursor.max(0) as u32, anchor.max(0) as u32);
                // The client changed the text, not the input method.
                im.text_change_cause(ChangeCause::Other);
                self.last_surrounding = Some(surrounding);
                sent = true;
            }
        }
        if let Some(content) = content_type {
            if self.last_content != Some(content) {
                im.content_type(
                    ContentHint::from_bits_truncate(content.0),
                    ContentPurpose::try_from(content.1).unwrap_or(ContentPurpose::Normal),
                );
                self.last_content = Some(content);
                sent = true;
            }
        }
        if sent {
            im.done();
        }
        self.pending = PendingText::default();
    }

    /// Apply the pending text batch to the focused client, then acknowledge.
    fn flush(&mut self, data: &mut TawcState) {
        if self.pending.is_empty() {
            return;
        }
        let pending = std::mem::take(&mut self.pending);
        info!(
            "ime: commit preedit={:?} commit_len={} delete=({}, {})",
            pending.preedit.as_ref().map(|(t, _, _)| t.as_str()),
            pending.commit.len(),
            pending.delete_before,
            pending.delete_after,
        );
        let _ = IME_COMMITS.fetch_add(1, Ordering::Relaxed);
        let commit = if pending.commit.is_empty() {
            None
        } else {
            Some(pending.commit)
        };
        // Android's IME path always reports the preedit cursor at the end and
        // never uses the protocol's -1 sentinel; mirror that for fcitx, whose
        // own cursor offsets are what froze Firefox. Identical updates are
        // also skipped.
        let mut preedit = pending.preedit.map(|(text, _cb, _ce)| {
            let len = text.len() as i32;
            (text, len, len)
        });
        if commit.is_some() {
            preedit = None;
            self.last_preedit = None;
        } else if let Some((text, _, _)) = preedit.as_ref() {
            if self.last_preedit.as_deref() == Some(text.as_str()) {
                preedit = None;
            } else {
                self.last_preedit = Some(text.clone());
            }
        }
        if preedit.is_none()
            && commit.is_none()
            && pending.delete_before == 0
            && pending.delete_after == 0
        {
            return;
        }
        let delivered = data.text_input_state.handle_ime_text(
            preedit,
            commit,
            pending.delete_before,
            pending.delete_after,
        );
        if !delivered {
            crate::toast_from_native("IME: commit dropped (no focus)");
        }
    }

    fn grab_keyboard(&mut self, data: &mut TawcState, grab: ZwpInputMethodKeyboardGrabV2) {
        let Some(keyboard) = data.seat.get_keyboard() else {
            return;
        };

        // Keymap: the IM needs the seat's current xkb keymap to decode the
        // key codes it receives.
        let keymap = keyboard.with_xkb_state(data, |ctx| {
            let xkb = ctx.xkb().lock().unwrap();
            let raw = unsafe { xkb.keymap() };
            KeymapFile::new(raw)
        });
        let _ = keymap.with_fd(false, |fd, size| {
            grab.keymap(KeymapFormat::XkbV1, fd, size as u32);
        });

        // Tell the IM not to synthesize its own key repeat: TAWC already
        // suppresses Android's repeat events, and letting fcitx repeat turns
        // a single tap into a burst of identical characters.
        grab.repeat_info(0, 0);
        grab.modifiers(
            smithay::utils::SERIAL_COUNTER.next_serial().into(),
            0,
            0,
            0,
            0,
        );

        let start_data = GrabStartData {
            focus: keyboard.current_focus(),
        };
        let ime_grab = ImeKeyboardGrab {
            grab: grab.clone(),
            start_data,
        };
        let serial = smithay::utils::SERIAL_COUNTER.next_serial();
        info!("ime: keyboard grabbed by input method");
        crate::toast_from_native("IME: keyboard grabbed");
        WAYLAND_IME_GRABBED.store(true, Ordering::Relaxed);
        // Re-arm the key/output diagnostics for this grab session.
        IME_KEY_EVENTS.store(0, Ordering::Relaxed);
        IME_COMMITS.store(0, Ordering::Relaxed);
        keyboard.set_grab(data, ime_grab, serial);
        // Now that fcitx owns the keyboard, get Android's soft keyboard out
        // of the way.
        data.text_input_state.drop_android_keyboard();

        self.keyboard_grab = Some(grab);
    }
}

fn with_ime<R>(data: &mut TawcState, f: impl FnOnce(&mut ImeState, &mut TawcState) -> R) -> R {
    let mut ime = std::mem::take(&mut data.ime);
    let result = f(&mut ime, data);
    data.ime = ime;
    result
}

/// Forwards the seat keyboard to the input method while it holds the grab.
struct ImeKeyboardGrab {
    grab: ZwpInputMethodKeyboardGrabV2,
    start_data: GrabStartData<TawcState>,
}

impl KeyboardGrab<TawcState> for ImeKeyboardGrab {
    fn input(
        &mut self,
        _data: &mut TawcState,
        _handle: &mut KeyboardInnerHandle<'_, TawcState>,
        keycode: Keycode,
        state: KeyState,
        modifiers: Option<ModifiersState>,
        _serial: Serial,
        time: u32,
    ) {
        // wl_keyboard.key / zwp_input_method_keyboard_grab_v2.key carry the
        // *evdev* scancode; clients add the xkbcommon +8 themselves. Smithay's
        // own grab does the same conversion (raw() - 8); sending raw() here
        // would be decoded one key off.
        let evdev = keycode.raw().saturating_sub(8);
        // Keep the counter for diagnostics without a JNI call on the hot path.
        let _ = IME_KEY_EVENTS.fetch_add(1, Ordering::Relaxed);
        let wl_state = match state {
            KeyState::Pressed => smithay::reexports::wayland_server::protocol::wl_keyboard::KeyState::Pressed,
            KeyState::Released => smithay::reexports::wayland_server::protocol::wl_keyboard::KeyState::Released,
        };
        self.grab
            .key(smithay::utils::SERIAL_COUNTER.next_serial().into(), time, evdev, wl_state);
        if let Some(mods) = modifiers {
            let m = mods.serialized;
            self.grab.modifiers(
                smithay::utils::SERIAL_COUNTER.next_serial().into(),
                m.depressed,
                m.latched,
                m.locked,
                m.layout_effective,
            );
        }
    }

    fn set_focus(
        &mut self,
        _data: &mut TawcState,
        _handle: &mut KeyboardInnerHandle<'_, TawcState>,
        _focus: Option<<TawcState as smithay::input::SeatHandler>::KeyboardFocus>,
        _serial: Serial,
    ) {
    }

    fn start_data(&self) -> &GrabStartData<TawcState> {
        &self.start_data
    }

    fn unset(&mut self, data: &mut TawcState) {
        if data
            .ime
            .keyboard_grab
            .as_ref()
            .is_some_and(|grab| grab.id() == self.grab.id())
        {
            data.ime.keyboard_grab = None;
        }
    }
}

// ---------------------------------------------------------------------------
// Protocol dispatch: zwp_input_method_manager_v2
// ---------------------------------------------------------------------------

impl GlobalDispatch<ZwpInputMethodManagerV2, ()> for TawcState {
    fn bind(
        _state: &mut Self,
        _handle: &DisplayHandle,
        _client: &Client,
        resource: New<ZwpInputMethodManagerV2>,
        _global_data: &(),
        data_init: &mut DataInit<'_, Self>,
    ) {
        data_init.init(resource, ());
    }
}

impl Dispatch<ZwpInputMethodManagerV2, ()> for TawcState {
    fn request(
        state: &mut Self,
        _client: &Client,
        _resource: &ZwpInputMethodManagerV2,
        request: zwp_input_method_manager_v2::Request,
        _data: &(),
        _dhandle: &DisplayHandle,
        data_init: &mut DataInit<'_, Self>,
    ) {
        match request {
            zwp_input_method_manager_v2::Request::GetInputMethod { seat: _, input_method } => {
                let im = data_init.init(input_method, ());
                info!("ime: input method connected");
                crate::toast_from_native("IME: connected");
                with_ime(state, |ime, data| {
                    if let Some(old) = ime.instance.replace(im.clone()) {
                        old.unavailable();
                    }
                    let focused = data.text_input_state.focused_surface.is_some();
                    let surrounding = data.text_input_state.focused_surrounding();
                    let content_type = data.text_input_state.focused_content_type();
                    ime.on_text_focus_changed(focused, surrounding, content_type);
                });
            }
            zwp_input_method_manager_v2::Request::Destroy => {}
            _ => {}
        }
    }
}

// ---------------------------------------------------------------------------
// Protocol dispatch: zwp_input_method_v2
// ---------------------------------------------------------------------------

impl Dispatch<ZwpInputMethodV2, ()> for TawcState {
    fn request(
        state: &mut Self,
        _client: &Client,
        _resource: &ZwpInputMethodV2,
        request: zwp_input_method_v2::Request,
        _data: &(),
        _dhandle: &DisplayHandle,
        data_init: &mut DataInit<'_, Self>,
    ) {
        match request {
            zwp_input_method_v2::Request::CommitString { text } => {
                state.ime.pending.commit.push_str(&text);
            }
            zwp_input_method_v2::Request::SetPreeditString { text, cursor_begin, cursor_end } => {
                state.ime.pending.preedit = Some((text, cursor_begin, cursor_end));
            }
            zwp_input_method_v2::Request::DeleteSurroundingText { before_length, after_length } => {
                state.ime.pending.delete_before += before_length;
                state.ime.pending.delete_after += after_length;
            }
            zwp_input_method_v2::Request::Commit { serial: _ } => {
                with_ime(state, |ime, data| {
                    ime.flush(data);
                    if let Some(im) = ime.instance.as_ref() {
                        im.done();
                    }
                });
            }
            zwp_input_method_v2::Request::GetInputPopupSurface { id, surface } => {
                let popup = data_init.init(id, ());
                state.ime.popup = Some(popup);
                state.ime.popup_surface = Some(surface);
                state.needs_render = true;
            }
            zwp_input_method_v2::Request::GrabKeyboard { keyboard } => {
                let grab = data_init.init(keyboard, ());
                with_ime(state, |ime, data| ime.grab_keyboard(data, grab));
            }
            zwp_input_method_v2::Request::Destroy => {
                teardown(state);
            }
            _ => {}
        }
    }

    fn destroyed(
        state: &mut Self,
        _client: smithay::reexports::wayland_server::backend::ClientId,
        _object: &ZwpInputMethodV2,
        _data: &(),
    ) {
        teardown(state);
    }
}

impl Dispatch<ZwpInputMethodKeyboardGrabV2, ()> for TawcState {
    fn request(
        state: &mut Self,
        _client: &Client,
        _resource: &ZwpInputMethodKeyboardGrabV2,
        request: zwp_input_method_keyboard_grab_v2::Request,
        _data: &(),
        _dhandle: &DisplayHandle,
        _data_init: &mut DataInit<'_, Self>,
    ) {
        let _ = request;
    }

    fn destroyed(
        state: &mut Self,
        _client: smithay::reexports::wayland_server::backend::ClientId,
        _object: &ZwpInputMethodKeyboardGrabV2,
        _data: &(),
    ) {
        release_keyboard_grab(state);
    }
}

/// Drop the keyboard grab when the IM destroys its grab object (or dies)
/// without destroying the whole input method. Leaving a grab pointed at a
/// dead resource would swallow every key.
fn release_keyboard_grab(state: &mut TawcState) {
    if state.ime.keyboard_grab.take().is_none() {
        return;
    }
    info!("ime: keyboard grab released");
    WAYLAND_IME_GRABBED.store(false, Ordering::Relaxed);
    if let Some(keyboard) = state.seat.get_keyboard() {
        if keyboard.is_grabbed() {
            keyboard.unset_grab(state);
        }
    }
}

impl Dispatch<ZwpInputPopupSurfaceV2, ()> for TawcState {
    fn request(
        _state: &mut Self,
        _client: &Client,
        _resource: &ZwpInputPopupSurfaceV2,
        _request: zwp_input_popup_surface_v2::Request,
        _data: &(),
        _dhandle: &DisplayHandle,
        _data_init: &mut DataInit<'_, Self>,
    ) {
    }

    fn destroyed(
        state: &mut Self,
        _client: smithay::reexports::wayland_server::backend::ClientId,
        object: &ZwpInputPopupSurfaceV2,
        _data: &(),
    ) {
        // fcitx recreates its UI window; drop the stale surface so the
        // renderer never touches freed surface data.
        if state
            .ime
            .popup
            .as_ref()
            .is_some_and(|popup| popup.id() == object.id())
        {
            state.ime.popup = None;
            state.ime.popup_surface = None;
        }
    }
}

/// Drop the input method and release the keyboard if it held the grab.
pub fn teardown(data: &mut TawcState) {
    WAYLAND_IME_ACTIVE.store(false, Ordering::Relaxed);
    WAYLAND_IME_GRABBED.store(false, Ordering::Relaxed);
    let had_grab = data.ime.keyboard_grabbed();
    if data.ime.has_instance() || had_grab {
        info!("ime: input method gone");
    }
    with_ime(data, |ime, _data| ime.unregister());
    if had_grab {
        if let Some(keyboard) = data.seat.get_keyboard() {
            if keyboard.is_grabbed() {
                keyboard.unset_grab(data);
            }
        }
    }
}

/// Called from [`TawcState::set_input_focus`] so the IM follows text focus.
pub fn text_focus_changed(data: &mut TawcState) {
    let focused = data.text_input_state.focused_instance_enabled();
    let surrounding = data.text_input_state.focused_surrounding();
    let content_type = data.text_input_state.focused_content_type();
    let activated = with_ime(data, |ime, _data| {
        let was_active = ime.active;
        ime.on_text_focus_changed(focused, surrounding, content_type);
        ime.active && !was_active
    });
    let _ = activated;
}

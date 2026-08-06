//! Heap-free hardware-independent display and keypad state.

#![no_std]
#![forbid(unsafe_code)]

use radio_domain::TxClass;
use radio_tx_policy::{LoadStatus, PermissionSet, StoredPermissions, PERMISSION_RECORD_LEN};

/// TX classes shown by the hidden permission editor, in display order.
///
/// `TxClass::Never` is deliberately absent and cannot be enabled by the UI.
pub const EDITABLE_TX_CLASSES: [TxClass; 6] = [
    TxClass::LicenceFreePlan,
    TxClass::Amateur,
    TxClass::Marine,
    TxClass::Aeronautical,
    TxClass::Business,
    TxClass::Experimental,
];

/// Product-level keypad actions independent of a physical key matrix.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum Key {
    /// Open or save a menu.
    Menu = 0,
    /// Move to the previous item.
    Up = 1,
    /// Move to the next item.
    Down = 2,
    /// Toggle or confirm the selected item.
    Confirm = 3,
    /// Cancel or return without saving.
    Back = 4,
}

/// A bounded set of logical keys held at one instant.
#[must_use]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct KeySet(u8);

impl KeySet {
    /// Returns a set with no keys held.
    pub const fn none() -> Self {
        Self(0)
    }

    /// Returns the exact hidden permission-menu boot gesture.
    pub const fn permission_menu_gesture() -> Self {
        Self::none().with(Key::Menu).with(Key::Back)
    }

    /// Returns a copy with one key held.
    pub const fn with(self, key: Key) -> Self {
        Self(self.0 | key_bit(key))
    }

    /// Reports whether one logical key is held.
    pub const fn contains(self, key: Key) -> bool {
        self.0 & key_bit(key) != 0
    }

    /// Reports whether no logical keys are held.
    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }

    const fn without(self, key: Key) -> Self {
        Self(self.0 & !key_bit(key))
    }
}

const fn key_bit(key: Key) -> u8 {
    1 << (key as u8)
}

/// Press or release edge reported by a logical keypad adapter.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum KeyEdge {
    /// The key changed from released to held.
    Pressed,
    /// The key changed from held to released.
    Released,
}

/// One logical key edge.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct KeyEvent {
    /// Logical key which changed state.
    pub key: Key,
    /// New edge state.
    pub edge: KeyEdge,
}

impl KeyEvent {
    /// Constructs a press edge.
    pub const fn pressed(key: Key) -> Self {
        Self {
            key,
            edge: KeyEdge::Pressed,
        }
    }

    /// Constructs a release edge.
    pub const fn released(key: Key) -> Self {
        Self {
            key,
            edge: KeyEdge::Released,
        }
    }
}

/// A save failure which can be rendered by a display adapter.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MenuError {
    /// The monotonic permission generation cannot advance.
    GenerationExhausted,
}

/// Bounded semantic display output independent of pixels, fonts, and geometry.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UiView {
    /// Ordinary runtime view; Work Package 5 does not define its contents.
    Normal,
    /// The exact boot gesture was accepted and all keys must be released.
    ReleaseBootKeys,
    /// Hidden TX-permission editor state.
    PermissionMenu {
        /// Selected authorisable TX class.
        selected: TxClass,
        /// Draft permission state for the selected class.
        enabled: bool,
        /// Whether the draft differs from the boot-loaded record.
        changed: bool,
        /// Last save error, if any.
        error: Option<MenuError>,
    },
    /// A record was emitted and a validated reboot is required to use it.
    PermissionsSaved {
        /// Monotonic generation encoded in the emitted record.
        generation: u32,
    },
}

/// Side effect requested by one UI key event.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UiAction {
    /// No external action is required.
    None,
    /// The editor was cancelled without emitting persisted bytes.
    MenuCancelled,
    /// Persist this complete redundant permission record.
    PersistPermissions([u8; PERMISSION_RECORD_LEN]),
    /// Saving was refused and no persistence record was emitted.
    SaveRefused(MenuError),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum UiState {
    Normal,
    AwaitBootKeyRelease,
    Editing,
    Saved { generation: u32 },
}

/// Boot-scoped hardware-independent UI controller.
///
/// The hidden permission editor can only be selected by the exact initial key
/// set passed to [`Self::boot`]. Later key events never provide an entry path.
pub struct BootUi {
    state: UiState,
    pressed: KeySet,
    loaded_permissions: PermissionSet,
    draft_permissions: PermissionSet,
    generation: u32,
    selected_index: u8,
    save_error: Option<MenuError>,
}

impl BootUi {
    /// Loads persisted permissions and selects the boot UI path.
    ///
    /// Invalid persisted bytes produce an empty permission draft and a
    /// `DefaultedDenied` status. Only the exact boot gesture enters the hidden
    /// path; incomplete or additional held keys select the normal view.
    pub fn boot(persisted: &[u8], initial_keys: KeySet) -> (Self, LoadStatus) {
        let (permissions, generation, status) = match StoredPermissions::decode(persisted) {
            Ok(stored) => (stored.permissions(), stored.generation(), LoadStatus::Valid),
            Err(error) => (PermissionSet::none(), 0, LoadStatus::DefaultedDenied(error)),
        };
        let state = if initial_keys == KeySet::permission_menu_gesture() {
            UiState::AwaitBootKeyRelease
        } else {
            UiState::Normal
        };
        (
            Self {
                state,
                pressed: initial_keys,
                loaded_permissions: permissions,
                draft_permissions: permissions,
                generation,
                selected_index: 0,
                save_error: None,
            },
            status,
        )
    }

    /// Returns the current bounded semantic display view.
    pub fn view(&self) -> UiView {
        match self.state {
            UiState::Normal => UiView::Normal,
            UiState::AwaitBootKeyRelease => UiView::ReleaseBootKeys,
            UiState::Editing => {
                let selected = self.selected_class();
                UiView::PermissionMenu {
                    selected,
                    enabled: self.draft_permissions.allows(selected),
                    changed: self.draft_permissions != self.loaded_permissions,
                    error: self.save_error,
                }
            }
            UiState::Saved { generation } => UiView::PermissionsSaved { generation },
        }
    }

    /// Applies one logical key edge and returns any requested persistence action.
    ///
    /// Duplicate press or release edges are ignored. Only press edges perform
    /// menu navigation; releases update held-key state and arm the boot menu.
    pub fn handle(&mut self, event: KeyEvent) -> UiAction {
        if !self.apply_edge(event) {
            return UiAction::None;
        }
        if self.state == UiState::AwaitBootKeyRelease {
            if self.pressed.is_empty() {
                self.state = UiState::Editing;
            }
            return UiAction::None;
        }
        if self.state != UiState::Editing || event.edge != KeyEdge::Pressed {
            return UiAction::None;
        }

        match event.key {
            Key::Up => {
                self.selected_index = if self.selected_index == 0 {
                    u8::try_from(EDITABLE_TX_CLASSES.len() - 1).unwrap_or(0)
                } else {
                    self.selected_index - 1
                };
                self.save_error = None;
                UiAction::None
            }
            Key::Down => {
                let next = usize::from(self.selected_index) + 1;
                self.selected_index = if next == EDITABLE_TX_CLASSES.len() {
                    0
                } else {
                    u8::try_from(next).unwrap_or(0)
                };
                self.save_error = None;
                UiAction::None
            }
            Key::Confirm => {
                let selected = self.selected_class();
                let enabled = !self.draft_permissions.allows(selected);
                self.draft_permissions = self.draft_permissions.with(selected, enabled);
                self.save_error = None;
                UiAction::None
            }
            Key::Back => {
                self.state = UiState::Normal;
                self.draft_permissions = self.loaded_permissions;
                self.save_error = None;
                UiAction::MenuCancelled
            }
            Key::Menu => self.save(),
        }
    }

    /// Returns the logical key set currently tracked as held.
    pub const fn pressed_keys(&self) -> KeySet {
        self.pressed
    }

    fn selected_class(&self) -> TxClass {
        EDITABLE_TX_CLASSES[usize::from(self.selected_index)]
    }

    fn apply_edge(&mut self, event: KeyEvent) -> bool {
        let was_pressed = self.pressed.contains(event.key);
        match event.edge {
            KeyEdge::Pressed if !was_pressed => {
                self.pressed = self.pressed.with(event.key);
                true
            }
            KeyEdge::Released if was_pressed => {
                self.pressed = self.pressed.without(event.key);
                true
            }
            KeyEdge::Pressed | KeyEdge::Released => false,
        }
    }

    fn save(&mut self) -> UiAction {
        let Some(generation) = self.generation.checked_add(1) else {
            self.save_error = Some(MenuError::GenerationExhausted);
            return UiAction::SaveRefused(MenuError::GenerationExhausted);
        };
        let record = StoredPermissions::new(self.draft_permissions, generation).encode();
        self.state = UiState::Saved { generation };
        UiAction::PersistPermissions(record)
    }
}

#[cfg(test)]
mod tests {
    use super::{BootUi, Key, KeyEvent, KeySet, MenuError, UiAction, UiView, EDITABLE_TX_CLASSES};
    use radio_domain::TxClass;
    use radio_tx_policy::{
        LoadStatus, PermissionSet, StoredPermissions, TxPolicy, PERMISSION_RECORD_LEN,
    };

    fn denied_record(generation: u32) -> [u8; PERMISSION_RECORD_LEN] {
        StoredPermissions::new(PermissionSet::none(), generation).encode()
    }

    fn release_boot_gesture(ui: &mut BootUi) {
        assert_eq!(ui.handle(KeyEvent::released(Key::Menu)), UiAction::None);
        assert_eq!(ui.view(), UiView::ReleaseBootKeys);
        assert_eq!(ui.handle(KeyEvent::released(Key::Back)), UiAction::None);
    }

    fn release_key(ui: &mut BootUi, key: Key) {
        assert_eq!(ui.handle(KeyEvent::released(key)), UiAction::None);
    }

    #[test]
    fn only_the_exact_boot_gesture_can_enter_the_hidden_menu() {
        let record = denied_record(4);
        let (mut normal, status) = BootUi::boot(&record, KeySet::none());
        assert_eq!(status, LoadStatus::Valid);
        assert_eq!(normal.view(), UiView::Normal);
        for event in [
            KeyEvent::pressed(Key::Menu),
            KeyEvent::pressed(Key::Back),
            KeyEvent::released(Key::Menu),
            KeyEvent::released(Key::Back),
        ] {
            assert_eq!(normal.handle(event), UiAction::None);
            assert_eq!(normal.view(), UiView::Normal);
        }

        let (incomplete, _) = BootUi::boot(&record, KeySet::none().with(Key::Menu));
        assert_eq!(incomplete.view(), UiView::Normal);
        let extra_keys = KeySet::permission_menu_gesture().with(Key::Confirm);
        let (extra, _) = BootUi::boot(&record, extra_keys);
        assert_eq!(extra.view(), UiView::Normal);

        let (mut hidden, _) = BootUi::boot(&record, KeySet::permission_menu_gesture());
        assert_eq!(hidden.view(), UiView::ReleaseBootKeys);
        release_boot_gesture(&mut hidden);
        assert_eq!(
            hidden.view(),
            UiView::PermissionMenu {
                selected: TxClass::LicenceFreePlan,
                enabled: false,
                changed: false,
                error: None,
            }
        );
    }

    #[test]
    fn navigation_toggle_and_cancel_are_bounded_and_emit_no_record() {
        assert!(!EDITABLE_TX_CLASSES.contains(&TxClass::Never));
        let record = denied_record(2);
        let (mut ui, _) = BootUi::boot(&record, KeySet::permission_menu_gesture());
        release_boot_gesture(&mut ui);

        assert_eq!(ui.handle(KeyEvent::pressed(Key::Down)), UiAction::None);
        assert_eq!(
            ui.view(),
            UiView::PermissionMenu {
                selected: TxClass::Amateur,
                enabled: false,
                changed: false,
                error: None,
            }
        );
        assert_eq!(ui.handle(KeyEvent::pressed(Key::Down)), UiAction::None);
        assert_eq!(
            ui.view(),
            UiView::PermissionMenu {
                selected: TxClass::Amateur,
                enabled: false,
                changed: false,
                error: None,
            }
        );
        release_key(&mut ui, Key::Down);
        assert_eq!(ui.handle(KeyEvent::pressed(Key::Confirm)), UiAction::None);
        assert_eq!(
            ui.view(),
            UiView::PermissionMenu {
                selected: TxClass::Amateur,
                enabled: true,
                changed: true,
                error: None,
            }
        );
        release_key(&mut ui, Key::Confirm);
        assert_eq!(
            ui.handle(KeyEvent::pressed(Key::Back)),
            UiAction::MenuCancelled
        );
        assert_eq!(ui.view(), UiView::Normal);
    }

    #[test]
    fn deliberate_save_requires_validated_reboot_before_policy_changes() {
        let record = denied_record(7);
        let (live_policy, status) = TxPolicy::load(&record);
        assert_eq!(status, LoadStatus::Valid);
        let (mut ui, _) = BootUi::boot(&record, KeySet::permission_menu_gesture());
        release_boot_gesture(&mut ui);
        assert_eq!(ui.handle(KeyEvent::pressed(Key::Confirm)), UiAction::None);
        release_key(&mut ui, Key::Confirm);
        let UiAction::PersistPermissions(saved) = ui.handle(KeyEvent::pressed(Key::Menu)) else {
            panic!("save did not emit a permission record");
        };
        assert_eq!(ui.view(), UiView::PermissionsSaved { generation: 8 });

        assert!(live_policy.authorise(TxClass::LicenceFreePlan).is_err());
        let stored = StoredPermissions::decode(&saved).unwrap();
        assert_eq!(stored.generation(), 8);
        assert!(stored.permissions().allows(TxClass::LicenceFreePlan));
        assert!(!stored.permissions().allows(TxClass::Never));
        let (rebooted_policy, reboot_status) = TxPolicy::load(&saved);
        assert_eq!(reboot_status, LoadStatus::Valid);
        assert!(rebooted_policy.authorise(TxClass::LicenceFreePlan).is_ok());
        assert!(rebooted_policy.authorise(TxClass::Amateur).is_err());
        assert!(rebooted_policy.authorise(TxClass::Never).is_err());
    }

    #[test]
    fn corrupt_persistence_defaults_the_editor_and_policy_to_denied() {
        let permissions = PermissionSet::none().with(TxClass::Amateur, true);
        let mut corrupt = StoredPermissions::new(permissions, 5).encode();
        corrupt[1] ^= 1;
        let (policy, policy_status) = TxPolicy::load(&corrupt);
        let (mut ui, ui_status) = BootUi::boot(&corrupt, KeySet::permission_menu_gesture());
        assert!(matches!(policy_status, LoadStatus::DefaultedDenied(_)));
        assert_eq!(ui_status, policy_status);
        assert!(policy.authorise(TxClass::Amateur).is_err());
        release_boot_gesture(&mut ui);
        assert_eq!(
            ui.view(),
            UiView::PermissionMenu {
                selected: TxClass::LicenceFreePlan,
                enabled: false,
                changed: false,
                error: None,
            }
        );
    }

    #[test]
    fn exhausted_generation_refuses_to_emit_persistence() {
        let record = denied_record(u32::MAX);
        let (mut ui, _) = BootUi::boot(&record, KeySet::permission_menu_gesture());
        release_boot_gesture(&mut ui);
        assert_eq!(
            ui.handle(KeyEvent::pressed(Key::Menu)),
            UiAction::SaveRefused(MenuError::GenerationExhausted)
        );
        assert_eq!(
            ui.view(),
            UiView::PermissionMenu {
                selected: TxClass::LicenceFreePlan,
                enabled: false,
                changed: false,
                error: Some(MenuError::GenerationExhausted),
            }
        );
    }
}

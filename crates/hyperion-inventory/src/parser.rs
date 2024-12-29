/* use snafu::{ResultExt, Snafu, ensure};

use super::action::{FullMouseButton, InventoryAction, MouseButton};

#[derive(Debug, Snafu)]
#[allow(dead_code)]
pub enum Error {
    #[snafu(display("Invalid mode: {}", mode))]
    InvalidMode {
        mode: u8,
    },

    #[snafu(display("Invalid button for mode {}: {}", mode, button))]
    InvalidButton {
        mode: u8,
        button: u8,
    },

    #[snafu(display("Invalid slot for mode {} and button {}: {}", mode, button, slot))]
    InvalidSlot {
        mode: u8,
        button: u8,
        slot: i16,
    },

    NegativeSlot {
        source: std::num::TryFromIntError,
    },

    #[snafu(display("Invalid number key: {}", key))]
    InvalidNumberKey {
        key: u8,
    },
}

pub type InventoryActionResult = Result<InventoryAction, Error>;

fn handle_normal_click(button: u8, slot: i16) -> InventoryActionResult {
    ensure!(button <= 1, InvalidButtonSnafu { mode: 0, button });
    ensure!(slot != -999, InvalidSlotSnafu {
        mode: 0,
        button,
        slot
    });

    let slot = u16::try_from(slot).context(NegativeSlotSnafu)?;

    Ok(InventoryAction::NormalClick {
        button: if button == 0 {
            MouseButton::Left
        } else {
            MouseButton::Right
        },
        slot,
    })
}

fn handle_outside_click(button: u8) -> InventoryActionResult {
    ensure!(button <= 1, InvalidButtonSnafu { mode: 0, button });
    Ok(InventoryAction::OutsideClick {
        button: if button == 0 {
            MouseButton::Left
        } else {
            MouseButton::Right
        },
    })
}

fn handle_shift_click(button: u8, slot: u16) -> InventoryActionResult {
    ensure!(button <= 1, InvalidButtonSnafu { mode: 1, button });
    Ok(InventoryAction::ShiftClick {
        button: if button == 0 {
            MouseButton::Left
        } else {
            MouseButton::Right
        },
        slot,
    })
}

fn handle_number_key(button: u8, slot: u16) -> InventoryActionResult {
    let key = button + 1;
    ensure!(button <= 8, InvalidNumberKeySnafu { key });
    Ok(InventoryAction::NumberKey { key, slot })
}

fn handle_drag(button: u8, slot: i16) -> InventoryActionResult {
    match (button, slot) {
        (0 | 4 | 8, -999) => Ok(InventoryAction::DragStart {
            button: match button {
                0 => FullMouseButton::Left,
                4 => FullMouseButton::Right,
                _ => FullMouseButton::Middle,
            },
        }),
        (1 | 5 | 9, slot) if slot != -999 => Ok(InventoryAction::DragAdd {
            button: match button {
                1 => FullMouseButton::Left,
                5 => FullMouseButton::Right,
                _ => FullMouseButton::Middle,
            },
            slot: slot.try_into().context(NegativeSlotSnafu)?,
        }),
        (2 | 6 | 10, -999) => Ok(InventoryAction::DragEnd {
            button: match button {
                2 => FullMouseButton::Left,
                6 => FullMouseButton::Right,
                _ => FullMouseButton::Middle,
            },
        }),
        _ => InvalidSlotSnafu {
            mode: 5,
            button,
            slot,
        }
        .fail(),
    }
}

pub fn create_inventory_action(mode: u8, button: u8, slot: i16) -> InventoryActionResult {
    match mode {
        0 if slot == -999 => handle_outside_click(button),
        0 => handle_normal_click(button, slot),
        1 => handle_shift_click(button, slot.try_into().context(NegativeSlotSnafu)?),
        2 => handle_number_key(button, slot.try_into().context(NegativeSlotSnafu)?),
        3 => match button {
            40 => Ok(InventoryAction::OffhandSwap {
                slot: slot.try_into().context(NegativeSlotSnafu)?,
            }),
            2 => Ok(InventoryAction::MiddleClick {
                slot: slot.try_into().context(NegativeSlotSnafu)?,
            }),
            _ => InvalidButtonSnafu { mode, button }.fail(),
        },
        4 => match button {
            0 => Ok(InventoryAction::Drop),
            1 => Ok(InventoryAction::CtrlDrop),
            _ => InvalidButtonSnafu { mode, button }.fail(),
        },
        5 => handle_drag(button, slot),
        6 => match button {
            0 => Ok(InventoryAction::DoubleClick {
                slot: slot.try_into().context(NegativeSlotSnafu)?,
            }),
            1 => Ok(InventoryAction::PickupAllReverse {
                slot: slot.try_into().context(NegativeSlotSnafu)?,
            }),
            _ => InvalidButtonSnafu { mode, button }.fail(),
        },
        _ => InvalidModeSnafu { mode }.fail(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_normal_click() {
        assert_eq!(
            create_inventory_action(0, 0, 0).unwrap(),
            InventoryAction::NormalClick {
                button: MouseButton::Left,
                slot: 0
            }
        );

        assert_eq!(
            create_inventory_action(0, 1, 0).unwrap(),
            InventoryAction::NormalClick {
                button: MouseButton::Right,
                slot: 0
            }
        );

        assert!(matches!(
            create_inventory_action(0, 2, 0).unwrap_err(),
            Error::InvalidButton { mode: 0, button: 2 }
        ));
    }

    #[test]
    fn test_middle_click() {
        assert_eq!(
            create_inventory_action(3, 2, 0).unwrap(),
            InventoryAction::MiddleClick { slot: 0 }
        );
    }

    #[test]
    fn test_drag() {
        assert_eq!(
            create_inventory_action(5, 8, -999).unwrap(),
            InventoryAction::DragStart {
                button: FullMouseButton::Middle
            }
        );

        assert_eq!(
            create_inventory_action(5, 9, 0).unwrap(),
            InventoryAction::DragAdd {
                button: FullMouseButton::Middle,
                slot: 0
            }
        );
    }

    #[test]
    fn test_pickup_all_reverse() {
        assert_eq!(
            create_inventory_action(6, 1, 0).unwrap(),
            InventoryAction::PickupAllReverse { slot: 0 }
        );
    }

    #[test]
    fn test_invalid_mode() {
        assert!(matches!(
            create_inventory_action(7, 0, 0).unwrap_err(),
            Error::InvalidMode { mode: 7 }
        ));
    }
}
 */
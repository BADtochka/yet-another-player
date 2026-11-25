use serde::Deserialize;

pub(super) const CTRL: u8 = 1;
pub(super) const SHIFT: u8 = 2;
pub(super) const ALT: u8 = 4;
pub(super) const META: u8 = 8;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Selection {
    modifiers: u8,
    key: Option<String>,
}

#[derive(Clone, Copy)]
pub(super) struct Binding {
    pub modifiers: u8,
    pub trigger: Option<Trigger>,
}

#[derive(Clone, Copy)]
pub(super) enum Trigger {
    Keyboard(Key),
    Mouse(u8),
}

#[derive(Clone, Copy)]
pub(super) struct Key {
    #[cfg(target_os = "macos")]
    pub mac: u16,
    #[cfg(target_os = "windows")]
    pub scan: u16,
    #[cfg(target_os = "linux")]
    pub x11: u8,
}

fn key(_mac: u16, _scan: u8, _extended: bool) -> Trigger {
    Trigger::Keyboard(Key {
        #[cfg(target_os = "macos")]
        mac: _mac,
        #[cfg(target_os = "windows")]
        scan: u16::from(_scan) | if _extended { 0xe000 } else { 0 },
        #[cfg(target_os = "linux")]
        x11: if _extended {
            match _scan {
                0x47 => 110, // Home
                0x48 => 111,
                0x49 => 112,
                0x4b => 113,
                0x4d => 114,
                0x4f => 115,
                0x50 => 116,
                0x51 => 117,
                0x53 => 119,
                _ => 0,
            }
        } else {
            _scan + 8
        },
    })
}

fn trigger(code: &str) -> Option<Trigger> {
    const LETTERS: [(u16, u8); 26] = [
        (0, 30),
        (11, 48),
        (8, 46),
        (2, 32),
        (14, 18),
        (3, 33),
        (5, 34),
        (4, 35),
        (34, 23),
        (38, 36),
        (40, 37),
        (37, 38),
        (46, 50),
        (45, 49),
        (31, 24),
        (35, 25),
        (12, 16),
        (15, 19),
        (1, 31),
        (17, 20),
        (32, 22),
        (9, 47),
        (13, 17),
        (7, 45),
        (16, 21),
        (6, 44),
    ];
    const DIGITS: [(u16, u8); 10] = [
        (29, 11),
        (18, 2),
        (19, 3),
        (20, 4),
        (21, 5),
        (23, 6),
        (22, 7),
        (26, 8),
        (28, 9),
        (25, 10),
    ];
    const FUNCTIONS: [u16; 12] = [122, 120, 99, 118, 96, 97, 98, 100, 101, 109, 103, 111];
    match code.as_bytes() {
        [b'K', b'e', b'y', letter @ b'A'..=b'Z'] => {
            let (mac, scan) = LETTERS[usize::from(letter - b'A')];
            return Some(key(mac, scan, false));
        }
        [b'D', b'i', b'g', b'i', b't', digit @ b'0'..=b'9'] => {
            let (mac, scan) = DIGITS[usize::from(digit - b'0')];
            return Some(key(mac, scan, false));
        }
        _ => {}
    }
    if let Some(number) = code
        .strip_prefix('F')
        .and_then(|value| value.parse::<u8>().ok())
    {
        if (1..=12).contains(&number) {
            let scan = if number <= 10 {
                58 + number
            } else {
                76 + number
            };
            return Some(key(FUNCTIONS[usize::from(number - 1)], scan, false));
        }
    }
    Some(match code {
        "Space" => key(49, 0x39, false),
        "Enter" => key(36, 0x1c, false),
        "Tab" => key(48, 0x0f, false),
        "Backspace" => key(51, 0x0e, false),
        "Delete" => key(117, 0x53, true),
        "Home" => key(115, 0x47, true),
        "End" => key(119, 0x4f, true),
        "PageUp" => key(116, 0x49, true),
        "PageDown" => key(121, 0x51, true),
        "ArrowUp" => key(126, 0x48, true),
        "ArrowDown" => key(125, 0x50, true),
        "ArrowLeft" => key(123, 0x4b, true),
        "ArrowRight" => key(124, 0x4d, true),
        "Backquote" => key(50, 0x29, false),
        "Minus" => key(27, 0x0c, false),
        "Equal" => key(24, 0x0d, false),
        "BracketLeft" => key(33, 0x1a, false),
        "BracketRight" => key(30, 0x1b, false),
        "Backslash" => key(42, 0x2b, false),
        "Semicolon" => key(41, 0x27, false),
        "Quote" => key(39, 0x28, false),
        "Comma" => key(43, 0x33, false),
        "Period" => key(47, 0x34, false),
        "Slash" => key(44, 0x35, false),
        "MouseMiddle" => Trigger::Mouse(2),
        "MouseBack" => Trigger::Mouse(3),
        "MouseForward" => Trigger::Mouse(4),
        _ => return None,
    })
}

impl TryFrom<Selection> for Binding {
    type Error = &'static str;

    fn try_from(selection: Selection) -> Result<Self, Self::Error> {
        let Selection { modifiers, key } = selection;
        if modifiers & !(CTRL | SHIFT | ALT | META) != 0 || modifiers == 0 && key.is_none() {
            return Err("Назначьте клавишу, сочетание или кнопку мыши.");
        }
        let trigger = key
            .as_deref()
            .map(|code| trigger(code).ok_or("Эта клавиша не поддерживается."))
            .transpose()?;
        Ok(Self { modifiers, trigger })
    }
}

pub(super) fn wheel_change(delta: f64, units: f64) -> f64 {
    (delta * units).clamp(-0.1, 0.1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn binding_accepts_modifiers_letters_and_mouse_buttons() {
        for (modifiers, key) in [
            (CTRL | SHIFT, None),
            (0, Some("KeyY")),
            (ALT, Some("MouseBack")),
            (0, Some("F12")),
        ] {
            assert!(Binding::try_from(Selection {
                modifiers,
                key: key.map(str::to_owned)
            })
            .is_ok());
        }
    }

    #[test]
    fn binding_rejects_empty_unknown_and_invalid_modifiers() {
        for (modifiers, key) in [(0, None), (ALT, Some("Unidentified")), (16, Some("Space"))] {
            assert!(Binding::try_from(Selection {
                modifiers,
                key: key.map(str::to_owned)
            })
            .is_err());
        }
    }

    #[test]
    fn wheel_deltas_match_the_player_slider_and_are_limited_per_event() {
        assert_eq!(wheel_change(1.0, 0.025), 0.025);
        assert_eq!(wheel_change(-24.0, 0.001), -0.024);
        assert_eq!(wheel_change(500.0, 0.001), 0.1);
    }
}

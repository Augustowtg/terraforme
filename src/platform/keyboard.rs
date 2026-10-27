//! Estado do teclado. O Caps Lock não é informado pelo terminal, então a
//! leitura é feita direto no sistema: no Linux, pelo LED do teclado em
//! `/sys/class/leds`; no Windows, pela API do teclado.

/// `Some(true)` com o Caps Lock ligado; `None` quando não dá para saber (ex.:
/// via SSH, sem acesso ao teclado local).
#[cfg(target_os = "linux")]
pub fn caps_lock() -> Option<bool> {
    let mut found = false;
    for entry in std::fs::read_dir("/sys/class/leds").ok()?.flatten() {
        if !entry.file_name().to_string_lossy().ends_with("::capslock") {
            continue;
        }
        let Ok(brightness) = std::fs::read_to_string(entry.path().join("brightness")) else {
            continue;
        };
        found = true;
        // Com mais de um teclado, basta um com o LED aceso.
        if brightness.trim() != "0" {
            return Some(true);
        }
    }
    found.then_some(false)
}

#[cfg(windows)]
pub fn caps_lock() -> Option<bool> {
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{GetKeyState, VK_CAPITAL};
    // O bit menos significativo indica a tecla "travada" (ligada).
    let state = unsafe { GetKeyState(VK_CAPITAL as i32) };
    Some(state & 1 != 0)
}

#[cfg(not(any(target_os = "linux", windows)))]
pub fn caps_lock() -> Option<bool> {
    None
}

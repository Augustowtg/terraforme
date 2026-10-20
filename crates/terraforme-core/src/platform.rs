//! O que muda entre Linux e Windows no controle de processos: PID vivo,
//! `Ctrl+C` no processo pai e processos que sobrevivem ao terminal.

use std::process::Command;

/// `true` se o processo `pid` ainda existe.
#[cfg(unix)]
pub fn is_pid_alive(pid: u32) -> bool {
    std::path::Path::new("/proc").join(pid.to_string()).exists()
}

/// `true` se o processo `pid` ainda existe.
#[cfg(windows)]
pub fn is_pid_alive(pid: u32) -> bool {
    use windows_sys::Win32::Foundation::{CloseHandle, STILL_ACTIVE};
    use windows_sys::Win32::System::Threading::{
        GetExitCodeProcess, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
    };

    // SAFETY: o handle vem do OpenProcess, é checado e fechado aqui mesmo.
    unsafe {
        let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if handle.is_null() {
            return false;
        }
        let mut code = 0u32;
        let ok = GetExitCodeProcess(handle, &mut code) != 0;
        CloseHandle(handle);
        ok && code == STILL_ACTIVE as u32
    }
}

/// Sem como checar: considera vivo.
#[cfg(not(any(unix, windows)))]
pub fn is_pid_alive(_pid: u32) -> bool {
    true
}

/// Faz o processo pai ignorar o `Ctrl+C` enquanto espera um filho que ficou
/// com o console. No Windows o `Ctrl+C` vai para todos os processos do
/// console: sem isso o terraforme morre e o filho continua lendo o teclado
/// junto com o shell de onde ele foi chamado. O filho não herda o handler.
pub fn ignore_ctrl_c() {
    #[cfg(windows)]
    {
        use windows_sys::Win32::System::Console::SetConsoleCtrlHandler;

        unsafe extern "system" fn handled(_ctrl_type: u32) -> windows_sys::core::BOOL {
            1
        }
        // SAFETY: registra uma função `extern "system"` sem estado.
        unsafe {
            SetConsoleCtrlHandler(Some(handled), 1);
        }
    }
}

/// Solta o processo do terminal de quem o iniciou, para ele sobreviver quando
/// essa janela fecha: sem console próprio no Windows e num grupo de processos
/// novo no Unix (o SIGHUP do terminal vai para o grupo do shell).
pub fn detach(command: &mut Command) {
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        use windows_sys::Win32::System::Threading::{CREATE_NEW_PROCESS_GROUP, DETACHED_PROCESS};
        command.creation_flags(DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP);
    }
    #[cfg(not(any(unix, windows)))]
    let _ = command;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn own_pid_is_alive() {
        assert!(is_pid_alive(std::process::id()));
    }

    #[test]
    fn finished_child_is_not_alive() {
        let mut child = if cfg!(windows) {
            Command::new("cmd").args(["/C", "exit"]).spawn().unwrap()
        } else {
            Command::new("true").spawn().unwrap()
        };
        let pid = child.id();
        child.wait().unwrap();
        assert!(!is_pid_alive(pid));
    }
}

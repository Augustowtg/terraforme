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

/// Roda o processo sem abrir uma janela de console no Windows (ex.: o
/// `powershell` chamado pela janela flutuante, que não tem console).
pub fn no_window(command: &mut Command) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        use windows_sys::Win32::System::Threading::CREATE_NO_WINDOW;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    #[cfg(not(windows))]
    let _ = command;
}

/// Pede ao processo `pid` para sair (`kill`/`SIGTERM` no Unix; `taskkill`
/// no Windows, que fecha a janela de um programa gráfico como um clique no
/// X e só força quando ele não sai). `true` se o pedido foi aceito.
pub fn kill(pid: u32) -> std::io::Result<bool> {
    use std::process::Stdio;

    let run = |program: &str, args: &[&str]| {
        let mut command = Command::new(program);
        command
            .args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        no_window(&mut command);
        command.status().map(|s| s.success())
    };
    let pid = pid.to_string();
    if cfg!(windows) {
        Ok(run("taskkill", &["/PID", &pid])? || run("taskkill", &["/F", "/PID", &pid])?)
    } else {
        run("kill", &[&pid])
    }
}

/// Deixa qualquer processo trazer a própria janela para frente logo depois
/// (o Windows só deixa quem recebeu o último clique fazer isso): a janela
/// flutuante chama antes de pedir ao terminal para vir para frente
/// ([`raise_terminal`]). Nada nos outros sistemas.
pub fn allow_foreground() {
    #[cfg(windows)]
    // SAFETY: chamada sem ponteiros.
    unsafe {
        use windows_sys::Win32::UI::WindowsAndMessaging::{ASFW_ANY, AllowSetForegroundWindow};
        AllowSetForegroundWindow(ASFW_ANY);
    }
}

/// Encerra todos os processos do usuário com o executável `name` (sem
/// extensão), menos este, e espera até 3s que saiam. Devolve quantos havia.
/// No Linux procura em `/proc/<pid>/comm` e manda `SIGTERM` (`kill`); no
/// Windows usa `tasklist`/`taskkill`.
pub fn kill_all(name: &str) -> std::io::Result<usize> {
    use std::process::Stdio;
    use std::time::{Duration, Instant};

    let me = std::process::id();
    let pids: Vec<u32> = {
        #[cfg(windows)]
        {
            let out = Command::new("tasklist")
                .args(["/FO", "CSV", "/NH", "/FI"])
                .arg(format!("IMAGENAME eq {name}.exe"))
                .stderr(Stdio::null())
                .output()?;
            // `"terraforme-mcp.exe","1234",...`
            String::from_utf8_lossy(&out.stdout)
                .lines()
                .filter_map(|line| line.split(',').nth(1)?.trim_matches('"').parse().ok())
                .filter(|&pid| pid != me)
                .collect()
        }
        #[cfg(not(windows))]
        {
            std::fs::read_dir("/proc")?
                .flatten()
                .filter_map(|entry| entry.file_name().to_str()?.parse::<u32>().ok())
                .filter(|&pid| pid != me)
                .filter(|pid| {
                    std::fs::read_to_string(format!("/proc/{pid}/comm"))
                        .is_ok_and(|comm| comm.trim_end() == name)
                })
                .collect()
        }
    };
    if pids.is_empty() {
        return Ok(0);
    }
    let mut command = if cfg!(windows) {
        let mut command = Command::new("taskkill");
        command.arg("/F");
        for pid in &pids {
            command.arg("/PID").arg(pid.to_string());
        }
        command
    } else {
        let mut command = Command::new("kill");
        command.args(pids.iter().map(u32::to_string));
        command
    };
    command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()?;
    let deadline = Instant::now() + Duration::from_secs(3);
    while pids.iter().any(|&pid| is_pid_alive(pid)) && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(100));
    }
    Ok(pids.len())
}

/// Abre o endereço no navegador padrão (`xdg-open`, `open` no macOS,
/// `explorer` no Windows), sem esperar por ele.
pub fn open_url(url: &str) -> std::io::Result<()> {
    use std::process::Stdio;

    let opener = if cfg!(windows) {
        "explorer"
    } else if cfg!(target_os = "macos") {
        "open"
    } else {
        "xdg-open"
    };
    let mut command = Command::new(opener);
    command
        .arg(url)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    detach(&mut command);
    command.spawn().map(drop)
}

/// Traz para frente a janela do terminal em que este processo roda (a
/// janela flutuante pediu para ir a uma instância). No GNOME, pela extensão
/// `terraforme@augustowtg.github.io` (`assets/gnome-shell`, instalada pelo
/// `install.linux.sh`), que ativa a janela do primeiro processo pai que tiver
/// uma, trocando de área de trabalho e saindo do minimizado — único jeito no
/// Wayland, onde um app não pode ativar a janela de outro. Sem ela, só no X11,
/// com `xdotool` ou `wmctrl`: a janela de `$WINDOWID` ou, sem ela, a do
/// primeiro processo pai que tiver uma. No Windows, a janela dona do console
/// (a do Windows Terminal; no console clássico, o próprio console) ou, sem
/// ela, a do primeiro processo pai que tiver uma (ex.: o VS Code), com
/// `SetForegroundWindow` — funciona porque a janela flutuante chama
/// [`allow_foreground`] antes. Devolve `false` quando não conseguiu.
pub fn raise_terminal() -> bool {
    #[cfg(target_os = "linux")]
    {
        use std::process::Stdio;

        let quiet = |bin: &str, args: &[&str]| {
            crate::status::find_in_path(bin).and_then(|path| {
                Command::new(path)
                    .args(args)
                    .stdin(Stdio::null())
                    .stderr(Stdio::null())
                    .output()
                    .ok()
                    .filter(|out| out.status.success())
            })
        };
        let pids = ancestors();
        if !pids.is_empty() {
            let list = pids.iter().map(u32::to_string).collect::<Vec<_>>();
            let found = quiet(
                "gdbus",
                &[
                    "call",
                    "--session",
                    "--dest",
                    "org.gnome.Shell",
                    "--object-path",
                    "/org/gnome/Shell/Extensions/Terraforme",
                    "--method",
                    "org.gnome.Shell.Extensions.Terraforme.Activate",
                    &format!("[uint32 {}]", list.join(", ")),
                ],
            );
            // Resposta `(true,)`; sem a extensão o `gdbus` falha.
            if found.is_some_and(|out| String::from_utf8_lossy(&out.stdout).contains("true")) {
                return true;
            }
        }
        let wayland = std::env::var("XDG_SESSION_TYPE").is_ok_and(|t| t == "wayland");
        if wayland || std::env::var_os("DISPLAY").is_none_or(|d| d.is_empty()) {
            return false;
        }
        let window = std::env::var("WINDOWID")
            .ok()
            .filter(|id| !id.is_empty())
            .or_else(|| {
                pids.iter().find_map(|pid| {
                    let out = quiet("xdotool", &["search", "--pid", &pid.to_string()])?;
                    let text = String::from_utf8_lossy(&out.stdout);
                    text.lines().next().map(str::to_string)
                })
            });
        let Some(window) = window else {
            return false;
        };
        quiet("xdotool", &["windowactivate", &window]).is_some()
            || quiet("wmctrl", &["-i", "-a", &window]).is_some()
    }
    #[cfg(windows)]
    {
        windows::raise_terminal()
    }
    #[cfg(not(any(target_os = "linux", windows)))]
    false
}

#[cfg(windows)]
mod windows {
    use windows_sys::Win32::Foundation::{CloseHandle, HWND, INVALID_HANDLE_VALUE, LPARAM};
    use windows_sys::Win32::System::Console::GetConsoleWindow;
    use windows_sys::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW,
        TH32CS_SNAPPROCESS,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        EnumWindows, GW_OWNER, GetWindow, GetWindowThreadProcessId, IsIconic, IsWindowVisible,
        SW_RESTORE, SetForegroundWindow, ShowWindow,
    };

    pub fn raise_terminal() -> bool {
        // SAFETY: só lê handles de janela devolvidos pelo próprio sistema.
        let console = unsafe { GetConsoleWindow() };
        let window = (!console.is_null())
            .then(|| {
                // No Windows Terminal o console é uma janela escondida cuja
                // dona é a janela do terminal.
                let owner = unsafe { GetWindow(console, GW_OWNER) };
                if owner.is_null() { console } else { owner }
            })
            .filter(|&w| unsafe { IsWindowVisible(w) } != 0)
            .or_else(|| ancestors().into_iter().find_map(window_of));
        let Some(window) = window else {
            return false;
        };
        // SAFETY: `window` é um handle de janela válido de `EnumWindows`/`GetWindow`.
        unsafe {
            if IsIconic(window) != 0 {
                ShowWindow(window, SW_RESTORE);
            }
            SetForegroundWindow(window) != 0
        }
    }

    /// PIDs dos processos pais deste, do mais próximo ao mais distante.
    fn ancestors() -> Vec<u32> {
        let mut parent = std::collections::HashMap::new();
        // SAFETY: o snapshot é checado e fechado aqui; `entry` tem o `dwSize` certo.
        unsafe {
            let snapshot = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
            if snapshot == INVALID_HANDLE_VALUE {
                return Vec::new();
            }
            let mut entry: PROCESSENTRY32W = std::mem::zeroed();
            entry.dwSize = size_of::<PROCESSENTRY32W>() as u32;
            let mut ok = Process32FirstW(snapshot, &mut entry) != 0;
            while ok {
                parent.insert(entry.th32ProcessID, entry.th32ParentProcessID);
                ok = Process32NextW(snapshot, &mut entry) != 0;
            }
            CloseHandle(snapshot);
        }
        let mut list = Vec::new();
        let mut pid = std::process::id();
        // O PID de um pai que já saiu pode ter sido reusado: o limite evita ciclos.
        while let Some(&ppid) = parent.get(&pid) {
            if ppid == 0 || list.len() >= 16 || list.contains(&ppid) {
                break;
            }
            list.push(ppid);
            pid = ppid;
        }
        list
    }

    /// A primeira janela principal visível do processo `pid`.
    fn window_of(pid: u32) -> Option<HWND> {
        struct Search {
            pid: u32,
            found: HWND,
        }
        unsafe extern "system" fn each(window: HWND, data: LPARAM) -> windows_sys::core::BOOL {
            // SAFETY: `data` é o `&mut Search` passado a `EnumWindows` abaixo.
            let search = unsafe { &mut *(data as *mut Search) };
            let mut owner = 0u32;
            unsafe { GetWindowThreadProcessId(window, &mut owner) };
            let main = unsafe { GetWindow(window, GW_OWNER) }.is_null();
            if owner == search.pid && main && unsafe { IsWindowVisible(window) } != 0 {
                search.found = window;
                return 0;
            }
            1
        }
        let mut search = Search {
            pid,
            found: std::ptr::null_mut(),
        };
        // SAFETY: `search` vive até o fim da enumeração, que é síncrona.
        unsafe { EnumWindows(Some(each), &mut search as *mut Search as LPARAM) };
        (!search.found.is_null()).then_some(search.found)
    }

    #[cfg(test)]
    mod tests {
        #[test]
        fn ancestors_start_at_the_parent() {
            // O `cargo test` sempre tem um pai vivo.
            assert!(!super::ancestors().is_empty());
        }
    }
}

/// PIDs dos processos pais deste, do mais próximo ao mais distante.
#[cfg(target_os = "linux")]
fn ancestors() -> Vec<u32> {
    let mut list = Vec::new();
    let mut pid = std::process::id();
    while list.len() < 16 {
        let Ok(stat) = std::fs::read_to_string(format!("/proc/{pid}/stat")) else {
            break;
        };
        // `pid (comando) estado ppid …`: o comando pode ter espaços e parênteses.
        let Some(ppid) = stat
            .rsplit_once(')')
            .and_then(|(_, rest)| rest.split_whitespace().nth(1))
            .and_then(|p| p.parse::<u32>().ok())
        else {
            break;
        };
        if ppid <= 1 {
            break;
        }
        list.push(ppid);
        pid = ppid;
    }
    list
}

#[cfg(test)]
mod tests {
    #[cfg(target_os = "linux")]
    #[test]
    fn ancestors_start_at_the_parent() {
        let parent = std::os::unix::process::parent_id();
        if parent > 1 {
            assert_eq!(super::ancestors().first(), Some(&parent));
        }
    }

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

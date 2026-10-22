//! Janela flutuante com o mascote (`terraforme pet`): o estado de atividade
//! de cada instância do agente, detectado pela TUI ([`Tracker`]) e enviado ao
//! daemon MCP, e o cliente que a janela usa para ler esse estado e focar uma
//! instância. A janela existe no Linux e no Windows (o macOS está no
//! `ROADMAP.md`).

use std::fs;
use std::path::PathBuf;
use std::process::{self, Stdio};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

use crate::mcp::LockFile;

/// Ignora a saída logo depois de abrir a instância (a tela inicial do agente).
const BOOT: Duration = Duration::from_secs(5);
/// Saída até este tempo depois de uma tecla é eco da digitação, não trabalho.
const ECHO: Duration = Duration::from_millis(1500);
/// Saída contínua por este tempo marca a instância como trabalhando.
const WORK_MIN: Duration = Duration::from_secs(2);
/// Silêncio depois de trabalhar que marca a instância como terminada.
const QUIET: Duration = Duration::from_secs(4);
/// Binário da janela, instalado como plugin (`terraforme plugin install pet`).
pub const BIN: &str = "terraforme-pet";
/// Tempo de espera pelas respostas do daemon.
const TIMEOUT: Duration = Duration::from_secs(2);

/// O que a instância está fazendo, do ponto de vista da janela flutuante.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Activity {
    /// Parada, sem aviso pendente.
    #[default]
    Idle,
    /// Gerando saída (o spinner do agente redesenha a tela).
    Working,
    /// Terminou de trabalhar e o usuário ainda não olhou.
    Done,
    /// Pediu atenção (sino ou notificação OSC 9/777 do agente).
    Waiting,
}

impl Activity {
    /// Ícone usado no menu da janela.
    pub fn icon(self) -> &'static str {
        match self {
            Activity::Idle => "○",
            Activity::Working => "●",
            Activity::Done => "✔",
            Activity::Waiting => "!",
        }
    }

    /// Precisa que o usuário olhe a instância.
    pub fn pending(self) -> bool {
        matches!(self, Activity::Done | Activity::Waiting)
    }
}

/// Detecta a atividade de uma instância pela saída do pseudoterminal e pelo
/// que o usuário digita. Cada método devolve o novo estado quando ele muda.
#[derive(Debug)]
pub struct Tracker {
    started: Instant,
    state: Activity,
    last_input: Option<Instant>,
    last_output: Option<Instant>,
    /// Início da saída contínua atual.
    busy_since: Option<Instant>,
}

impl Tracker {
    /// Cria um rastreador iniciado no instante `now`.
    pub fn new(now: Instant) -> Self {
        Self {
            started: now,
            state: Activity::Idle,
            last_input: None,
            last_output: None,
            busy_since: None,
        }
    }

    /// Atividade atual da instância.
    pub fn state(&self) -> Activity {
        self.state
    }

    fn set(&mut self, state: Activity) -> Option<Activity> {
        (self.state != state).then(|| {
            self.state = state;
            state
        })
    }

    /// O agente escreveu na tela.
    pub fn output(&mut self, now: Instant) -> Option<Activity> {
        if now.duration_since(self.started) < BOOT
            || self
                .last_input
                .is_some_and(|at| now.duration_since(at) < ECHO)
        {
            return None;
        }
        // Saída depois de um silêncio começa um período novo.
        if self
            .last_output
            .is_none_or(|at| now.duration_since(at) >= QUIET)
        {
            self.busy_since = Some(now);
        }
        self.last_output = Some(now);
        let busy = self
            .busy_since
            .is_some_and(|at| now.duration_since(at) >= WORK_MIN);
        if busy && self.state != Activity::Waiting {
            return self.set(Activity::Working);
        }
        None
    }

    /// O usuário digitou na instância: viu o aviso, se havia um.
    pub fn input(&mut self, now: Instant) -> Option<Activity> {
        self.last_input = Some(now);
        self.busy_since = None;
        self.seen()
    }

    /// O usuário abriu a instância.
    pub fn seen(&mut self) -> Option<Activity> {
        if self.state.pending() {
            return self.set(Activity::Idle);
        }
        None
    }

    /// O agente tocou o sino ou mandou uma notificação.
    pub fn bell(&mut self, now: Instant) -> Option<Activity> {
        if now.duration_since(self.started) < BOOT {
            return None;
        }
        self.set(Activity::Waiting)
    }

    /// Chamado periodicamente: o silêncio depois de trabalhar vira "terminou".
    pub fn tick(&mut self, now: Instant) -> Option<Activity> {
        let quiet = self
            .last_output
            .is_none_or(|at| now.duration_since(at) >= QUIET);
        if !quiet {
            return None;
        }
        self.busy_since = None;
        if self.state == Activity::Working {
            return self.set(Activity::Done);
        }
        None
    }
}

/// Instância como o daemon a devolve em `GET /pet/state`.
#[derive(Debug, Clone, Deserialize)]
pub struct Instance {
    pub id: String,
    /// Apelido único (`space/claude-1`).
    pub handle: String,
    pub agent: String,
    /// Vazio fora de um space.
    #[serde(default)]
    pub space: String,
    #[serde(default)]
    pub cwd: String,
    /// Número da sub-aba no terminal (`claude 2` → 2).
    #[serde(default)]
    pub number: usize,
    #[serde(default)]
    pub activity: Activity,
    /// Segundos desde a época Unix da última mudança de `activity`.
    #[serde(default)]
    pub changed: u64,
}

impl Instance {
    /// Nome mostrado no menu: `space · claude 2` (a pasta fora de space).
    pub fn label(&self) -> String {
        let place = if self.space.is_empty() {
            std::path::Path::new(&self.cwd)
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| self.handle.clone())
        } else {
            self.space.clone()
        };
        format!("{place} · {} {}", self.agent, self.number)
    }
}

fn agent() -> ureq::Agent {
    ureq::Agent::config_builder()
        .timeout_global(Some(TIMEOUT))
        .build()
        .into()
}

/// Instâncias de todos os terminais; vazio quando o daemon não está rodando.
pub fn instances() -> Result<Vec<Instance>> {
    let Some(lock) = LockFile::read()? else {
        return Ok(Vec::new());
    };
    #[derive(Deserialize)]
    struct State {
        instances: Vec<Instance>,
    }
    let body = agent()
        .get(&format!("http://127.0.0.1:{}/pet/state", lock.port))
        .call()
        .context("consultando o daemon MCP")?
        .body_mut()
        .read_to_string()?;
    let state: State = serde_json::from_str(&body).context("resposta inválida do daemon")?;
    Ok(state.instances)
}

/// Pede ao terminal da instância para abrir a sub-aba dela (e vir para frente).
pub fn focus(instance_id: &str) -> Result<()> {
    let Some(lock) = LockFile::read()? else {
        bail!("o daemon MCP não está rodando");
    };
    // No Windows, só quem recebeu o clique pode passar o foco a outra janela.
    crate::platform::allow_foreground();
    let body = serde_json::json!({ "instance_id": instance_id }).to_string();
    agent()
        .post(&format!("http://127.0.0.1:{}/pet/focus", lock.port))
        .header("Content-Type", "application/json")
        .send(body.as_bytes())
        .context("pedindo o foco ao daemon MCP")?;
    Ok(())
}

/// Mostra uma notificação do Windows (toast) com o título `terraforme` e o
/// texto de `TERRAFORME_NOTIFY` (variável, para o texto nunca virar código).
/// Usa o AppUserModelID do PowerShell, que já vem registrado no sistema.
const TOAST_PS1: &str = r#"$ErrorActionPreference = 'Stop'
$m = [Windows.UI.Notifications.ToastNotificationManager, Windows.UI.Notifications, ContentType = WindowsRuntime]
$xml = $m::GetTemplateContent([Windows.UI.Notifications.ToastTemplateType]::ToastText02)
$text = $xml.GetElementsByTagName('text')
$text.Item(0).AppendChild($xml.CreateTextNode('terraforme')) > $null
$text.Item(1).AppendChild($xml.CreateTextNode($env:TERRAFORME_NOTIFY)) > $null
$toast = [Windows.UI.Notifications.ToastNotification]::new($xml)
$m::CreateToastNotifier('{1AC14E77-02E7-4E5D-B744-2EB1AE5198B7}\WindowsPowerShell\v1.0\powershell.exe').Show($toast)
"#;

/// Notificação do sistema: `notify-send` (se instalado) no Linux e toast no
/// Windows (sem esperar o PowerShell, que leva um instante para abrir).
pub fn notify(text: &str) {
    if cfg!(windows) {
        let mut cmd = process::Command::new("powershell");
        cmd.args([
            "-NoProfile",
            "-NonInteractive",
            "-ExecutionPolicy",
            "Bypass",
            "-Command",
            TOAST_PS1,
        ])
        .env("TERRAFORME_NOTIFY", text)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
        crate::platform::no_window(&mut cmd);
        let _ = cmd.spawn();
        return;
    }
    if let Some(bin) = crate::status::find_in_path("notify-send") {
        let _ = process::Command::new(bin)
            .args(["--app-name=terraforme", "terraforme", text])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
}

/// Se a janela existe neste sistema.
pub fn is_supported() -> bool {
    cfg!(any(target_os = "linux", windows))
}

/// `<pasta de dados>/terraforme/pet.lock`, com o PID da janela aberta.
pub fn lock_path() -> Result<PathBuf> {
    Ok(LockFile::path()?.with_file_name("pet.lock"))
}

/// PID da janela aberta, se houver.
pub fn running() -> Option<u32> {
    let pid = fs::read_to_string(lock_path().ok()?)
        .ok()?
        .trim()
        .parse()
        .ok()?;
    crate::platform::is_pid_alive(pid).then_some(pid)
}

/// Marca esta janela como a aberta. `false` se já há outra.
pub fn claim() -> Result<bool> {
    if running().is_some_and(|pid| pid != process::id()) {
        return Ok(false);
    }
    let path = lock_path()?;
    fs::create_dir_all(path.parent().context("caminho inválido")?)?;
    fs::write(&path, process::id().to_string())
        .with_context(|| format!("gravando {}", path.display()))?;
    Ok(true)
}

/// Apaga o lock file se ainda é desta janela.
pub fn release() {
    if running() == Some(process::id())
        && let Ok(path) = lock_path()
    {
        let _ = fs::remove_file(path);
    }
}

/// Abre a janela em segundo plano, se ainda não está aberta. Devolve `false`
/// quando ela já estava.
pub fn start() -> Result<bool> {
    if !is_supported() {
        bail!("a janela flutuante ainda não existe neste sistema — veja o ROADMAP.md");
    }
    if running().is_some() {
        return Ok(false);
    }
    let bin = crate::plugin::locate(BIN)
        .context("o mascote é um plugin — instale com `terraforme plugin install pet`")?;
    let mut cmd = process::Command::new(bin);
    cmd.stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    // Sobrevive ao terminal que a abriu.
    crate::platform::detach(&mut cmd);
    cmd.spawn().context("falha ao abrir terraforme-pet")?;
    Ok(true)
}

/// Fecha a janela aberta, se houver, e espera até 3s que o processo saia (no
/// Windows o `.exe` só pode ser apagado ou trocado depois disso). Devolve
/// `false` quando não havia.
pub fn stop() -> Result<bool> {
    let Some(pid) = running() else {
        return Ok(false);
    };
    if !crate::platform::kill(pid).context("falha ao fechar terraforme-pet")? {
        bail!("falha ao fechar terraforme-pet (pid {pid})");
    }
    let deadline = Instant::now() + Duration::from_secs(3);
    while crate::platform::is_pid_alive(pid) && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(100));
    }
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(start: Instant, ms: u64) -> Instant {
        start + Duration::from_millis(ms)
    }

    /// Saída a cada 100ms entre `from` e `to` (ms); devolve as mudanças.
    fn stream(t: &mut Tracker, start: Instant, from: u64, to: u64) -> Vec<Activity> {
        (from..to)
            .step_by(100)
            .filter_map(|ms| t.output(at(start, ms)))
            .collect()
    }

    #[test]
    fn continuous_output_then_silence_is_done() {
        let start = Instant::now();
        let mut t = Tracker::new(start);
        // A tela inicial não conta.
        assert!(stream(&mut t, start, 0, 3000).is_empty());
        assert_eq!(stream(&mut t, start, 6000, 9000), vec![Activity::Working]);
        assert_eq!(t.tick(at(start, 10_000)), None);
        assert_eq!(t.tick(at(start, 13_000)), Some(Activity::Done));
        // Abrir a instância limpa o aviso.
        assert_eq!(t.seen(), Some(Activity::Idle));
        assert_eq!(t.seen(), None);
    }

    #[test]
    fn typing_echo_and_short_bursts_are_not_work() {
        let start = Instant::now();
        let mut t = Tracker::new(start);
        for ms in (6000..9000).step_by(200) {
            t.input(at(start, ms));
            assert_eq!(t.output(at(start, ms + 50)), None);
        }
        // Rajada curta (ex.: redesenho) não chega a trabalho.
        assert!(stream(&mut t, start, 20_000, 21_000).is_empty());
        assert_eq!(t.tick(at(start, 30_000)), None);
        assert_eq!(t.state(), Activity::Idle);
    }

    #[test]
    fn bell_waits_until_seen_or_typed() {
        let start = Instant::now();
        let mut t = Tracker::new(start);
        assert_eq!(t.bell(at(start, 1000)), None);
        assert_eq!(t.bell(at(start, 6000)), Some(Activity::Waiting));
        // A saída não apaga o pedido de atenção.
        assert!(stream(&mut t, start, 6000, 9000).is_empty());
        assert_eq!(t.input(at(start, 9500)), Some(Activity::Idle));
    }

    #[test]
    fn instance_label_uses_space_or_folder() {
        let mut i: Instance = serde_json::from_value(serde_json::json!({
            "id": "a", "handle": "web/claude-2", "agent": "claude", "space": "web",
            "cwd": "/home/u/web", "number": 2, "activity": "done", "changed": 1
        }))
        .unwrap();
        assert_eq!(i.label(), "web · claude 2");
        assert_eq!(i.activity, Activity::Done);
        i.space.clear();
        i.cwd = "/home/u/Projeto".into();
        assert_eq!(i.label(), "Projeto · claude 2");
    }
}

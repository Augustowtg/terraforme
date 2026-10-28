//! O daemon orquestrador (o hub): um processo só por usuário que acompanha
//! todos os terminais do terra abertos na máquina.
//!
//! Cada `terra cli` é uma **instância**: ao abrir, ela se conecta ao hub
//! ([`client::Reporter`]) e manda o próprio estado quando ele muda (o chat
//! parado ou rodando, o que o agente está fazendo, quantos chats estão
//! abertos). Se o hub não existir, a instância o inicia desacoplado
//! (`terra __hub`). A aba Orquestrador é um **monitor** ([`client::Monitor`]):
//! recebe a lista inteira e depois só o que muda. O `terra status` pega a
//! lista uma vez e sai.
//!
//! O hub fecha sozinho quando o último terminal fecha (depois de
//! [`IDLE_EXIT`] sem nenhuma conexão). O documento completo, com o
//! protocolo, está em `docs/infra/orquestrador.md`.

pub mod client;
pub mod daemon;
pub mod endpoint;
pub mod protocol;
pub mod view;

use std::time::Duration;

/// A versão do protocolo. Um hub e uma instância com versões diferentes não
/// conversam: o mais novo vence (veja [`daemon`]).
pub const PROTO: u64 = 1;
/// A versão do terra, que vai no `hello` e no `hub.lock`.
pub const APP_VERSION: &str = env!("CARGO_PKG_VERSION");
/// De quanto em quanto tempo a instância sem novidades manda o `heartbeat`.
pub const HEARTBEAT: Duration = Duration::from_secs(2);
/// Sem nenhum sinal por esse tempo (3 heartbeats perdidos), a instância
/// fica `stale`: provavelmente travou.
pub const STALE_AFTER: Duration = Duration::from_secs(6);
/// Quanto tempo uma instância `gone` continua na lista.
pub const GONE_KEEP: Duration = Duration::from_secs(30);
/// Sem nenhuma conexão por esse tempo, o hub sai. Curto de propósito: o
/// hub fecha junto com o último terminal, e o intervalo só cobre quem
/// fecha um terra e abre outro em seguida.
pub const IDLE_EXIT: Duration = Duration::from_secs(5);

/// Uma conexão com o hub, que lê de um lado e escreve do outro (`&Stream`
/// lê e escreve; clonar a conexão é só clonar o `Arc`).
#[derive(Clone)]
pub struct Conn(std::sync::Arc<interprocess::local_socket::Stream>);

impl Conn {
    pub fn new(stream: interprocess::local_socket::Stream) -> Self {
        Self(std::sync::Arc::new(stream))
    }

    pub fn set_recv_timeout(&self, timeout: Option<Duration>) -> std::io::Result<()> {
        use interprocess::local_socket::traits::Stream as _;
        self.0.set_recv_timeout(timeout)
    }
}

impl std::io::Read for Conn {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        (&*self.0).read(buf)
    }
}

impl std::io::Write for Conn {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        (&*self.0).write(buf)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        (&*self.0).flush()
    }
}

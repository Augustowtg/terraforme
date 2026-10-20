//! Registro de clientes, watchdog de PIDs e timer de encerramento automático.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use tokio::sync::watch;
use tracing::{info, warn};

use crate::hub::Hub;

pub struct ClientInfo {
    pub pid: u32,
    pub control_addr: String,
    pub last_heartbeat: Instant,
}

#[derive(Clone)]
pub struct ClientRegistry {
    clients: Arc<Mutex<HashMap<String, ClientInfo>>>,
    shutdown_tx: Arc<watch::Sender<bool>>,
    grace_seconds: u64,
    /// Instâncias dos clientes, removidas junto com eles.
    hub: Hub,
}

impl ClientRegistry {
    pub fn new(grace_seconds: u64, shutdown_tx: Arc<watch::Sender<bool>>, hub: Hub) -> Self {
        Self {
            clients: Arc::new(Mutex::new(HashMap::new())),
            shutdown_tx,
            grace_seconds,
            hub,
        }
    }

    pub fn register(&self, client_id: String, pid: u32, control_addr: String) {
        let mut map = self.clients.lock().unwrap();
        info!(client_id, pid, control_addr, "cliente registrado");
        map.insert(
            client_id,
            ClientInfo {
                pid,
                control_addr,
                last_heartbeat: Instant::now(),
            },
        );
    }

    pub fn unregister(&self, client_id: &str) {
        let mut map = self.clients.lock().unwrap();
        if map.remove(client_id).is_some() {
            info!(client_id, "cliente desconectado");
        }
        self.hub.drop_client(client_id);
        let count = map.len();
        drop(map);
        if count == 0 {
            self.schedule_shutdown();
        }
    }

    pub fn heartbeat(&self, client_id: &str) {
        let mut map = self.clients.lock().unwrap();
        if let Some(info) = map.get_mut(client_id) {
            info.last_heartbeat = Instant::now();
        }
    }

    pub fn control_addr(&self, client_id: &str) -> Option<String> {
        self.clients
            .lock()
            .unwrap()
            .get(client_id)
            .map(|i| i.control_addr.clone())
    }

    #[allow(dead_code)]
    pub fn count(&self) -> usize {
        self.clients.lock().unwrap().len()
    }

    fn schedule_shutdown(&self) {
        let tx = self.shutdown_tx.clone();
        let grace = self.grace_seconds;
        let clients = self.clients.clone();
        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_secs(grace)).await;
            if clients.lock().unwrap().is_empty() {
                info!("nenhum cliente após {}s de tolerância — encerrando", grace);
                let _ = tx.send(true);
            }
        });
    }

    /// Tarefa que remove clientes com PID morto a cada 5s.
    pub fn spawn_watchdog(&self) {
        let clients = self.clients.clone();
        let shutdown_tx = self.shutdown_tx.clone();
        let grace = self.grace_seconds;
        let hub = self.hub.clone();
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(std::time::Duration::from_secs(5)).await;
                let dead: Vec<String> = {
                    let map = clients.lock().unwrap();
                    map.iter()
                        .filter(|(_, info)| !terraforme_core::mcp::LockFile::is_pid_alive(info.pid))
                        .map(|(id, _)| id.clone())
                        .collect()
                };
                if dead.is_empty() {
                    continue;
                }
                let mut map = clients.lock().unwrap();
                for id in &dead {
                    warn!(client_id = id, "PID morto — removendo cliente");
                    map.remove(id);
                    hub.drop_client(id);
                }
                let count = map.len();
                drop(map);
                if count == 0 {
                    let tx = shutdown_tx.clone();
                    let c = clients.clone();
                    tokio::spawn(async move {
                        tokio::time::sleep(std::time::Duration::from_secs(grace)).await;
                        if c.lock().unwrap().is_empty() {
                            info!("nenhum cliente após watchdog — encerrando");
                            let _ = tx.send(true);
                        }
                    });
                }
            }
        });
    }
}

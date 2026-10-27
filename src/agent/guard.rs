//! O terra sempre abre, mesmo com um provedor quebrado.
//!
//! [`Guarded`] envolve um [`ChatAdapter`]: um panic em qualquer chamada vira
//! o valor padrão dela (ou um erro na resposta), em vez de derrubar a tela.
//! O [`install_hook`] faz o mesmo com os panics das threads de fundo dos
//! adapters: eles não restauram o terminal, só viram um aviso
//! ([`take_panics`]) que o chat mostra.

use std::any::Any;
use std::cell::Cell;
use std::panic::{self, AssertUnwindSafe, PanicHookInfo};
use std::path::Path;
use std::process::Command;
use std::sync::Mutex;
use std::sync::mpsc::Receiver;
use std::thread::{self, ThreadId};

use super::{
    AccountOption, AgentEvent, Catalog, ChatAdapter, DailyTokens, Interactive, ModelOption,
    PermissionMode, Request, Session, Usage,
};

thread_local! {
    /// Dentro de [`catch`]: o panic é tratado lá.
    static GUARDED: Cell<bool> = const { Cell::new(false) };
}

/// Panics das threads de fundo, para o chat mostrar.
static PANICS: Mutex<Vec<String>> = Mutex::new(Vec::new());

/// Instala o hook de panic da tela principal, por cima do atual (o do
/// ratatui, que restaura o terminal). Só um panic da thread principal fora
/// de [`catch`] chega a ele; os outros viram avisos.
pub fn install_hook() {
    let previous = panic::take_hook();
    let main = thread::current().id();
    panic::set_hook(Box::new(move |info| {
        if is_fatal(main, thread::current().id(), GUARDED.get()) {
            previous(info);
        } else if !GUARDED.get() {
            record(message(info));
        }
    }));
}

fn is_fatal(main: ThreadId, current: ThreadId, guarded: bool) -> bool {
    current == main && !guarded
}

fn message(info: &PanicHookInfo<'_>) -> String {
    payload_text(info.payload())
}

fn payload_text(payload: &(dyn Any + Send)) -> String {
    payload
        .downcast_ref::<&str>()
        .map(|s| s.to_string())
        .or_else(|| payload.downcast_ref::<String>().cloned())
        .unwrap_or_else(|| "erro inesperado".to_string())
}

fn record(message: String) {
    if let Ok(mut panics) = PANICS.lock() {
        panics.push(message);
    }
}

/// Os avisos de panics desde a última chamada.
pub fn take_panics() -> Vec<String> {
    PANICS
        .lock()
        .map(|mut panics| std::mem::take(&mut *panics))
        .unwrap_or_default()
}

/// Roda `f`; se ele entrar em panic, devolve o erro em vez de propagar.
pub fn catch<T>(f: impl FnOnce() -> T) -> Result<T, String> {
    let was = GUARDED.replace(true);
    let result = panic::catch_unwind(AssertUnwindSafe(f));
    GUARDED.set(was);
    result.map_err(|payload| payload_text(payload.as_ref()))
}

/// O provedor de `make`, protegido; se criar ele já falha, um provedor
/// indisponível que diz o porquê.
pub fn guarded(make: impl FnOnce() -> Box<dyn ChatAdapter>) -> Box<dyn ChatAdapter> {
    match catch(make) {
        Ok(adapter) => Box::new(Guarded::new(adapter)),
        Err(reason) => Box::new(Broken(reason)),
    }
}

/// Um [`ChatAdapter`] em que um panic vira o valor padrão da chamada.
pub struct Guarded {
    inner: Box<dyn ChatAdapter>,
    name: String,
}

impl Guarded {
    pub fn new(inner: Box<dyn ChatAdapter>) -> Self {
        let name = catch(|| inner.name().to_string()).unwrap_or_else(|_| "Provedor".into());
        Self { inner, name }
    }
}

fn failed(name: &str, reason: &str) -> String {
    format!("{name} falhou: {reason}")
}

impl ChatAdapter for Guarded {
    fn name(&self) -> &str {
        &self.name
    }

    fn is_available(&self) -> bool {
        catch(|| self.inner.is_available()).unwrap_or(false)
    }

    fn unavailable_hint(&self) -> String {
        catch(|| self.inner.unavailable_hint()).unwrap_or_else(|reason| failed(&self.name, &reason))
    }

    fn configured_mode(&self, cwd: &Path, account: Option<&str>) -> Option<PermissionMode> {
        catch(|| self.inner.configured_mode(cwd, account))
            .ok()
            .flatten()
    }

    fn accounts(&self) -> Vec<AccountOption> {
        catch(|| self.inner.accounts()).unwrap_or_default()
    }

    fn models(&self) -> Vec<ModelOption> {
        catch(|| self.inner.models()).unwrap_or_default()
    }

    fn model_label(&self, id: &str) -> String {
        catch(|| self.inner.model_label(id)).unwrap_or_else(|_| id.to_string())
    }

    fn load_catalog(&self, account: Option<&str>) -> Option<Receiver<Result<Catalog, String>>> {
        catch(|| self.inner.load_catalog(account)).ok().flatten()
    }

    fn load_usage(&self, account: Option<&str>) -> Option<Receiver<Result<Usage, String>>> {
        catch(|| self.inner.load_usage(account)).ok().flatten()
    }

    fn login(&self, account: Option<&str>) -> Option<Command> {
        catch(|| self.inner.login(account)).ok().flatten()
    }

    fn interactive(&self, cwd: &Path, options: Interactive<'_>) -> Option<Command> {
        catch(|| self.inner.interactive(cwd, options))
            .ok()
            .flatten()
    }

    fn load_daily(
        &self,
        account: Option<&str>,
        days: usize,
    ) -> Option<Receiver<Result<Vec<DailyTokens>, String>>> {
        catch(|| self.inner.load_daily(account, days))
            .ok()
            .flatten()
    }

    fn start(&self, request: Request<'_>) -> Result<Box<dyn Session>, String> {
        let session =
            catch(|| self.inner.start(request)).map_err(|reason| failed(&self.name, &reason))??;
        Ok(Box::new(GuardedSession {
            inner: session,
            name: self.name.clone(),
        }))
    }
}

/// Uma resposta em que um panic vira [`AgentEvent::Failed`].
struct GuardedSession {
    inner: Box<dyn Session>,
    name: String,
}

impl Session for GuardedSession {
    fn poll(&mut self) -> Vec<AgentEvent> {
        catch(|| self.inner.poll())
            .unwrap_or_else(|reason| vec![AgentEvent::Failed(failed(&self.name, &reason))])
    }
}

/// O provedor que não conseguiu nem ser criado: fica Desconectado.
struct Broken(String);

impl ChatAdapter for Broken {
    fn name(&self) -> &str {
        "Provedor"
    }

    fn is_available(&self) -> bool {
        false
    }

    fn unavailable_hint(&self) -> String {
        failed("O provedor", &self.0)
    }

    fn start(&self, _request: Request<'_>) -> Result<Box<dyn Session>, String> {
        Err(self.unavailable_hint())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Um provedor que entra em panic em tudo.
    struct Panics;

    impl ChatAdapter for Panics {
        fn name(&self) -> &str {
            "Ruim"
        }
        fn is_available(&self) -> bool {
            panic!("is_available quebrou")
        }
        fn accounts(&self) -> Vec<AccountOption> {
            panic!("accounts quebrou")
        }
        fn start(&self, _request: Request<'_>) -> Result<Box<dyn Session>, String> {
            panic!("start quebrou")
        }
    }

    #[test]
    fn panics_become_defaults_and_errors() {
        let adapter = guarded(|| Box::new(Panics));
        assert_eq!(adapter.name(), "Ruim");
        assert!(!adapter.is_available());
        assert!(adapter.accounts().is_empty());
        let error = adapter
            .start(Request {
                prompt: "oi",
                mode: PermissionMode::Agente,
                session_id: None,
                model: None,
                account: None,
                effort: None,
            })
            .err()
            .unwrap();
        assert_eq!(error, "Ruim falhou: start quebrou");
    }

    #[test]
    fn a_provider_that_fails_to_be_created_is_unavailable() {
        let adapter = guarded(|| panic!("detect quebrou"));
        assert!(!adapter.is_available());
        assert_eq!(
            adapter.unavailable_hint(),
            "O provedor falhou: detect quebrou"
        );
    }

    #[test]
    fn only_unguarded_main_thread_panics_are_fatal() {
        let main = thread::current().id();
        let other = thread::spawn(|| thread::current().id()).join().unwrap();
        assert!(is_fatal(main, main, false));
        assert!(!is_fatal(main, main, true));
        assert!(!is_fatal(main, other, false));
    }
}

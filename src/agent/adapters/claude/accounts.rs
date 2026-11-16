//! Contas do Claude neste computador.
//!
//! Cada conta é uma pasta de configuração na home (`~/.claude`,
//! `~/.claude-trabalho`...) com credenciais; o `claude` usa a escolhida pela
//! variável `CLAUDE_CONFIG_DIR`. A conta "Default" não define a variável, e o
//! `claude` usa a conta padrão dele.
//!
//! O tipo vem da organização: a organização pessoal da Anthropic se chama
//! "<e-mail>'s Organization"; as outras são de trabalho.

use std::fs;
use std::path::{Path, PathBuf};

use serde_json::Value;

use crate::agent::AccountOption;

/// Nome das pastas de conta: `.claude` ou `.claude-<algo>`.
const PREFIX: &str = ".claude";
const CREDENTIALS: &str = ".credentials.json";

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Kind {
    Pessoal,
    Trabalho,
}

impl Kind {
    fn label(self) -> &'static str {
        match self {
            Kind::Pessoal => "Pessoal",
            Kind::Trabalho => "Trabalho",
        }
    }
}

/// E-mail e organização de uma conta, lidos do `.claude.json`.
struct Info {
    email: String,
    org: String,
}

impl Info {
    fn kind(&self) -> Kind {
        if self.org.is_empty() || self.org.ends_with("'s Organization") {
            Kind::Pessoal
        } else {
            Kind::Trabalho
        }
    }
}

/// As contas: primeiro a Default (com o e-mail da conta que o `claude` usa
/// sem `CLAUDE_CONFIG_DIR`, lido de `default_json`), depois as pastas de
/// `home` com credenciais, pessoais antes das de trabalho.
///
/// A pasta `default_dir` (a que o `claude` usa sem `CLAUDE_CONFIG_DIR`,
/// normalmente `~/.claude`) não entra na lista: as credenciais dela são as
/// da Default. Listadas as duas, cada login numa delas trocava o token da
/// outra, e a outra caía no login de novo.
pub fn discover(
    home: &Path,
    default_json: &Path,
    default_dir: Option<&Path>,
) -> Vec<AccountOption> {
    let default = AccountOption {
        id: None,
        label: match read_info(default_json) {
            Some(info) => format!("Default · {}", info.email),
            None => "Default".to_string(),
        },
    };

    let mut found: Vec<(Kind, String, String)> = fs::read_dir(home)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|entry| {
            let name = entry.file_name().to_string_lossy().into_owned();
            let path = entry.path();
            let is_default = default_dir.is_some_and(|dir| same_dir(&path, dir));
            (is_account_name(&name) && !is_default && path.join(CREDENTIALS).is_file())
                .then_some((name, path))
        })
        .map(|(name, path)| match read_info(&path.join(".claude.json")) {
            Some(info) => {
                let kind = info.kind();
                let label = match kind {
                    Kind::Pessoal => format!("{} · {}", kind.label(), info.email),
                    Kind::Trabalho => {
                        format!("{} · {} ({})", kind.label(), info.email, info.org)
                    }
                };
                (kind, name, label)
            }
            // Sem o `.claude.json`, não dá para saber de quem é: o nome da pasta.
            None => (Kind::Pessoal, name.clone(), name),
        })
        .collect();
    found.sort();

    std::iter::once(default)
        .chain(found.into_iter().map(|(_, name, label)| AccountOption {
            id: Some(name),
            label,
        }))
        .collect()
}

/// O e-mail da conta, lido do `.claude.json` em `json`.
pub fn email(json: &Path) -> Option<String> {
    read_info(json).map(|info| info.email)
}

/// A pasta de configuração da conta `id`, se ela existir. Só aceita nomes de
/// conta (sem `/` nem `..`), para um valor salvo não apontar para outro lugar.
pub fn config_dir(home: &Path, id: &str) -> Option<PathBuf> {
    let path = home.join(id);
    (is_account_name(id) && path.is_dir()).then_some(path)
}

/// Cria a pasta de uma conta nova em `home`: a primeira
/// `.claude-conta-N` livre. Uma pasta dessas sem credenciais (de um
/// `/new-login` que não terminou) não é conta e é usada de novo.
pub fn create_new(home: &Path) -> std::io::Result<PathBuf> {
    let path = (1..)
        .map(|n| home.join(format!("{PREFIX}-conta-{n}")))
        .find(|path| !path.join(CREDENTIALS).exists())
        .expect("sempre há um número livre");
    fs::create_dir_all(&path)?;
    Ok(path)
}

/// As duas pastas são a mesma (com links e `..` resolvidos, quando existem).
pub fn same_dir(a: &Path, b: &Path) -> bool {
    match (a.canonicalize(), b.canonicalize()) {
        (Ok(a), Ok(b)) => a == b,
        _ => a == b,
    }
}

fn is_account_name(name: &str) -> bool {
    let rest = name.strip_prefix(PREFIX);
    let valid = |rest: &str| {
        rest.is_empty()
            || rest
                .strip_prefix('-')
                .is_some_and(|s| !s.is_empty() && !s.contains(['/', '\\']) && s != "..")
    };
    rest.is_some_and(valid)
}

fn read_info(path: &Path) -> Option<Info> {
    let json: Value = serde_json::from_str(&fs::read_to_string(path).ok()?).ok()?;
    let account = &json["oauthAccount"];
    Some(Info {
        email: account["emailAddress"].as_str()?.to_string(),
        org: account["organizationName"]
            .as_str()
            .unwrap_or_default()
            .to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn account(dir: &Path, email: &str, org: &str, credentials: bool) {
        fs::create_dir_all(dir).unwrap();
        let json = format!(
            r#"{{"oauthAccount":{{"emailAddress":"{email}","organizationName":"{org}"}}}}"#
        );
        fs::write(dir.join(".claude.json"), json).unwrap();
        if credentials {
            fs::write(dir.join(CREDENTIALS), "{}").unwrap();
        }
    }

    #[test]
    fn discovers_default_personal_and_work_accounts() {
        let home = tempfile::tempdir().unwrap();
        let home = home.path();
        account(
            &home.join(".claude-empresa"),
            "eu@empresa.com",
            "Empresa",
            true,
        );
        account(
            &home.join(".claude"),
            "eu@gmail.com",
            "eu@gmail.com's Organization",
            true,
        );
        // Sem credenciais, não é uma conta.
        account(&home.join(".claude-velha"), "x@y.com", "Y", false);
        // O `~/.claude.json` da home é o da conta Default.
        fs::write(
            home.join(".claude.json"),
            r#"{"oauthAccount":{"emailAddress":"eu@gmail.com"}}"#,
        )
        .unwrap();

        let accounts = discover(home, &home.join(".claude.json"), None);
        let found: Vec<_> = accounts
            .iter()
            .map(|a| (a.id.as_deref(), a.label.as_str()))
            .collect();
        assert_eq!(
            found,
            [
                (None, "Default · eu@gmail.com"),
                (Some(".claude"), "Pessoal · eu@gmail.com"),
                (
                    Some(".claude-empresa"),
                    "Trabalho · eu@empresa.com (Empresa)"
                ),
            ]
        );
    }

    #[test]
    fn the_default_folder_is_the_default_account_and_not_a_second_one() {
        let home = tempfile::tempdir().unwrap();
        let home = home.path();
        // O `~/.claude.json` diz uma conta e o `~/.claude/.claude.json`
        // outra, mas as credenciais são o mesmo `~/.claude/.credentials.json`.
        account(&home.join(".claude"), "eu@gmail.com", "", true);
        account(
            &home.join(".claude-empresa"),
            "eu@empresa.com",
            "Empresa",
            true,
        );
        fs::write(
            home.join(".claude.json"),
            r#"{"oauthAccount":{"emailAddress":"eu@empresa.com"}}"#,
        )
        .unwrap();

        let accounts = discover(
            home,
            &home.join(".claude.json"),
            Some(&home.join(".claude")),
        );
        let ids: Vec<_> = accounts.iter().map(|a| a.id.as_deref()).collect();
        assert_eq!(ids, [None, Some(".claude-empresa")]);
    }

    #[test]
    fn new_account_folder_skips_accounts_and_reuses_unfinished_ones() {
        let home = tempfile::tempdir().unwrap();
        let home = home.path();
        let first = create_new(home).unwrap();
        assert_eq!(first, home.join(".claude-conta-1"));
        assert!(first.is_dir());
        // Sem credenciais (login não terminou), a mesma pasta serve.
        assert_eq!(create_new(home).unwrap(), first);
        // Com credenciais, é uma conta: a próxima é outra.
        fs::write(first.join(CREDENTIALS), "{}").unwrap();
        assert_eq!(create_new(home).unwrap(), home.join(".claude-conta-2"));
        assert!(is_account_name(".claude-conta-2"));
    }

    #[test]
    fn config_dir_only_accepts_account_folders() {
        let home = tempfile::tempdir().unwrap();
        let home = home.path();
        fs::create_dir_all(home.join(".claude-trabalho")).unwrap();
        assert_eq!(
            config_dir(home, ".claude-trabalho"),
            Some(home.join(".claude-trabalho"))
        );
        assert_eq!(config_dir(home, ".claude-inexistente"), None);
        assert_eq!(config_dir(home, "../etc"), None);
        assert_eq!(config_dir(home, ".claude-/x"), None);
        assert_eq!(config_dir(home, "Documentos"), None);
    }
}

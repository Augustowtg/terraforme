//! O `terra send` e o `terra handoff new`, do lado de quem manda.
//!
//! ```text
//! terra send <agente> [texto] [--ref <arquivo>]... [--handoff <arquivo>] [--tag <tag>]
//! terra send --lote <arquivo | -> [--tag <tag>]
//! ```
//!
//! O nível sai dos argumentos: com `--handoff`, 3; com `--ref`, 2; só o
//! texto, 1. Antes de mandar, o terra confere o destino, os arquivos, a nota,
//! os saltos e o tamanho do texto, e grava a nota da mensagem (a mini nota
//! ou a cópia do `--handoff`) em `messages/<msg_id>/note.md`. O hub confere
//! de novo e leva a mensagem. A saída é uma linha só: ela entra no contexto
//! do agente que mandou.

use std::fs;
use std::path::{Path, PathBuf};

use super::config::{self, Config};
use super::message::{self, Message, Reject, State};
use super::note;
use crate::hub::protocol::{new_id, now};

/// Os argumentos do `terra send`.
#[derive(Debug, Clone, Default)]
pub struct Args {
    pub to: String,
    pub text: Option<String>,
    pub refs: Vec<PathBuf>,
    pub handoff: Option<PathBuf>,
    pub tag: Option<String>,
}

/// Monta a mensagem, confere e grava a nota dela em `home`. `from` e
/// `hops` são de quem roda o comando ([`super::sender`]); os caminhos
/// relativos são de `cwd`.
pub fn prepare(
    args: &Args,
    from: &str,
    hops: u32,
    config: &Config,
    home: &Path,
    cwd: &Path,
) -> Result<Message, Reject> {
    if !config::valid_name(&args.to) {
        return Err(Reject::new(
            "agente",
            format!("\"{}\" não é um nome de agente", args.to),
        ));
    }
    if args.to == from {
        return Err(Reject::new("agente", "o destino é o próprio remetente"));
    }
    let text = args
        .text
        .as_deref()
        .map(str::trim)
        .filter(|t| !t.is_empty());
    if text.is_none() && args.handoff.is_none() {
        return Err(Reject::new("texto", "falta o texto ou o --handoff"));
    }
    let refs = args
        .refs
        .iter()
        .map(|path| existing(path, cwd))
        .collect::<Result<Vec<_>, _>>()?;
    let handoff = match &args.handoff {
        Some(path) => {
            let path = existing(path, cwd)?;
            let text = fs::read_to_string(&path)
                .map_err(|err| Reject::new("arquivo", format!("{}: {err}", path.display())))?;
            note::check(&text).map_err(|err| Reject::new("nota", err))?;
            Some(text)
        }
        None => None,
    };
    let id = handoff
        .as_deref()
        .and_then(note::id_of)
        .map_or_else(new_id, str::to_string);
    let dir = message::dir(home, &id);
    if dir.join("message.json").exists() {
        return Err(Reject::new(
            "nota",
            "esta nota já foi mandada: crie outra com `terra handoff new`",
        ));
    }
    let level = match (&handoff, refs.is_empty()) {
        (Some(_), _) => 3,
        (None, false) => 2,
        (None, true) => 1,
    };
    let message = Message {
        id: id.clone(),
        from: from.to_string(),
        to: args.to.clone(),
        level,
        hops: hops + 1,
        tag: args.tag.clone(),
        text: text.map(str::to_string),
        refs,
        note: dir.join("note.md"),
        created: now(),
    };
    let note = match handoff {
        Some(text) => text,
        None => note::mini(&id, from, &args.to, text.unwrap_or_default(), &message.refs),
    };
    fs::create_dir_all(&dir)
        .and_then(|()| fs::write(&message.note, note))
        .map_err(|err| Reject::new("arquivo", format!("{}: {err}", dir.display())))?;
    if let Err(reject) = message.check(&config.global) {
        let _ = fs::remove_dir_all(&dir);
        return Err(reject);
    }
    Ok(message)
}

/// As mensagens do `terra send --lote`: uma por linha, `agente: texto`.
/// Linhas vazias e comentários (`#`) ficam de fora; uma linha sem o `:` ou
/// sem texto é um erro com o número dela, antes de mandar qualquer uma.
pub fn batch(text: &str) -> Result<Vec<(String, String)>, String> {
    let mut items = Vec::new();
    for (n, line) in text.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let (to, text) = line
            .split_once(':')
            .map(|(to, text)| (to.trim(), text.trim()))
            .filter(|(to, text)| !to.is_empty() && !text.is_empty())
            .ok_or_else(|| format!("linha {}: use `agente: texto`", n + 1))?;
        items.push((to.to_string(), text.to_string()));
    }
    if items.is_empty() {
        return Err("o lote não tem mensagens".into());
    }
    Ok(items)
}

/// O caminho absoluto de um arquivo que existe.
fn existing(path: &Path, cwd: &Path) -> Result<PathBuf, Reject> {
    let full = cwd.join(path);
    if !full.exists() {
        return Err(Reject::new(
            "arquivo",
            format!("o arquivo não existe: {}", path.display()),
        ));
    }
    crate::db::canonical_path(&full)
        .map_err(|err| Reject::new("arquivo", format!("{}: {err}", path.display())))
}

/// A linha do `terra send` quando deu certo (ex.: `enviada a1b2c3d4 →
/// frontend (nível 2)`).
pub fn sent_line(message: &Message, state: State) -> String {
    format!(
        "{} {} → {} (nível {})",
        state.key().replace('_', " "),
        &message.id[..message.id.len().min(8)],
        message.to,
        message.level
    )
}

/// O `terra handoff new <destino>`: grava a nota em branco em
/// `handoffs/<msg_id>.md` e devolve o caminho.
pub fn new_handoff(to: &str, from: &str, home: &Path) -> Result<PathBuf, String> {
    if !config::valid_name(to) {
        return Err(format!("\"{to}\" não é um nome de agente"));
    }
    let id = new_id();
    let dir = home.join("handoffs");
    let path = dir.join(format!("{id}.md"));
    fs::create_dir_all(&dir)
        .and_then(|()| fs::write(&path, note::template(&id, from, to)))
        .map_err(|err| format!("{}: {err}", path.display()))?;
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(to: &str, text: &str) -> Args {
        Args {
            to: to.into(),
            text: Some(text.into()),
            ..Args::default()
        }
    }

    #[test]
    fn the_level_comes_from_the_arguments_and_a_note_is_always_written() {
        let home = tempfile::tempdir().unwrap();
        let cwd = tempfile::tempdir().unwrap();
        fs::write(cwd.path().join("a.rs"), "").unwrap();
        let config = Config::default();
        let prepare = |args: &Args| prepare(args, "usuario", 0, &config, home.path(), cwd.path());

        let one = prepare(&args("backend", "Rode os testes")).unwrap();
        assert_eq!((one.level, one.hops), (1, 1));
        let note = fs::read_to_string(&one.note).unwrap();
        assert!(note.contains("usuario → backend"));

        let two = prepare(&Args {
            refs: vec!["a.rs".into()],
            ..args("backend", "Veja o arquivo")
        })
        .unwrap();
        assert_eq!(two.level, 2);
        assert!(two.refs[0].is_absolute());
        assert!(fs::read_to_string(&two.note).unwrap().contains("a.rs"));

        let handoff = new_handoff("backend", "frontend", home.path()).unwrap();
        let three = prepare(&Args {
            to: "backend".into(),
            handoff: Some(handoff.clone()),
            ..Args::default()
        })
        .unwrap();
        assert_eq!(three.level, 3);
        assert!(handoff.to_string_lossy().contains(&three.id));
        // A mesma nota não vai duas vezes.
        message::save(&three, State::Queued, None, home.path()).unwrap();
        let again = prepare(&Args {
            to: "backend".into(),
            handoff: Some(handoff),
            ..Args::default()
        });
        assert_eq!(again.unwrap_err().code, "nota");
    }

    #[test]
    fn bad_sends_are_refused_before_reaching_the_hub() {
        let home = tempfile::tempdir().unwrap();
        let cwd = home.path();
        let config = Config::default();
        let refuse = |args: &Args, hops: u32| {
            prepare(args, "frontend", hops, &config, home.path(), cwd)
                .unwrap_err()
                .code
        };
        assert_eq!(refuse(&args("Backend", "x"), 0), "agente");
        assert_eq!(refuse(&args("frontend", "x"), 0), "agente");
        assert_eq!(refuse(&args("backend", "  "), 0), "texto");
        let missing = Args {
            refs: vec!["sumiu.rs".into()],
            ..args("backend", "x")
        };
        assert_eq!(refuse(&missing, 0), "arquivo");
        // Com 5 saltos antes, este seria o sexto (o máximo é 5).
        assert_eq!(refuse(&args("backend", "x"), 5), "hops");
        assert_eq!(refuse(&args("backend", &"p ".repeat(501)), 0), "palavras");
        // A recusa não deixa a pasta da mensagem para trás.
        assert!(
            !home.path().join("messages").exists()
                || fs::read_dir(home.path().join("messages"))
                    .unwrap()
                    .next()
                    .is_none()
        );
    }

    #[test]
    fn the_answer_is_one_short_line() {
        let msg = Message {
            id: "a1b2c3d4e5f6".into(),
            from: "usuario".into(),
            to: "frontend".into(),
            level: 2,
            hops: 1,
            tag: None,
            text: None,
            refs: Vec::new(),
            note: PathBuf::new(),
            created: 0,
        };
        assert_eq!(
            sent_line(&msg, State::Sent),
            "enviada a1b2c3d4 → frontend (nível 2)"
        );
        assert_eq!(
            sent_line(&msg, State::Queued),
            "na fila a1b2c3d4 → backend (nível 2)".replace("backend", "frontend")
        );
    }

    #[test]
    fn a_batch_has_one_message_per_line() {
        let items = batch("# plano\n\ndev: crie slug.py\ntester: teste: tudo\n").unwrap();
        assert_eq!(
            items,
            [
                ("dev".to_string(), "crie slug.py".to_string()),
                ("tester".into(), "teste: tudo".into())
            ]
        );
        assert!(
            batch("dev: ok\nsem dois pontos\n")
                .unwrap_err()
                .contains("linha 2")
        );
        assert!(batch("dev:\n").is_err());
        assert!(batch("# nada\n").is_err());
    }
}

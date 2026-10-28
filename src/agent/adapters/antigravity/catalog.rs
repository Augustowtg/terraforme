//! O catálogo e o consumo do Antigravity, lidos de comandos do `agy` que
//! não chamam o modelo nem gastam cota:
//!
//! - `agy models`: os modelos, um por linha (`id<TAB>nome`). Os que só
//!   mudam no nível de raciocínio (`gemini-3.8-flash-low`, `-medium`,
//!   `-high`) viram um modelo só, com os níveis como esforço;
//! - `agy -p=/help --output-format json`: os comandos da `/`;
//! - `agy -p=/skills --output-format json`: as skills;
//! - `agy -p=/model --output-format json`: o modelo em uso (o padrão);
//! - `agy -p=/usage --output-format json`: as cotas da conta.
//!
//! Os comandos da `/` no `-p` respondem num `result` com `num_turns: 0` e o
//! que pediram em `command.data`.

use std::collections::HashMap;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::mpsc::{self, Receiver};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use serde_json::Value;

use crate::agent::{Catalog, CommandInfo, ModelInfo, Usage, UsageWindow};

/// Quanto esperar um comando do `agy` antes de desistir dele.
const TIMEOUT: Duration = Duration::from_secs(20);
/// Por quanto tempo uma resposta do `/usage` vale: a aba Consumo e a barra
/// pedem juntas.
const FRESH: Duration = Duration::from_secs(50);
/// Por quanto tempo um erro do `/usage` (ex.: sem login) vale, para a tela
/// não rodar o `agy` toda hora enquanto ele falha.
const FRESH_ERROR: Duration = Duration::from_secs(30);

/// Os níveis de raciocínio que o `agy` põe no fim do id de um modelo, do
/// menor para o maior (os do `--effort` dele).
const EFFORTS: &[&str] = &["low", "medium", "high", "xhigh", "max"];

/// Os nomes dos modelos (id → nome), que o catálogo preenche para o
/// [`super::AntigravityAdapter::model_label`].
pub type Labels = Arc<Mutex<HashMap<String, String>>>;

/// Os níveis de cada modelo agrupado (id sem o nível → níveis, do menor para
/// o maior), que o adapter usa para montar o id que o `agy` recebe.
pub type Families = Arc<Mutex<HashMap<String, Vec<String>>>>;

/// Carrega, numa thread, os comandos, as skills e os modelos.
pub fn load(bin: PathBuf, labels: Labels, families: Families) -> Receiver<Result<Catalog, String>> {
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        let _ = tx.send(catalog(&bin, &labels, &families));
    });
    rx
}

fn catalog(bin: &Path, labels: &Labels, families: &Families) -> Result<Catalog, String> {
    let all = parse_models(&run(bin, &["models"])?);
    let (models, grouped) = group_models(&all);
    {
        // Os ids de cada nível seguem com o nome deles: a conversa informa
        // o id completo (ex.: `gemini-3.8-flash-low`).
        let mut labels = labels.lock().unwrap();
        for model in all.iter().chain(&models) {
            labels.insert(model.id.clone(), model.label.clone());
        }
    }
    *families.lock().unwrap() = grouped;
    let mut commands = parse_commands(&slash(bin, "/help").unwrap_or_default());
    commands.extend(parse_skills(&slash(bin, "/skills").unwrap_or_default()));
    let default_model = slash(bin, "/model").ok().and_then(|json| {
        let id = json["command"]["data"]["id"].as_str()?;
        Some(group_id(&models, id))
    });
    Ok(Catalog {
        commands,
        models,
        default_model,
    })
}

/// O consumo: uma cota por grupo de modelos. A resposta (certa ou de erro)
/// é reaproveitada por [`FRESH`] ou [`FRESH_ERROR`], porque cada `agy` leva
/// uns 7 s e a aba Consumo e a barra pedem juntas, a cada minuto.
pub fn load_usage(bin: PathBuf) -> Receiver<Result<Usage, String>> {
    static CACHE: Mutex<Option<(Instant, Result<Usage, String>)>> = Mutex::new(None);
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        // O trinco fica preso durante a consulta: o segundo pedido espera o
        // primeiro e já acha a resposta no cache, em vez de rodar outro `agy`.
        let mut cache = CACHE
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if let Some((at, result)) = cache.as_ref() {
            let fresh = if result.is_ok() { FRESH } else { FRESH_ERROR };
            if at.elapsed() < fresh {
                let _ = tx.send(result.clone());
                return;
            }
        }
        let result = slash(&bin, "/usage").and_then(|json| parse_usage(&json));
        *cache = Some((Instant::now(), result.clone()));
        let _ = tx.send(result);
    });
    rx
}

/// Um comando da `/` no `-p`, com a saída em JSON. Erro se o `agy` diz
/// que falhou (ex.: sem login).
fn slash(bin: &Path, command: &str) -> Result<Value, String> {
    let prompt = format!("-p={command}");
    let out = run(bin, &[&prompt, "--output-format", "json"])?;
    let json: Value = serde_json::from_str(out.trim())
        .map_err(|_| format!("resposta inesperada do agy a {command}"))?;
    if json["status"] != "SUCCESS" {
        let error = json["error"].as_str().unwrap_or("erro desconhecido");
        return Err(format!("o agy falhou em {command}: {error}"));
    }
    Ok(json)
}

/// Roda o `agy` com `args` e devolve o stdout. O stdin fica fechado (o
/// `agy` não espera nada) e o processo morre depois do [`TIMEOUT`].
fn run(bin: &Path, args: &[&str]) -> Result<String, String> {
    let mut child = super::command(bin)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|err| format!("não foi possível iniciar o agy: {err}"))?;
    let mut stdout = child.stdout.take().expect("stdout configurado como pipe");
    let reader = thread::spawn(move || {
        let mut text = String::new();
        let _ = stdout.read_to_string(&mut text);
        text
    });
    let start = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if start.elapsed() < TIMEOUT => thread::sleep(Duration::from_millis(50)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!("o agy {} não respondeu", args.join(" ")));
            }
        }
    };
    let text = reader.join().unwrap_or_default();
    // O `-p` com erro sai com código diferente de zero, mas o JSON dele diz
    // o porquê; sem saída, vale o código.
    if !status.success() && text.trim().is_empty() {
        return Err(format!("o agy {} falhou ({status})", args.join(" ")));
    }
    Ok(text)
}

/// `id<TAB>nome` por linha. As outras linhas (ex.: avisos) ficam de fora.
fn parse_models(text: &str) -> Vec<ModelInfo> {
    text.lines()
        .filter_map(|line| {
            let (id, label) = line.split_once('\t')?;
            let (id, label) = (id.trim(), label.trim());
            (!id.is_empty() && !id.contains(' ')).then(|| ModelInfo {
                id: id.to_string(),
                label: label.to_string(),
                description: String::new(),
                resolved: None,
                // O nível de raciocínio já vem no id (ex.: `-high`).
                efforts: Vec::new(),
            })
        })
        .collect()
}

/// Separa o nível do fim do id (`gemini-3.8-flash-low` → `gemini-3.8-flash`
/// e `low`).
fn split_effort(id: &str) -> Option<(&str, &str)> {
    let (base, level) = id.rsplit_once('-')?;
    (!base.is_empty() && EFFORTS.contains(&level)).then_some((base, level))
}

/// Junta os modelos que só diferem no nível num modelo só, com os níveis em
/// `efforts` (o nome perde o `(High)` do fim). Um modelo com um nível só,
/// ou sem nível no id, fica como está. Devolve também os níveis de cada
/// modelo agrupado.
fn group_models(models: &[ModelInfo]) -> (Vec<ModelInfo>, HashMap<String, Vec<String>>) {
    let mut levels: HashMap<&str, Vec<&str>> = HashMap::new();
    for (base, level) in models.iter().filter_map(|m| split_effort(&m.id)) {
        let levels = levels.entry(base).or_default();
        if !levels.contains(&level) {
            levels.push(level);
        }
    }
    levels.retain(|_, levels| levels.len() > 1);
    let rank = |level: &&str| EFFORTS.iter().position(|e| e == level);
    for levels in levels.values_mut() {
        levels.sort_by_key(rank);
    }

    let mut grouped = HashMap::new();
    let mut out: Vec<ModelInfo> = Vec::new();
    for model in models {
        let Some((base, _)) = split_effort(&model.id).filter(|(base, _)| levels.contains_key(base))
        else {
            out.push(model.clone());
            continue;
        };
        if out.iter().any(|m| m.id == base) {
            continue;
        }
        let efforts: Vec<String> = levels[base].iter().map(|l| l.to_string()).collect();
        grouped.insert(base.to_string(), efforts.clone());
        out.push(ModelInfo {
            id: base.to_string(),
            label: strip_level(&model.label).to_string(),
            efforts,
            ..model.clone()
        });
    }
    (out, grouped)
}

/// O nome sem o nível do fim (`Gemini 3.8 Flash (High)` → `Gemini 3.8 Flash`).
fn strip_level(label: &str) -> &str {
    match label
        .strip_suffix(')')
        .and_then(|rest| rest.rsplit_once(" ("))
    {
        Some((name, _)) if !name.is_empty() => name,
        _ => label,
    }
}

/// O id do catálogo para o `id` que o `agy` informa: o do modelo agrupado,
/// se `id` for um dos níveis dele.
fn group_id(models: &[ModelInfo], id: &str) -> String {
    match split_effort(id) {
        Some((base, _)) if models.iter().any(|m| m.id == base) => base.to_string(),
        _ => id.to_string(),
    }
}

/// Os comandos do `/help`, com os apelidos (ex.: `quota` do `usage`).
fn parse_commands(json: &Value) -> Vec<CommandInfo> {
    let Some(commands) = json["command"]["data"]["commands"].as_array() else {
        return Vec::new();
    };
    commands
        .iter()
        .filter_map(|command| {
            let name = command["name"].as_str().filter(|n| !n.is_empty())?;
            let description = command["description"].as_str().unwrap_or_default();
            let aliases = command["aliases"].as_array().into_iter().flatten();
            Some(
                std::iter::once(name)
                    .chain(aliases.filter_map(Value::as_str).filter(|a| !a.is_empty()))
                    .map(|name| CommandInfo {
                        name: name.to_string(),
                        description: description.to_string(),
                        skill: false,
                    })
                    .collect::<Vec<_>>(),
            )
        })
        .flatten()
        .collect()
}

/// As skills do `/skills`, que a `/` mostra como skills.
fn parse_skills(json: &Value) -> Vec<CommandInfo> {
    let Some(skills) = json["command"]["data"]["skills"].as_array() else {
        return Vec::new();
    };
    skills
        .iter()
        .filter_map(|skill| {
            let name = skill["name"].as_str().filter(|n| !n.is_empty())?;
            Some(CommandInfo {
                name: name.to_string(),
                description: skill["description"]
                    .as_str()
                    .unwrap_or_default()
                    .to_string(),
                skill: true,
            })
        })
        .collect()
}

/// Uma janela por cota (`groups[].buckets[]`): o nome do grupo de modelos,
/// o que resta (`remaining_fraction`) e quando zera (`reset_time`). O `agy`
/// só tem cotas semanais: nenhuma é a da sessão, então a barra de consumo do
/// chat não aparece.
fn parse_usage(json: &Value) -> Result<Usage, String> {
    let groups = json["command"]["data"]["groups"]
        .as_array()
        .ok_or("o agy não informou as cotas")?;
    let windows = groups
        .iter()
        .flat_map(|group| {
            let name = group["name"].as_str().unwrap_or("Modelos").to_string();
            let buckets = group["buckets"].as_array().cloned().unwrap_or_default();
            let several = buckets.len() > 1;
            buckets.into_iter().map(move |bucket| {
                let remaining = bucket["remaining_fraction"].as_f64().unwrap_or(1.0);
                let window = window_label(bucket["window"].as_str().unwrap_or_default());
                let label = match (several, bucket["name"].as_str()) {
                    (true, Some(bucket)) => format!("{name} · {bucket}"),
                    _ => format!("{name}{window}"),
                };
                UsageWindow {
                    label,
                    percent: ((1.0 - remaining) * 100.0).clamp(0.0, 100.0),
                    resets_at: bucket["reset_time"]
                        .as_str()
                        .and_then(|time| time.parse::<jiff::Timestamp>().ok())
                        .map(|time| time.as_second()),
                    detail: None,
                    session: false,
                }
            })
        })
        .collect();
    Ok(Usage {
        plan: None,
        windows,
    })
}

fn window_label(window: &str) -> &'static str {
    match window {
        "weekly" => " (semanal)",
        "daily" => " (diária)",
        "monthly" => " (mensal)",
        _ => "",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn models_come_one_per_line() {
        let models = parse_models(
            "gemini-3.8-flash-high\tGemini 3.8 Flash (High)\nclaude-sonnet-4-6\tClaude Sonnet 4.6 (Thinking)\nFetching available models...\n",
        );
        let ids: Vec<&str> = models.iter().map(|m| m.id.as_str()).collect();
        assert_eq!(ids, ["gemini-3.8-flash-high", "claude-sonnet-4-6"]);
        assert_eq!(models[0].label, "Gemini 3.8 Flash (High)");
        assert!(models[0].efforts.is_empty());
    }

    #[test]
    fn help_and_skills_become_commands() {
        let help = serde_json::json!({"status":"SUCCESS","command":{"name":"help","data":{"commands":[
            {"name":"config","aliases":["settings"],"description":"Open settings panel"},
            {"name":"usage","aliases":["quota"],"description":"View model quota usage"}
        ]}}});
        let skills = serde_json::json!({"status":"SUCCESS","command":{"name":"skills","data":{"skills":[
            {"name":"automation","description":"Agendar","path":"/x/SKILL.md","builtin":true}
        ]}}});
        let mut commands = parse_commands(&help);
        commands.extend(parse_skills(&skills));
        let names: Vec<(&str, bool)> = commands
            .iter()
            .map(|c| (c.name.as_str(), c.skill))
            .collect();
        assert_eq!(
            names,
            [
                ("config", false),
                ("settings", false),
                ("usage", false),
                ("quota", false),
                ("automation", true)
            ]
        );
    }

    fn models(text: &str) -> Vec<ModelInfo> {
        parse_models(text)
    }

    #[test]
    fn levels_of_one_model_become_its_efforts() {
        let all = models(
            "gemini-3.8-flash-high\tGemini 3.8 Flash (High)\ngemini-3.8-flash-medium\tGemini 3.8 Flash (Medium)\ngemini-3.8-flash-low\tGemini 3.8 Flash (Low)\nclaude-sonnet-4-6\tClaude Sonnet 4.6 (Thinking)\ngpt-oss-120b-medium\tGPT-OSS 120B (Medium)\n",
        );
        let (grouped, families) = group_models(&all);
        let ids: Vec<&str> = grouped.iter().map(|m| m.id.as_str()).collect();
        // Um nível só ou sem nível no id: fica como o `agy models` informa.
        assert_eq!(
            ids,
            [
                "gemini-3.8-flash",
                "claude-sonnet-4-6",
                "gpt-oss-120b-medium"
            ]
        );
        assert_eq!(grouped[0].label, "Gemini 3.8 Flash");
        assert_eq!(grouped[0].efforts, ["low", "medium", "high"]);
        assert_eq!(grouped[1].label, "Claude Sonnet 4.6 (Thinking)");
        assert!(grouped[1].efforts.is_empty() && grouped[2].efforts.is_empty());
        assert_eq!(families["gemini-3.8-flash"], ["low", "medium", "high"]);
        assert_eq!(families.len(), 1);
    }

    #[test]
    fn the_default_model_is_the_grouped_one() {
        let all = models("m-high\tM (High)\nm-low\tM (Low)\nsolo-medium\tSolo (Medium)\n");
        let (grouped, _) = group_models(&all);
        assert_eq!(group_id(&grouped, "m-low"), "m");
        assert_eq!(group_id(&grouped, "solo-medium"), "solo-medium");
        assert_eq!(group_id(&grouped, "outro"), "outro");
    }

    #[test]
    fn usage_has_one_window_per_quota() {
        let json = serde_json::json!({"status":"SUCCESS","command":{"name":"usage","data":{"groups":[
            {"name":"Gemini Models","buckets":[{"id":"gemini-weekly","name":"Weekly Limit Remaining","window":"weekly","remaining_fraction":0.75,"reset_time":"2026-11-04T04:19:16Z"}]},
            {"name":"Claude and GPT models","buckets":[{"id":"3p-weekly","name":"Weekly Limit Remaining","window":"weekly","remaining_fraction":1,"reset_time":"2026-11-04T04:29:09Z"}]}
        ]}}});
        let usage = parse_usage(&json).unwrap();
        assert_eq!(usage.windows.len(), 2);
        assert_eq!(usage.windows[0].label, "Gemini Models (semanal)");
        assert_eq!(usage.windows[0].percent, 25.0);
        assert_eq!(usage.windows[0].resets_at, Some(1_793_765_956));
        assert_eq!(usage.windows[1].percent, 0.0);
        // Só cotas semanais: a barra do chat não tem janela de sessão.
        assert!(usage.session().is_none());
    }

    #[test]
    fn usage_without_groups_is_an_error() {
        assert!(parse_usage(&serde_json::json!({"status":"SUCCESS"})).is_err());
    }
}

//! O `terra usage`: o consumo da aba Consumo, em texto ou JSON. Os limites
//! da conta do workspace e os tokens dos últimos dias, por modelo e por
//! modelo no diretório atual (com todos os provedores e contas).

use std::path::Path;
use std::sync::mpsc::Receiver;

use anyhow::{Result, bail};
use jiff::Timestamp;
use jiff::tz::TimeZone;
use serde_json::{Value, json};

use crate::agent::{ChatAdapter, ModelTokens, Usage, adapters};
use crate::tui::{account_ids, compact_count};

/// `provider` e `account` são os do workspace quando a linha de comando não
/// diz outros.
pub fn run(
    provider: Option<&str>,
    account: Option<&str>,
    days: usize,
    as_json: bool,
    cwd: &Path,
) -> Result<()> {
    let adapter = adapters::select(provider);
    if provider.is_some_and(|id| id != adapter.id()) {
        bail!("provedor desconhecido: {}", provider.unwrap_or_default());
    }
    if !adapter.is_available() {
        bail!("{} não encontrado", adapter.name());
    }
    // Uma conta salva que é só outro nome da padrão é a padrão.
    let account = adapter.normalize_account(account.map(str::to_string));
    let account = account.as_deref();
    // Tudo começa junto; cada um chega pelo seu canal.
    let usage = adapter.load_usage(account);
    let report = adapter.load_daily(account, days, cwd);
    // As outras contas de cada provedor, para o total do workspace.
    let others: Vec<_> = adapters::all()
        .into_iter()
        .filter(|a| a.is_available())
        .map(|a| {
            let reports: Vec<_> = account_ids(a.as_ref())
                .into_iter()
                .filter(|id| !(a.id() == adapter.id() && id.as_deref() == account))
                .map(|id| a.load_daily(id.as_deref(), days, cwd))
                .collect();
            (a, reports)
        })
        .collect();

    let usage = usage.map(|rx| wait(Some(rx)));
    let report = wait(report).unwrap_or_default();
    let mut workspace = rows(adapter.as_ref(), &report.workspace, true);
    for (other, reports) in others {
        for report in reports {
            let report = wait(report).unwrap_or_default();
            for row in rows(other.as_ref(), &report.workspace, true) {
                // O mesmo modelo em outra conta do provedor soma.
                match workspace.iter_mut().find(|r| r.label == row.label) {
                    Some(found) => {
                        found.tokens += row.tokens;
                        found.cache += row.cache;
                    }
                    None => workspace.push(row),
                }
            }
        }
    }
    let models = rows(adapter.as_ref(), &report.models, false);

    if as_json {
        let list = |rows: &[Row]| -> Vec<Value> {
            rows.iter()
                .map(|r| json!({"provider": r.provider, "model": r.model, "label": r.label, "tokens": r.tokens, "cache": r.cache}))
                .collect()
        };
        let (windows, error) = match &usage {
            Some(Ok(usage)) => (
                usage
                    .windows
                    .iter()
                    .map(|w| json!({"label": w.label, "used_percent": w.percent, "resets_at": w.resets_at, "detail": w.detail}))
                    .collect(),
                None,
            ),
            Some(Err(error)) => (Vec::new(), Some(error.clone())),
            None => (Vec::new(), None),
        };
        let plan = usage.as_ref().and_then(|u| u.as_ref().ok()?.plan.clone());
        println!(
            "{:#}",
            json!({
                "provider": adapter.id(),
                "account": account,
                "plan": plan,
                "windows": windows,
                "error": error,
                "days": days,
                "models": list(&models),
                "workspace": list(&workspace),
            })
        );
        return Ok(());
    }

    println!("{}", adapter.name());
    match &usage {
        Some(Ok(usage)) => print_windows(usage),
        Some(Err(error)) => println!("  {error}"),
        None => println!("  Este provedor não informa consumo."),
    }
    print_rows(&format!("Tokens por modelo (últimos {days} dias)"), models);
    print_rows(
        &format!("Tokens na workspace (últimos {days} dias)"),
        workspace,
    );
    Ok(())
}

fn wait<T>(rx: Option<Receiver<Result<T, String>>>) -> Result<T, String> {
    rx.ok_or_else(|| "o provedor não informa".to_string())?
        .recv()
        .map_err(|_| "o carregamento parou".to_string())?
}

struct Row {
    provider: String,
    model: String,
    label: String,
    tokens: u64,
    cache: u64,
}

fn rows(adapter: &dyn ChatAdapter, list: &[ModelTokens], with_provider: bool) -> Vec<Row> {
    list.iter()
        .filter(|m| m.tokens + m.cache > 0)
        .map(|m| {
            let label = adapter.model_label(&m.model);
            Row {
                provider: adapter.id().to_string(),
                model: m.model.clone(),
                label: if with_provider {
                    format!("{} · {label}", adapter.name())
                } else {
                    label
                },
                tokens: m.tokens,
                cache: m.cache,
            }
        })
        .collect()
}

fn print_windows(usage: &Usage) {
    if let Some(plan) = &usage.plan {
        println!("  Plano: {plan}");
    }
    let tz = TimeZone::system();
    let width = usage.windows.iter().map(|w| w.label.chars().count()).max();
    let width = width.unwrap_or(0);
    for window in &usage.windows {
        let left = (100.0 - window.percent).clamp(0.0, 100.0);
        let reset = match (&window.detail, window.resets_at) {
            (Some(detail), _) => format!(" · {detail}"),
            (None, Some(at)) => Timestamp::from_second(at)
                .map(|t| {
                    format!(
                        " · reseta {}",
                        t.to_zoned(tz.clone()).strftime("%d-%m-%Y %H:%M")
                    )
                })
                .unwrap_or_default(),
            (None, None) => String::new(),
        };
        println!("  {:<width$}  {left:>3.0}% restantes{reset}", window.label);
    }
}

/// O maior primeiro, com o total: os tokens e, ao lado, o cache.
fn print_rows(title: &str, mut rows: Vec<Row>) {
    println!("\n{title}");
    if rows.is_empty() {
        println!("  Sem consumo no período.");
        return;
    }
    rows.sort_by(|a, b| b.tokens.cmp(&a.tokens).then(a.label.cmp(&b.label)));
    let total = Row {
        provider: String::new(),
        model: String::new(),
        label: "Total".to_string(),
        tokens: rows.iter().map(|r| r.tokens).sum(),
        cache: rows.iter().map(|r| r.cache).sum(),
    };
    rows.push(total);
    let width = rows
        .iter()
        .map(|r| r.label.chars().count())
        .max()
        .unwrap_or(0);
    for row in &rows {
        println!(
            "  {:<width$}  {:>6}  (+{} de cache)",
            row.label,
            compact_count(row.tokens),
            compact_count(row.cache)
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rows_skip_unused_models_and_name_the_provider() {
        let adapter = adapters::claude::ClaudeAdapter::detect();
        let list = [
            ModelTokens {
                model: "claude-fable-5-2".into(),
                tokens: 10,
                cache: 5,
            },
            ModelTokens {
                model: "<synthetic>".into(),
                tokens: 0,
                cache: 0,
            },
        ];
        let found = rows(&adapter, &list, true);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].label, "Claude · Fable - 5.2");
        assert_eq!(rows(&adapter, &list, false)[0].label, "Fable - 5.2");
    }
}

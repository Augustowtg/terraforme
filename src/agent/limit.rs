//! O limite de uso do provedor: perceber que uma resposta acabou nele e quando
//! ele reseta, para o chat mandar o `continue` sozinho depois.

use jiff::Timestamp;
use jiff::civil::Time;
use jiff::tz::TimeZone;

/// O que vai ao agente para ele retomar a tarefa.
pub const RESUME_PROMPT: &str = "continue";
/// Quanto depois do reset o `continue` vai, em segundos.
pub const RESUME_DELAY: i64 = 5 * 60;
/// Sem o horário do reset, quanto esperar para tentar de novo.
pub const RETRY_WITHOUT_RESET: i64 = 30 * 60;

/// A mensagem de erro é a do limite de uso (ex.: `You've hit your limit ·
/// resets 3pm` ou o antigo `Claude AI usage limit reached|1764356400`).
pub fn is_limit_message(text: &str) -> bool {
    let text = text.to_ascii_lowercase();
    ["hit your limit", "usage limit reached"]
        .iter()
        .any(|words| text.contains(words))
}

/// Quando o limite reseta (segundos Unix), lido da mensagem de erro: o
/// número depois do `|` ou o `resets 3pm` / `resets 3:30pm (fuso)`, a
/// primeira vez que esse horário chega depois de `after`. Sem o fuso na
/// mensagem, vale o do sistema.
pub fn reset_from_message(text: &str, after: i64) -> Option<i64> {
    if let Some((_, epoch)) = text.rsplit_once('|')
        && let Ok(epoch) = epoch.trim().parse::<i64>()
    {
        return Some(epoch);
    }
    let lower = text.to_ascii_lowercase();
    let start = lower.find("resets ")? + "resets ".len();
    let (time, used) = parse_time(&lower[start..])?;
    let rest = text[start + used..].trim_start();
    let tz = rest
        .strip_prefix('(')
        .and_then(|rest| rest.split_once(')'))
        .and_then(|(name, _)| TimeZone::get(name.trim()).ok())
        .unwrap_or_else(TimeZone::system);
    let after = Timestamp::from_second(after).ok()?.to_zoned(tz.clone());
    let mut at = after.date().to_datetime(time).to_zoned(tz.clone()).ok()?;
    if at.timestamp() < after.timestamp() {
        at = after
            .date()
            .tomorrow()
            .ok()?
            .to_datetime(time)
            .to_zoned(tz)
            .ok()?;
    }
    Some(at.timestamp().as_second())
}

/// `3pm`, `3:30pm`, `15:00`: o horário e quantos bytes ele ocupa.
fn parse_time(text: &str) -> Option<(Time, usize)> {
    let digits = |s: &str| s.find(|c: char| !c.is_ascii_digit()).unwrap_or(s.len());
    let n = digits(text);
    let mut hour: i8 = text[..n].parse().ok()?;
    let mut used = n;
    let mut minute: i8 = 0;
    if let Some(rest) = text[used..].strip_prefix(':') {
        let m = digits(rest);
        minute = rest[..m].parse().ok()?;
        used += 1 + m;
    }
    let rest = &text[used..];
    let space = rest.len() - rest.trim_start().len();
    if rest.trim_start().starts_with("am") {
        hour %= 12;
        used += space + 2;
    } else if rest.trim_start().starts_with("pm") {
        hour = hour % 12 + 12;
        used += space + 2;
    }
    Some((Time::new(hour, minute, 0, 0).ok()?, used))
}

/// Quando mandar o `continue`: [`RESUME_DELAY`] depois do reset (ou de
/// agora, se ele já passou) ou, sem o reset, [`RETRY_WITHOUT_RESET`] depois
/// de agora.
pub fn resume_at(resets_at: Option<i64>, now: i64) -> i64 {
    match resets_at {
        Some(reset) => reset.max(now) + RESUME_DELAY,
        None => now + RETRY_WITHOUT_RESET,
    }
}

/// O aviso no chat, com a hora do `continue` (ex.: `15:05`).
pub fn notice(at: i64) -> String {
    format!(
        "Limite de uso atingido. O terra manda `{RESUME_PROMPT}` sozinho às {}, 5 min depois \
         de o limite resetar. Se você ainda está com o terminal aberto: quando o tempo \
         acabar, digite `continue` (ou `retome a tarefa anterior`). Ele vai ler os arquivos \
         que já modificou e continuar os passos seguintes.",
        clock(at)
    )
}

/// A hora local de `at`; com o dia, quando não é hoje.
fn clock(at: i64) -> String {
    let tz = TimeZone::system();
    let Ok(at) = Timestamp::from_second(at).map(|t| t.to_zoned(tz.clone())) else {
        return at.to_string();
    };
    let today = Timestamp::now().to_zoned(tz).date();
    let format = if at.date() == today {
        "%H:%M"
    } else {
        "%d/%m %H:%M"
    };
    at.strftime(format).to_string()
}

/// Agora, em segundos Unix.
pub fn now() -> i64 {
    Timestamp::now().as_second()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_reset_comes_from_the_limit_message() {
        assert!(is_limit_message("You've hit your limit · resets 3pm"));
        assert!(!is_limit_message("erro do claude: error_during_execution"));

        // 2026-11-28 12:00 em São Paulo (UTC-3).
        let noon = 1_795_878_000;
        let sp = "You've hit your limit · resets 3pm (America/Sao_Paulo)";
        assert_eq!(reset_from_message(sp, noon), Some(noon + 3 * 3600));
        let half = "You've hit your limit · resets 3:30pm (America/Sao_Paulo)";
        assert_eq!(reset_from_message(half, noon), Some(noon + 3 * 3600 + 1800));
        let tomorrow = "You've hit your limit · resets 11am (America/Sao_Paulo)";
        assert_eq!(reset_from_message(tomorrow, noon), Some(noon + 23 * 3600));
        let old = "Claude AI usage limit reached|1795888800";
        assert_eq!(reset_from_message(old, noon), Some(1_795_888_800));
    }

    #[test]
    fn the_continue_goes_five_minutes_after_the_reset() {
        assert_eq!(resume_at(Some(1_000), 500), 1_300);
        assert_eq!(resume_at(Some(1_000), 2_000), 2_300);
        assert_eq!(resume_at(None, 500), 500 + RETRY_WITHOUT_RESET);
    }
}

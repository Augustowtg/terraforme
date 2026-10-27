//! Tema visual (Catppuccin).
//!
//! Todas as cores da aplicação saem daqui. Para trocar de variante basta
//! alterar [`FLAVOR`].

use catppuccin::{ColorName, FlavorName, PALETTE};
use crossterm::style::{Color as TermColor, Stylize};
use ratatui::style::{Color, Modifier, Style};

/// Variante ativa do Catppuccin.
pub const FLAVOR: FlavorName = FlavorName::Mocha;

/// Marcador exibido à esquerda do item em foco.
pub const FOCUS_MARKER: &str = "▌";

fn color(name: ColorName) -> Color {
    PALETTE[FLAVOR][name].into()
}

/// Fundo da tela e texto normal.
pub fn screen() -> Style {
    Style::new()
        .bg(color(ColorName::Base))
        .fg(color(ColorName::Text))
}

pub fn text() -> Style {
    Style::new().fg(color(ColorName::Text))
}

pub fn section_title() -> Style {
    Style::new()
        .fg(color(ColorName::Mauve))
        .add_modifier(Modifier::BOLD)
}

pub fn path() -> Style {
    Style::new().fg(color(ColorName::Subtext0))
}

/// Linha inteira do item em foco (fundo + texto).
pub fn focused() -> Style {
    Style::new()
        .bg(color(ColorName::Surface1))
        .fg(color(ColorName::Lavender))
        .add_modifier(Modifier::BOLD)
}

pub fn show_more() -> Style {
    Style::new().fg(color(ColorName::Blue))
}

/// Itens vazios ou com path inexistente.
pub fn muted() -> Style {
    Style::new().fg(color(ColorName::Overlay0))
}

pub fn error() -> Style {
    Style::new().fg(color(ColorName::Red))
}

/// Aba ativa da barra de navegação: mesmo destaque do item em foco.
pub fn tab_active() -> Style {
    focused()
}

/// Abas inativas e "Sair": texto secundário, sem fundo.
pub fn tab_inactive() -> Style {
    path()
}

/// Botão "Sair" da barra de navegação: sempre vermelho.
pub fn quit() -> Style {
    error()
}

/// Botão "Sair" em foco (depois do `Esc`): fundo vermelho.
pub fn quit_focused() -> Style {
    button(ColorName::Red)
}

/// Botão "Sim" das confirmações: verde, preenchido quando selecionado.
pub fn confirm_yes(selected: bool) -> Style {
    choice(ColorName::Green, selected)
}

/// Botão "Não" das confirmações: vermelho, preenchido quando selecionado.
pub fn confirm_no(selected: bool) -> Style {
    choice(ColorName::Red, selected)
}

/// Botão "Sem cache" do gráfico de consumo: verde, preenchido quando escolhido.
pub fn without_cache(selected: bool) -> Style {
    choice(ColorName::Green, selected)
}

/// Botão "Com cache" do gráfico de consumo: vermelho, preenchido quando escolhido.
pub fn with_cache(selected: bool) -> Style {
    choice(ColorName::Red, selected)
}

fn choice(name: ColorName, selected: bool) -> Style {
    if selected {
        button(name)
    } else {
        Style::new().fg(color(name))
    }
}

fn button(name: ColorName) -> Style {
    Style::new()
        .bg(color(name))
        .fg(color(ColorName::Base))
        .add_modifier(Modifier::BOLD)
}

/// Variantes de cor de badges e estados. Todas saem da paleta Catppuccin.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Variant {
    /// Mesmo destaque do pill ativo da navbar.
    Active,
    Secondary,
    Accent,
    Success,
    Warning,
    Danger,
}

/// Cor de texto da variante, sem fundo.
pub fn variant(variant: Variant) -> Style {
    let name = match variant {
        Variant::Active => ColorName::Lavender,
        Variant::Secondary => ColorName::Subtext0,
        Variant::Accent => ColorName::Mauve,
        Variant::Success => ColorName::Green,
        Variant::Warning => ColorName::Yellow,
        Variant::Danger => ColorName::Red,
    };
    Style::new().fg(color(name))
}

/// Texto do badge: a variante `Active` usa o pill da navbar; as outras, só a cor.
pub fn badge_text(variant: Variant) -> Style {
    match variant {
        Variant::Active => tab_active(),
        other => self::variant(other).add_modifier(Modifier::BOLD),
    }
}

/// Borda do badge, na cor da variante.
pub fn badge_border(variant: Variant) -> Style {
    self::variant(variant)
}

/// Cor do contexto da sessão: verde, laranja da metade do limite e vermelho
/// a partir de 75% dele (`ratio` é a parte do limite ocupada).
fn context_color(ratio: f64) -> Color {
    let name = match ratio {
        r if r < 0.5 => ColorName::Green,
        r if r < 0.75 => ColorName::Peach,
        _ => ColorName::Red,
    };
    color(name)
}

/// Barra de contexto: a parte ocupada na cor do contexto.
pub fn context_bar(ratio: f64) -> Style {
    Style::new()
        .fg(context_color(ratio))
        .bg(color(ColorName::Surface0))
        .add_modifier(Modifier::BOLD)
}

/// Borda da barra de contexto, na cor dele.
pub fn context_border(ratio: f64) -> Style {
    Style::new().fg(context_color(ratio))
}

/// Cor do consumo da sessão pelo que resta (`left`, de 0 a 100): verde,
/// laranja de 50% para baixo, vermelho de 25% para baixo e cinza no 0%.
fn usage_color(left: f64) -> Color {
    let name = match left {
        l if l <= 0.0 => ColorName::Overlay0,
        l if l <= 25.0 => ColorName::Red,
        l if l <= 50.0 => ColorName::Peach,
        _ => ColorName::Green,
    };
    color(name)
}

/// Barra de consumo: a parte que resta na cor do consumo.
pub fn usage_bar(left: f64) -> Style {
    Style::new()
        .fg(usage_color(left))
        .bg(color(ColorName::Surface0))
        .add_modifier(Modifier::BOLD)
}

/// Borda da barra de consumo, na cor dele.
pub fn usage_border(left: f64) -> Style {
    Style::new().fg(usage_color(left))
}

/// Texto selecionado com o mouse: mesmo destaque do item em foco.
pub fn selection() -> Style {
    focused()
}

/// Badge de menção de arquivo no input (`@/src/main.rs`): pill cinza.
pub fn mention() -> Style {
    Style::new()
        .bg(color(ColorName::Surface1))
        .fg(color(ColorName::Subtext1))
}

/// Linha de atividade do agente ("Pensando...").
pub fn activity() -> Style {
    path().add_modifier(Modifier::ITALIC)
}

/// Borda da caixa de input: destaque no modo INSERT.
pub fn input_border(inserting: bool) -> Style {
    if inserting {
        variant(Variant::Active)
    } else {
        border()
    }
}

/// Bordas de blocos.
pub fn border() -> Style {
    muted()
}

/// Pinta uma mensagem de confirmação para o terminal (fora da TUI).
pub fn paint_success(msg: &str, colored: bool) -> String {
    paint(msg, ColorName::Green, colored)
}

/// Pinta uma mensagem de erro para o terminal (fora da TUI).
pub fn paint_error(msg: &str, colored: bool) -> String {
    paint(msg, ColorName::Red, colored)
}

fn paint(msg: &str, name: ColorName, colored: bool) -> String {
    if !colored {
        return msg.to_string();
    }
    let rgb = PALETTE[FLAVOR][name].rgb;
    msg.with(TermColor::Rgb {
        r: rgb.r,
        g: rgb.g,
        b: rgb.b,
    })
    .to_string()
}

//! A janela: o mascote animado conforme o estado das instâncias, o balão do
//! último aviso e o menu dos agentes abertos.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use eframe::egui::{
    self, Align2, Color32, ColorImage, CornerRadius, FontId, Pos2, Rect, Sense, Stroke,
    TextureHandle, TextureOptions, Vec2, ViewportCommand,
};
use terraforme_core::pet::{self, Activity, Instance, notify};

/// Intervalo entre as consultas ao daemon.
const POLL: Duration = Duration::from_secs(1);
/// Quadros por segundo da animação.
const FPS: f64 = 5.0;
/// Quadros em cada tira de sprites (`assets/*_strip.png`).
const FRAMES: usize = 4;
/// Tamanho de um quadro, em pixels da arte.
const FRAME: Vec2 = Vec2::new(48.0, 34.0);
/// Escala do pixel art na tela.
const SCALE: f32 = 3.0;
/// Ociosos há mais que isso, o mascote fica cansado.
const TIRED_AFTER: u64 = 10 * 60;
/// Largura da janela com o balão ou o menu aberto.
const WIDE: f32 = 290.0;
const BUBBLE_H: f32 = 34.0;
const ROW_H: f32 = 26.0;
const MENU_PAD: f32 = 8.0;

const BG: Color32 = Color32::from_rgb(0x2b, 0x28, 0x33);
const FG: Color32 = Color32::from_rgb(0xf3, 0xf0, 0xe8);
const MUTED: Color32 = Color32::from_rgb(0xa8, 0xa2, 0x96);

/// Animação do mascote, na ordem das tiras de `assets/`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mood {
    Sleeping,
    Happy,
    Tired,
    Attention,
    Notification,
    Danger,
}

impl Mood {
    fn index(self) -> usize {
        self as usize
    }
}

const STRIPS: [&[u8]; 6] = [
    include_bytes!("../../../assets/01_dormindo_strip.png"),
    include_bytes!("../../../assets/02_feliz_strip.png"),
    include_bytes!("../../../assets/03_cansado_strip.png"),
    include_bytes!("../../../assets/04_atencao_strip.png"),
    include_bytes!("../../../assets/05_notificacao_strip.png"),
    include_bytes!("../../../assets/06_perigo_strip.png"),
];

/// O mascote resume todas as instâncias: o caso mais urgente ganha.
fn mood(instances: &[Instance], now: u64) -> Mood {
    let any = |a: Activity| instances.iter().any(|i| i.activity == a);
    if instances.is_empty() {
        Mood::Sleeping
    } else if any(Activity::Waiting) {
        Mood::Danger
    } else if any(Activity::Done) {
        Mood::Notification
    } else if any(Activity::Working) {
        Mood::Attention
    } else {
        let last = instances.iter().map(|i| i.changed).max().unwrap_or(now);
        if now.saturating_sub(last) > TIRED_AFTER {
            Mood::Tired
        } else {
            Mood::Happy
        }
    }
}

/// Instâncias que acabaram de passar a pedir atenção (terminou ou precisa de
/// você) desde a consulta anterior.
fn new_alerts<'a>(before: &HashMap<String, Activity>, now: &'a [Instance]) -> Vec<&'a Instance> {
    now.iter()
        .filter(|i| i.activity.pending() && before.get(&i.id) != Some(&i.activity))
        .collect()
}

/// Texto do aviso de uma instância.
fn alert_text(i: &Instance) -> String {
    match i.activity {
        Activity::Waiting => format!("{} precisa de você", i.label()),
        _ => format!("{} terminou", i.label()),
    }
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

#[derive(Default)]
struct Shared {
    instances: Vec<Instance>,
    /// A instância do aviso mais recente que ainda está pendente.
    alert: Option<String>,
}

/// Consulta o daemon a cada [`POLL`] e avisa das instâncias que terminaram.
fn poll(shared: Arc<Mutex<Shared>>, ctx: egui::Context) {
    let mut before: HashMap<String, Activity> = HashMap::new();
    let mut first = true;
    loop {
        // Sem daemon (ou sem resposta), ninguém aberto: o mascote dorme.
        let instances = pet::instances().unwrap_or_default();
        // Na primeira consulta só lembra o estado, sem avisar o que já estava pendente.
        let alerts: Vec<_> = if first {
            Vec::new()
        } else {
            new_alerts(&before, &instances)
        };
        for i in &alerts {
            notify(&alert_text(i));
        }
        first = false;
        before = instances
            .iter()
            .map(|i| (i.id.clone(), i.activity))
            .collect();
        {
            let mut shared = shared.lock().unwrap();
            if let Some(i) = alerts.last() {
                shared.alert = Some(i.id.clone());
            }
            let still = |id: &String| {
                instances
                    .iter()
                    .any(|i| &i.id == id && i.activity.pending())
            };
            if !shared.alert.as_ref().is_some_and(still) {
                // O aviso foi visto: passa para outro que ainda esteja pendente.
                shared.alert = instances
                    .iter()
                    .filter(|i| i.activity.pending())
                    .max_by_key(|i| i.changed)
                    .map(|i| i.id.clone());
            }
            shared.instances = instances;
        }
        ctx.request_repaint();
        thread::sleep(POLL);
    }
}

struct Pet {
    shared: Arc<Mutex<Shared>>,
    strips: Vec<TextureHandle>,
    menu: bool,
    size: Vec2,
}

impl Pet {
    fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let strips = STRIPS
            .iter()
            .enumerate()
            .map(|(n, bytes)| {
                let img = image::load_from_memory(bytes)
                    .expect("sprite embutido inválido")
                    .to_rgba8();
                let size = [img.width() as usize, img.height() as usize];
                let color = ColorImage::from_rgba_unmultiplied(size, img.as_raw());
                // Pixel art: sem suavização na escala.
                cc.egui_ctx
                    .load_texture(format!("mascote-{n}"), color, TextureOptions::NEAREST)
            })
            .collect();
        let shared = Arc::new(Mutex::new(Shared::default()));
        let ctx = cc.egui_ctx.clone();
        let poller = shared.clone();
        thread::spawn(move || poll(poller, ctx));
        Self {
            shared,
            strips,
            menu: false,
            size: Vec2::ZERO,
        }
    }

    /// Pede ao terminal da instância para abrir a sub-aba dela.
    fn focus(id: String) {
        thread::spawn(move || {
            if let Err(e) = pet::focus(&id) {
                notify(&format!("não deu para abrir a instância: {e:#}"));
            }
        });
    }
}

/// Retângulo arredondado com texto, usado no balão e nas linhas do menu.
fn pill(ui: &egui::Ui, rect: Rect, text: &str, color: Color32, hover: bool) {
    let fill = if hover {
        Color32::from_rgb(0x3d, 0x39, 0x47)
    } else {
        BG
    };
    ui.painter().rect(
        rect,
        CornerRadius::same(8),
        fill,
        Stroke::new(1.0, Color32::from_rgb(0x4a, 0x45, 0x55)),
        egui::StrokeKind::Inside,
    );
    let text = truncate(text, ((rect.width() - 20.0) / 7.0) as usize);
    ui.painter().text(
        rect.left_center() + Vec2::new(10.0, 0.0),
        Align2::LEFT_CENTER,
        text,
        FontId::proportional(14.0),
        color,
    );
}

fn truncate(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    let cut: String = text.chars().take(max.saturating_sub(1)).collect();
    format!("{cut}…")
}

impl eframe::App for Pet {
    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        [0.0; 4]
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        let (instances, alert) = {
            let shared = self.shared.lock().unwrap();
            let alert = shared
                .alert
                .as_ref()
                .and_then(|id| shared.instances.iter().find(|i| &i.id == id))
                .cloned();
            (shared.instances.clone(), alert)
        };
        let mood = mood(&instances, now());

        let mascot = FRAME * SCALE;
        let wide = self.menu || alert.is_some();
        let width = if wide { WIDE } else { mascot.x };
        let mut y = 0.0;

        // Balão com o aviso mais recente: um clique vai à instância.
        if let Some(i) = &alert {
            let rect = Rect::from_min_size(Pos2::new(0.0, y), Vec2::new(width, BUBBLE_H - 4.0));
            let response = ui.allocate_rect(rect, Sense::click());
            let color = if i.activity == Activity::Waiting {
                Color32::from_rgb(0xff, 0xb4, 0x5a)
            } else {
                FG
            };
            pill(
                ui,
                rect,
                &format!("{} {}", i.activity.icon(), alert_text(i)),
                color,
                response.hovered(),
            );
            if response.clicked() {
                Self::focus(i.id.clone());
                self.menu = false;
            }
            y += BUBBLE_H;
        }

        // O mascote: clique abre o menu, arrastar move a janela.
        let rect = Rect::from_min_size(Pos2::new(0.0, y), mascot);
        let response = ui.allocate_rect(rect, Sense::click_and_drag());
        let frame = (ui.input(|i| i.time) * FPS) as usize % FRAMES;
        let step = 1.0 / FRAMES as f32;
        let uv = Rect::from_min_max(
            Pos2::new(frame as f32 * step, 0.0),
            Pos2::new((frame + 1) as f32 * step, 1.0),
        );
        ui.painter()
            .image(self.strips[mood.index()].id(), rect, uv, Color32::WHITE);
        if response.drag_started() {
            ctx.send_viewport_cmd(ViewportCommand::StartDrag);
        }
        if response.clicked() || response.secondary_clicked() {
            self.menu = !self.menu;
        }
        response.on_hover_text(match instances.len() {
            0 => "Nenhum agente aberto".to_string(),
            1 => "1 agente aberto".to_string(),
            n => format!("{n} agentes abertos"),
        });
        y += mascot.y;

        // Menu dos agentes abertos em todos os terminais.
        if self.menu {
            y += MENU_PAD;
            let row =
                |y: f32| Rect::from_min_size(Pos2::new(0.0, y), Vec2::new(width, ROW_H - 3.0));
            if instances.is_empty() {
                pill(ui, row(y), "Nenhum agente aberto", MUTED, false);
                y += ROW_H;
            }
            for i in &instances {
                let rect = row(y);
                let response = ui.allocate_rect(rect, Sense::click());
                let color = if i.activity.pending() { FG } else { MUTED };
                pill(
                    ui,
                    rect,
                    &format!("{}  {}", i.activity.icon(), i.label()),
                    color,
                    response.hovered(),
                );
                if response.clicked() {
                    Self::focus(i.id.clone());
                    self.menu = false;
                }
                y += ROW_H;
            }
            let rect = row(y);
            let response = ui.allocate_rect(rect, Sense::click());
            pill(ui, rect, "✕  Sair", MUTED, response.hovered());
            if response.clicked() {
                ctx.send_viewport_cmd(ViewportCommand::Close);
            }
            y += ROW_H;
        }

        // A janela acompanha o conteúdo (o resto seria área transparente que
        // ainda captura o mouse).
        let size = Vec2::new(width, y).round();
        if size != self.size {
            self.size = size;
            ctx.send_viewport_cmd(ViewportCommand::InnerSize(size));
        }
        ctx.request_repaint_after(Duration::from_secs_f64(1.0 / FPS));
    }
}

pub fn run() -> eframe::Result {
    let size = FRAME * SCALE;
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("terraforme")
            .with_app_id("terraforme-pet")
            .with_inner_size(size)
            .with_decorations(false)
            .with_transparent(true)
            .with_resizable(false)
            .with_taskbar(false)
            .with_always_on_top(),
        ..Default::default()
    };
    eframe::run_native(
        "terraforme-pet",
        options,
        Box::new(|cc| Ok(Box::new(Pet::new(cc)))),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn instance(id: &str, activity: Activity, changed: u64) -> Instance {
        serde_json::from_value(serde_json::json!({
            "id": id, "handle": format!("web/claude-{id}"), "agent": "claude",
            "space": "web", "number": 1, "activity": activity, "changed": changed
        }))
        .unwrap()
    }

    #[test]
    fn the_most_urgent_instance_sets_the_mood() {
        let now = 10_000;
        assert_eq!(mood(&[], now), Mood::Sleeping);
        let idle = instance("1", Activity::Idle, now - 60);
        assert_eq!(mood(std::slice::from_ref(&idle), now), Mood::Happy);
        let old = instance("1", Activity::Idle, now - TIRED_AFTER - 1);
        assert_eq!(mood(&[old], now), Mood::Tired);
        let working = instance("2", Activity::Working, now);
        assert_eq!(mood(&[idle.clone(), working.clone()], now), Mood::Attention);
        let done = instance("3", Activity::Done, now);
        assert_eq!(
            mood(&[working.clone(), done.clone()], now),
            Mood::Notification
        );
        let waiting = instance("4", Activity::Waiting, now);
        assert_eq!(mood(&[done, waiting, working], now), Mood::Danger);
    }

    #[test]
    fn alerts_only_for_new_pending_states() {
        let before = HashMap::from([
            ("1".to_string(), Activity::Working),
            ("2".to_string(), Activity::Done),
        ]);
        let now = [
            instance("1", Activity::Done, 1),
            instance("2", Activity::Done, 1),
            instance("3", Activity::Waiting, 1),
            instance("4", Activity::Working, 1),
        ];
        let ids: Vec<_> = new_alerts(&before, &now)
            .iter()
            .map(|i| i.id.as_str())
            .collect();
        assert_eq!(ids, ["1", "3"]);
        assert_eq!(alert_text(&now[0]), "web · claude 1 terminou");
        assert_eq!(alert_text(&now[2]), "web · claude 1 precisa de você");
    }

    #[test]
    fn sprites_are_strips_of_four_frames() {
        for bytes in STRIPS {
            let img = image::load_from_memory(bytes).unwrap();
            assert_eq!(img.width() as f32, FRAME.x * FRAMES as f32);
            assert_eq!(img.height() as f32, FRAME.y);
        }
    }
}

//! `terraforme-pet` — janela flutuante com o mascote do terraforme (o
//! tatu-peba). Fica sobre as outras janelas, mostra pelo mascote o que os
//! agentes de todos os `terraforme cli` estão fazendo, avisa quando um termina
//! e, com um clique, abre o menu dos agentes para ir a um deles. Aberta por
//! `terraforme pet` (ou `[tui] pet`). Só no Linux por enquanto: o Windows está
//! no `ROADMAP.md`.

#[cfg(target_os = "linux")]
mod app;

#[cfg(target_os = "linux")]
fn main() {
    match terraforme_core::pet::claim() {
        Ok(true) => {}
        // Já existe uma janela aberta.
        Ok(false) => return,
        Err(e) => {
            eprintln!("terraforme-pet: {e:#}");
            std::process::exit(1);
        }
    }
    let result = app::run();
    terraforme_core::pet::release();
    if let Err(e) = result {
        eprintln!("terraforme-pet: {e}");
        std::process::exit(1);
    }
}

#[cfg(not(target_os = "linux"))]
fn main() {
    eprintln!(
        "terraforme-pet: a janela flutuante ainda não existe neste sistema — veja o ROADMAP.md"
    );
    std::process::exit(1);
}

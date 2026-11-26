# Terra Remote — App Mobile Android

Aplicativo mobile nativo em **Kotlin + Jetpack Compose** para monitorar e controlar o **Terraforme** de forma remota pelo celular.

---

## 📱 Funcionalidades

1. **Monitoramento ao Vivo (Monitor):**
   - Exibe todas as instâncias ativas do Terra no computador, agrupadas por workspace (ex.: `terraforme`, `anime`, etc.).
   - Status em tempo real com indicador visual: **RODANDO** (com animação pulsante azul) ou **PARADO** (verde).
   - Card detalhado do **que está sendo feito no momento** (pensamento do modelo, ferramenta em execução, subagentes).
   - Lista de abas e chats abertos em cada janela.

2. **Histórico e Detalhes de Execuções (Execuções):**
   - Lista de todas as execuções despachadas (`runs`), com status (`done`, `running`, `failed`, `queued`), timestamps, modelo e consumo de tokens.
   - Tela de **Detalhes da Execução**: inspeção de logs `stderr` e todos os eventos passo a passo de `output.jsonl`.

3. **Controle Remoto de Agentes (Controle):**
   - Seletor dinâmico de agentes configurados no `orchestrator.toml` (ex.: `terra-dev`, `backend`, `frontend`, etc.).
   - Caixa de texto para envio de instruções / prompts diretamente do celular.
   - Suporte a arquivo de referência (`--ref`) e tags.
   - Sugestões e templates de comandos rápidos com um toque.
   - Feedback em tempo real com o ID da mensagem despachada.

4. **Configuração de Conexão Rápida:**
   - Botão no cabeçalho para alternar entre conexão **USB via ADB** (`localhost:8765`) e **Wi-Fi** (IP da rede local).

---

## 🚀 Como Usar

### 1. Inicie o Servidor Remoto no Computador

No terminal do computador, dentro do projeto `terraforme`, execute:

```bash
cargo run -- remote
# ou se o binário já estiver instalado:
terra remote
```

O servidor iniciará escutando em `http://0.0.0.0:8765`.

---

### 2. Conectando o Celular

#### Opção A: Via Cabo USB (Recomendada / Mais Rápida)
Com o celular conectado ao computador via USB e a Depuração USB ativada:

```bash
adb reverse tcp:8765 tcp:8765
```

No aplicativo no celular, o endereço padrão `http://localhost:8765` se conectará diretamente ao computador!

#### Opção B: Via Wi-Fi (Mesma Rede Local)
1. Descubra o IP do seu computador na rede (ex.: `ip -4 addr show` ou `192.168.1.102`).
2. No app do celular, clique no ícone de engrenagem no topo e digite:
   `http://192.168.1.102:8765`

---

## 📦 Compilação e Instalação no Celular

Para compilar o APK e instalá-lo no celular conectado:

```bash
cd android
chmod +x gradlew

# Compilar o APK (release com R8, ~1 MB; o de debug passa de 16 MB)
./gradlew assembleRelease

# Instalar diretamente no celular conectado via ADB
adb install -r app/build/outputs/apk/release/app-release.apk

# Abrir o aplicativo no celular
adb shell am start -n com.terraforme.remote/.MainActivity
```

---

## 🛠️ Tecnologias Utilizadas

- **Kotlin 2.0.21**
- **Jetpack Compose (Material 3)**
- **Coroutines & StateFlow**
- **AndroidX Lifecycle & ViewModel**
- **Paleta Catppuccin Macchiato** harmonizada com o tema original do Terraforme TUI
- **Backend Rust (`terra remote`)**: Servidor HTTP REST leve embutido sem dependências externas pesadas.

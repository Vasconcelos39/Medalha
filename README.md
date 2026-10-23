# Medalha

Cliente desktop **não oficial** do [Medal](https://medal.tv) para **Linux e Windows**, escrito em Rust ([Tauri 2](https://tauri.app)).

- **Aba Medal**: o medal.tv embutido, com login persistente (feed, perfil, clipes na nuvem).
- **Aba Clipes**: biblioteca dos seus clipes locais, com miniaturas, player, filtro por jogo, renomear, mostrar na pasta e lixeira.
- **Replay buffer**: grava em segundo plano e, ao apertar o atalho (padrão `F8`), salva os últimos N segundos. Funciona no **Windows**, no **X11** e no **Wayland**.
- **Cortar clipes**: no player, marque o início e o fim (arrastando as alças ou com `I` / `O`) e salve o trecho como um clipe novo, sem reencodar.
- **Nome do jogo**: o clipe é salvo com o nome da janela em foco, ex. `Elden Ring_2026-10-23_21-00-00.mp4`.
- **Iniciar com o sistema** (minimizado na bandeja) e **som ao salvar**, nas configurações.

> O Medalha não é afiliado ao Medal.tv. O Medal não oferece API pública de upload, e o site não aceita envio de vídeos pelo navegador. Por isso o upload automático para a nuvem do Medal não é possível hoje: os clipes ficam na sua pasta local (padrão `Vídeos/Medalha`).

## Como a gravação funciona

| Sistema | Gravador | Captura | Áudio |
| --- | --- | --- | --- |
| Windows 10/11 | ffmpeg | `ddagrab` (Desktop Duplication), frames direto na GPU com NVENC/AMF | WASAPI loopback (som do sistema) + microfone, capturados em Rust |
| Linux X11 | gpu-screen-recorder, ou ffmpeg | KMS/X11, ou `x11grab` | PipeWire/PulseAudio |
| Linux Wayland | gpu-screen-recorder | xdg-desktop-portal (GNOME, KDE, Hyprland…) ou KMS | PipeWire/PulseAudio |

No modo **Automático**:
- **Linux**: usa o `gpu-screen-recorder` se ele estiver instalado. Sem ele, cai para o ffmpeg no X11. No Wayland ele é obrigatório.
- **Windows**: testa os encoders de GPU (NVENC → AMF → QSV) e usa o primeiro que funcionar, senão a CPU (x264).

Com o ffmpeg, o buffer é uma sequência de segmentos de 2 s reaproveitados em ciclo. Salvar um clipe só junta os segmentos num MP4, sem reencodar, então leva menos de um segundo.

## Instalação

Baixe o instalador da última versão em [Releases](https://github.com/Vasconcelos39/medalha/releases/latest):

| Sistema | Arquivo |
| --- | --- |
| Windows 10/11 | `Medalha_X.Y.Z_x64-setup.exe` |
| Debian, Ubuntu, Mint, Pop!_OS | `Medalha_X.Y.Z_amd64.deb` (`sudo apt install ./Medalha_*.deb`) |
| Fedora, openSUSE | `Medalha-X.Y.Z-1.x86_64.rpm` (no Fedora o `ffmpeg` vem do [RPM Fusion](https://rpmfusion.org)) |
| Qualquer distro | `Medalha_X.Y.Z_amd64.AppImage` (`chmod +x` e rode) |

Builds de cada commit ficam em [Actions](https://github.com/Vasconcelos39/medalha/actions).

### Publicar uma versão

Atualize a versão em `src-tauri/Cargo.toml` e `src-tauri/tauri.conf.json`, escreva a seção `## vX.Y.Z` no `CHANGELOG.md` e envie uma tag:

```bash
git tag v0.2.0
git push origin v0.2.0
```

O GitHub Actions compila no Windows e no Linux e publica a release com os instaladores e as notas do CHANGELOG.

### Dependências

**Windows**: o WebView2 já vem no Windows 11. Instale o ffmpeg:

```powershell
winget install Gyan.FFmpeg
```

**Linux** (Debian/Ubuntu):

```bash
sudo apt install ffmpeg libwebkit2gtk-4.1-0 gstreamer1.0-libav gstreamer1.0-plugins-good
```

Para Wayland (e para gravar melhor também no X11), instale o [gpu-screen-recorder](https://git.dec05eba.com/gpu-screen-recorder/about/). Ele está no AUR, no Flathub e em vários repositórios:

```bash
flatpak install flathub com.dec05eba.gpu_screen_recorder
```

`gstreamer1.0-libav` é necessário para o player embutido tocar H.264 no WebKitGTK.

## Nome do jogo

Ao salvar, o Medalha lê o título da janela em foco:

- **Windows**: qualquer jogo.
- **Linux X11**, **Hyprland** e **Sway**: qualquer jogo (usa `xprop`, `hyprctl` ou `swaymsg`).
- **GNOME/KDE no Wayland**: só jogos que rodam via XWayland, que são a maioria (Proton/Wine e boa parte dos nativos). Os outros ficam com o nome `Medalha_…`.

Navegadores, Discord e Steam viram só o nome do app. Dá para desligar em *Configurações → Nomear clipes com o jogo em foco*.

## Atalho no Wayland

No Wayland, um programa comum não consegue registrar atalhos globais. Crie um atalho no seu ambiente (GNOME: *Configurações → Teclado → Atalhos personalizados*; KDE: *Atalhos personalizados*) que rode:

```bash
medalha --save-clip
```

Outros comandos que você pode associar a atalhos ou scripts:

| Comando | Ação |
| --- | --- |
| `medalha --save-clip` | salva o replay agora |
| `medalha --toggle` | liga/desliga o buffer |
| `medalha --start` / `--stop` | liga / desliga o buffer |
| `medalha --show` | mostra a janela |
| `medalha --hidden` | inicia minimizado na bandeja |

Os comandos são enviados para a instância que já está aberta.

## Compilando

Pré-requisitos: [Rust](https://rustup.rs) e as [dependências do Tauri](https://tauri.app/start/prerequisites/). No Windows isso é o VS Build Tools com C++; no Linux, `libwebkit2gtk-4.1-dev libayatana-appindicator3-dev librsvg2-dev libxdo-dev libssl-dev`.

```bash
cd src-tauri
cargo run            # desenvolvimento
cargo install tauri-cli --version "^2" --locked
cargo tauri build    # instaladores (.exe / .deb / .rpm / .AppImage)
```

A interface é HTML/CSS/JS puro em `ui/` (sem npm nem bundler).

## Estrutura

```
src-tauri/src/
  main.rs            janela, abas, bandeja, atalhos, comandos da UI, CLI
  medal.rs           webview do medal.tv
  clips.rs           biblioteca local (miniaturas, corte, renomear, lixeira, watcher)
  game.rs            nome da janela em foco (Windows, X11, Hyprland, Sway)
  autostart.rs       iniciar com o sistema (registro Run / ~/.config/autostart)
  sound.rs           som de confirmação ao salvar
  media.rs           descoberta do ffmpeg e escolha de captura/encoder
  config.rs          configurações (JSON na pasta de config do sistema)
  recorder/
    mod.rs           ciclo de vida do replay buffer
    ffmpeg.rs        buffer segmentado com ffmpeg (Windows/X11)
    gsr.rs           gpu-screen-recorder (Linux, X11 e Wayland)
    audio_win.rs     loopback WASAPI + microfone → ffmpeg
ui/                  interface (abas, clipes, configurações)
```

## Licença

MIT

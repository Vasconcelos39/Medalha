# Medalha

Cliente desktop não oficial do [Medal](https://medal.tv) para Linux e Windows, feito em Rust com [Tauri 2](https://tauri.app).

Tem duas abas. A primeira é o medal.tv embutido, com login salvo. A segunda é a biblioteca dos clipes gravados no computador, com miniaturas, player, corte, filtro por jogo, renomear e lixeira.

Os clipes saem de um replay buffer: o Medalha grava a tela continuamente e, quando você aperta o atalho (`F8` por padrão), salva os últimos segundos. Um card no canto da tela confirma que o clipe foi salvo, sem tirar o foco do jogo. No Wayland o card precisa do protocolo layer-shell (KDE Plasma, Hyprland, Sway); no GNOME aparece uma notificação do sistema no lugar.

Quando sai uma versão nova, o botão "Atualizar para X" aparece no cabeçalho. No AppImage, no `.deb`, no `.rpm` e no instalador do Windows a atualização é feita pelo próprio app.

O Medalha não tem relação com a Medal.tv. O Medal não oferece API pública de upload e o site não aceita envio de vídeos pelo navegador, então os clipes ficam só na pasta local (`Vídeos/Medalha`).

## Instalação

Baixe da página de [releases](https://github.com/Vasconcelos39/Medalha/releases/latest):

- Windows 10/11: `Medalha_X.Y.Z_x64-setup.exe`
- Debian, Ubuntu, Mint, Pop!_OS: `Medalha_X.Y.Z_amd64.deb` (`sudo apt install ./Medalha_*.deb`)
- Fedora, openSUSE: `Medalha-X.Y.Z-1.x86_64.rpm` (no Fedora o `ffmpeg` vem do [RPM Fusion](https://rpmfusion.org))
- Qualquer distro: `Medalha_X.Y.Z_amd64.AppImage`

No Windows, instale também o ffmpeg:

```powershell
winget install Gyan.FFmpeg
```

No Linux, o `.deb` já puxa o ffmpeg e o GStreamer (usado pelo player). Para gravar no Wayland é preciso o [gpu-screen-recorder](https://git.dec05eba.com/gpu-screen-recorder/about/). Se ele não estiver instalado, a aba Clipes oferece instalar o Flatpak do Flathub. Também dá para instalar na mão:

```bash
flatpak install flathub com.dec05eba.gpu_screen_recorder
```

## Como a gravação funciona

| Sistema | Gravador | Áudio |
| --- | --- | --- |
| Windows | ffmpeg com `ddagrab` e encoder da GPU (NVENC, AMF ou Quick Sync), ou x264 | loopback WASAPI e microfone, capturados pelo próprio Medalha |
| Linux X11 | gpu-screen-recorder ou ffmpeg com `x11grab` | PipeWire/PulseAudio |
| Linux Wayland | gpu-screen-recorder, pelo portal do desktop ou KMS | PipeWire/PulseAudio |

Com o ffmpeg, o buffer é uma fila de segmentos de 2 segundos reaproveitados em ciclo. Salvar só junta os segmentos num MP4, sem reencodar, e leva menos de um segundo. Se a captura cair no meio do jogo (troca de resolução, UAC, tela bloqueada), o Medalha religa sozinho.

O nome do clipe vem da janela em foco na hora de salvar, por exemplo `Elden Ring_2026-10-23_21-00-00.mp4`. Funciona no Windows, no X11, no Hyprland, no Sway e, no GNOME/KDE com Wayland, com jogos que rodam via XWayland (Proton/Wine e boa parte dos nativos).

## Atalho no Wayland

No Wayland, o atalho global só funciona quando a janela em foco é XWayland. Para funcionar sempre, crie um atalho no sistema (GNOME: Configurações → Teclado → Atalhos personalizados; KDE: Configurações do sistema → Atalhos) que rode:

```bash
medalha --save-clip
```

Outros comandos, que vão para a instância já aberta:

| Comando | O que faz |
| --- | --- |
| `medalha --save-clip` | salva o replay agora |
| `medalha --toggle` | liga ou desliga o buffer |
| `medalha --start`, `medalha --stop` | liga, desliga |
| `medalha --show` | mostra a janela |
| `medalha --hidden` | abre minimizado na bandeja |

## Compilando

Precisa do [Rust](https://rustup.rs) e das [dependências do Tauri](https://tauri.app/start/prerequisites/): no Windows, o VS Build Tools com C++; no Linux, `libwebkit2gtk-4.1-dev libayatana-appindicator3-dev librsvg2-dev libxdo-dev libssl-dev`.

```bash
cd src-tauri
cargo run
```

Para gerar os instaladores:

```bash
cargo install tauri-cli --version "^2" --locked
cargo tauri build
```

A interface é HTML, CSS e JavaScript sem build, na pasta `ui/`. O código Rust fica em `src-tauri/src/`: `recorder/` cuida do replay buffer, `clips.rs` da biblioteca, `medal.rs` do webview do site, `overlay.rs` do card por cima do jogo, `update.rs` das atualizações e `main.rs` da janela, da bandeja e dos comandos.

## Publicando uma versão

Atualize a versão em `src-tauri/Cargo.toml` e `src-tauri/tauri.conf.json`, escreva a seção `## vX.Y.Z` no `CHANGELOG.md` e envie a tag:

```bash
git tag v0.2.2
git push origin v0.2.2
```

O GitHub Actions compila nas duas plataformas e publica a release com os instaladores e as notas do CHANGELOG.

## Licença

MIT

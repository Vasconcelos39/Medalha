# Mudanças

## v0.2.1

- **Instalar o gpu-screen-recorder com um clique**: no Wayland, se ele não estiver instalado, a aba Clipes mostra o botão *Instalar gpu-screen-recorder*, que baixa o Flatpak do Flathub e liga o buffer.
- O gpu-screen-recorder em Flatpak agora é encontrado direto pela pasta de instalação, sem depender do `flatpak info`.
- Programas do sistema (flatpak, ffmpeg, pactl, xprop…) rodam sem as variáveis do AppImage, que podiam fazê-los carregar bibliotecas de dentro dele.
- O ffmpeg e o gpu-screen-recorder também são procurados em `~/.local/bin` e no Homebrew (Bazzite), que os apps abertos pelo menu não têm no PATH.
- O diagnóstico mostra quando o Medalha está rodando como AppImage.

## v0.2.0

**Novidades**

- **Cortar clipes**: no player, arraste as alças (ou aperte `I` / `O`) para marcar o trecho e clique em *Salvar corte*. O corte vira um clipe novo, sem reencodar e sem mexer no original.
- **Nome do jogo no clipe**: ao salvar, o clipe recebe o nome da janela em foco (`Elden Ring_2026-10-23_21-00-00.mp4`). A aba Clipes ganhou um filtro por jogo. Funciona no Windows, no X11, no Hyprland, no Sway e com jogos XWayland no GNOME/KDE.
- **Iniciar com o sistema**: abre minimizado na bandeja quando você entra no Windows ou no Linux.
- **Som ao salvar**: um bipe curto confirma que o clipe foi salvo (dá para desligar nas configurações).

**Correções**

- "Mostrar na pasta" no Windows usava um caminho `\\?\C:\…` que o Explorer não entende.
- Avisos (erros e confirmações) ficavam escondidos atrás das janelas de configuração e do player.
- Miniaturas: os cards antigos ficavam observados para sempre depois de cada atualização da lista.
- Aba Clipes mostra um aviso quando a busca ou o filtro não encontram nada.
- No Wayland, as configurações explicam que o atalho global só funciona com jogos XWayland.

**Pacotes**

- O `.deb` agora instala o `gstreamer1.0-libav` e o `gstreamer1.0-plugins-good`, necessários para o player tocar os clipes.
- O AppImage traz o GStreamer junto.
- Instaladores publicados em Releases: `.exe` (Windows), `.deb`, `.rpm` e `.AppImage` (Linux).

## v0.1.0

- Primeira versão: abas Medal e Clipes, replay buffer com ffmpeg (Windows/X11) e gpu-screen-recorder (Linux X11/Wayland), atalho global, bandeja e CLI.

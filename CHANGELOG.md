# Mudanças

## v0.3.2

Proteções depois de um travamento do sistema num PC com Bazzite (KDE Plasma, Wayland) com o buffer ligado. A causa ainda está sendo investigada.

- Linux: se o Medalha terminar de repente com o buffer ligado (o computador travou, faltou luz, o app foi morto), na próxima abertura o buffer não liga sozinho. Um aviso explica o motivo, e um clique em "Buffer desligado" liga do mesmo jeito. Desligar o computador ou sair da sessão normalmente não conta.
- Novo arquivo `medalha.log` na pasta de cache, com o que o Medalha fez: buffer ligado e desligado, o comando do gpu-screen-recorder e as mensagens dele, clipes salvos, o card. Cada linha vai para o disco na hora, então sobrevive a um travamento. O caminho aparece em Configurações → Avançado.
- `medalha --no-buffer` abre o app sem ligar o buffer, mesmo com "Ligar o buffer ao abrir" marcado.

## v0.3.1

A v0.3.0 não chegou a ser publicada (a compilação no GitHub travou), então esta é a primeira versão com estas mudanças.

- Card "Clipe salvo" por cima do jogo, como o do Medal, com o nome do jogo e a duração. Não tira o foco do jogo. Também aparece quando um clipe não pôde ser salvo (por exemplo, com o buffer desligado).
  - Windows e Linux X11: canto superior direito do monitor principal.
  - Linux Wayland: via layer-shell, no KDE Plasma, Hyprland, Sway e outros compositores wlroots. O AppImage traz a biblioteca; no `.deb` ela vem como recomendação (`libgtk-layer-shell0`). No GNOME, que não tem o protocolo, o Medalha usa a notificação do sistema.
- Aviso de versão nova: o botão "Atualizar para X" aparece no cabeçalho, com as notas da versão.
  - AppImage: o arquivo é trocado pelo novo e o Medalha reabre sozinho.
  - `.deb` e `.rpm`: o pacote é instalado pedindo a senha do sistema, e o Medalha reabre.
  - Windows: o instalador abre depois que o Medalha fecha.
  - Dá para desligar a busca ou procurar na hora em Configurações → Sistema.
- Botão (i) na aba Clipes: explica que os clipes ficam no computador e que os da nuvem do Medal, inclusive os privados, aparecem na aba Medal, com a sua conta.
- Fechar a janela com "Fechar para a bandeja" desligado agora encerra o app de vez.

## v0.2.2

Correções:

- Linux: o site do Medal aparecia deslocado para baixo, com a interface ocupando metade da janela. O GTK empilha os dois webviews em vez de sobrepor, e agora a interface encolhe para a altura do cabeçalho quando a aba Medal está aberta.
- Linux: o gpu-screen-recorder em Flatpak grava numa pasta escondida dentro da pasta de clipes, que o sandbox dele consegue acessar.
- Clipes salvos às vezes saíam mais longos que a duração configurada quando o ffmpeg perdia frames. Agora os segmentos são escolhidos pelo horário, não pela contagem.
- O clipe e o corte são escritos num arquivo temporário e só aparecem na aba Clipes prontos. Antes a miniatura podia falhar e ficar em branco até reabrir o app.
- Ligar e desligar o buffer rápido, ou mudar as configurações enquanto ele iniciava, podia apagar os segmentos da sessão nova.
- Se a captura cair no meio do jogo, o buffer religa sozinho (até 5 tentativas seguidas).
- Quando a máquina não dá conta do fps configurado (comum em notebook na bateria), o Medalha avisa e sugere 30 fps ou uma resolução menor, em vez de gravar clipes travados sem explicação.
- Windows: o ffmpeg fecha junto com o Medalha, mesmo se o app travar ou for encerrado pelo Gerenciador de Tarefas.
- Salvar um clipe não trava mais a interface por um instante no Linux enquanto o nome do jogo é lido.
- Enter no "Renomear" e nas configurações confirmava como "Cancelar".
- Ao gravar um atalho novo, apertar o atalho atual salvava um clipe.
- Renomear mudando só maiúsculas e minúsculas dava "Já existe um clipe com esse nome" no Windows.
- Mudar as configurações depois de um erro no buffer não tentava ligar de novo.
- Terminais, editores e outros apps comuns não viram mais "nome do jogo" no arquivo.

Interface:

- Configurações reorganizadas em grupos, uma opção por linha. Gravador, encoder e caminho do ffmpeg ficaram em "Avançado".
- Visual mais sóbrio no cabeçalho e na biblioteca.

## v0.2.1

- No Wayland, se o gpu-screen-recorder não estiver instalado, a aba Clipes oferece instalar o Flatpak do Flathub e liga o buffer em seguida.
- O Flatpak do gpu-screen-recorder é encontrado direto pela pasta de instalação, sem depender do `flatpak info`.
- Programas do sistema (flatpak, ffmpeg, pactl, xprop) rodam sem as variáveis do AppImage, que podiam fazê-los carregar bibliotecas de dentro dele.
- O ffmpeg e o gpu-screen-recorder também são procurados em `~/.local/bin` e no Homebrew, que os apps abertos pelo menu não têm no PATH.
- O diagnóstico mostra quando o Medalha está rodando como AppImage.

## v0.2.0

- Corte de clipes no player: arraste as alças (ou aperte `I` e `O`) e salve o trecho como um clipe novo, sem reencodar e sem mexer no original.
- Nome do jogo no arquivo do clipe e filtro por jogo na aba Clipes.
- Iniciar com o sistema, minimizado na bandeja.
- Som curto ao salvar um clipe (dá para desligar).
- Correções: "Mostrar na pasta" no Windows, avisos escondidos atrás das janelas, miniaturas acumulando observadores, aviso de filtro sem resultados, dica do atalho no Wayland.
- O `.deb` instala o GStreamer que o player precisa, o AppImage traz o GStreamer junto e a release publica `.exe`, `.deb`, `.rpm` e `.AppImage`.

## v0.1.0

- Primeira versão: abas Medal e Clipes, replay buffer com ffmpeg (Windows, X11) e gpu-screen-recorder (Linux), atalho global, bandeja e comandos de linha de comando.

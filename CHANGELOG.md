# Mudanças

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

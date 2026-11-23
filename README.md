<p align="center">
  <img src="packaging/branding/nuvio-astra.png" width="120" alt="Nuvio Astra" />
</p>

<h1 align="center">Nuvio Astra</h1>

<p align="center">
  Fork não oficial do <a href="https://github.com/NuvioMedia/NuvioDesktop">Nuvio Desktop</a> para Windows, reescrito em <b>Rust + Tauri</b> para ser leve e rápido.<br/>
  Usa os mesmos addons do ecossistema Stremio e a mesma conta Nuvio.
</p>

<p align="center">
  <a href="../../releases/latest"><b>⬇️ Baixar a versão mais recente</b></a>
</p>

---

## Capturas de tela

| Início | Detalhes |
|---|---|
| ![Tela inicial](docs/screenshots/home.jpg) | ![Detalhes de um título](docs/screenshots/details.jpg) |
| **Pesquisa** | **TV ao vivo** |
| ![Pesquisa](docs/screenshots/search.jpg) | ![TV ao vivo com guia de programação](docs/screenshots/abacanais.png) |

<sub>Os catálogos, capas e informações exibidos vêm dos addons instalados pelo usuário. Nenhum conteúdo acompanha o aplicativo.</sub>

## Recursos

### Início e descoberta

- **Destaque em carrossel**, com os dois catálogos que você escolher.
- **Catálogos dos seus addons** em fileiras, que podem ser renomeadas, ocultadas e reordenadas.
- **Coleções da conta Nuvio**, com capas em GIF que animam ao passar o mouse.
- **Pesquisa** com resultados enquanto você digita e histórico.
- **Descobrir** na tela de pesquisa: filtros por tipo, catálogo e gênero dos addons, com mais títulos ao rolar.
- **Detalhes** com temporadas, episódios, elenco e trailers do YouTube.

### Player

- **Player embutido** (libmpv), com aceleração por hardware, faixas de áudio e controle de velocidade.
- **Legendas dentro do player:** do arquivo, da fonte e dos addons, com atraso, tamanho e posição.
- **Estilo das legendas:** fontes do sistema ou importadas, cores, contorno, sombra, fundo e alinhamento.
- **Pular introdução, recapitulação e créditos**, usando os capítulos do vídeo, IntroDB e AniSkip, sem precisar de chave. Também pode pular automaticamente.
- **Próximo episódio**, com a escolha da fonte por cima do vídeo.
- **Picture-in-picture** com controles.
- **Torrent / P2P** no próprio player, com reprodução antes do download terminar.

### Qualidade de imagem

- **Anime4K** com modos A, B, C, A+A, B+B e C+A, além de efeitos de nitidez, traço e redução de ruído.
- **Ajustes de cor** e melhoria para filmes e séries.
- **Movimento:** "Sem tremidas" (ajusta o ritmo ao monitor) ou "Fluido" (mistura quadros vizinhos).
- **NVIDIA RTX Video** Super Resolution e HDR, em placas e drivers compatíveis.

### Biblioteca, conta e perfis

- **Continuar assistindo** com o próximo episódio e porcentagem de assistido ajustável de 90% a 100%.
- **Favoritos e histórico** por perfil.
- **Perfis** com PIN e avatares.
- **Sincronização automática com a conta Nuvio:** perfis, addons, favoritos, progresso, ordem do Início e preferências compatíveis. As coleções são recebidas da conta.

### TV ao vivo e atualizações

- **TV ao vivo** com categorias, favoritos, filtro por addon e guia de programação (quando o addon informa).
- **Atualizações automáticas** pelo próprio app, com verificação de assinatura.

O que ainda não foi portado do Nuvio original está em **Configurações → Ainda não disponível**.

## Instalação

1. Baixe `Nuvio-Astra-<versão>-x64-setup.exe` na página de [Releases](../../releases/latest).
2. Execute o instalador. Não é preciso ser administrador.
3. Novas versões chegam pelo próprio aplicativo: um aviso no topo mostra a versão e as novidades.

**Requisitos:** Windows 10 ou 11 (64 bits) com o Microsoft Edge WebView2 (já incluído no Windows 11).

O instalador ainda não tem assinatura digital, então o Windows SmartScreen pode pedir confirmação na primeira vez: **Mais informações → Executar assim mesmo**. Os arquivos de cada versão têm checksums em `SHA256SUMS.txt`.

Na primeira abertura, o app pede a conta Nuvio e sincroniza sozinho (também dá para continuar sem conta). Quem já usa o Nuvio Desktop pode ter perfis, addons e biblioteca importados automaticamente; a senha e o login do Nuvio Desktop não são copiados, e os arquivos originais não são alterados. Os dados do Nuvio Astra ficam em `%APPDATA%\NuvioNextGen` e são mantidos em atualizações e desinstalações.

## Compilar a partir do código

```powershell
cargo test --workspace
cd apps\nuvio-desktop\web; npm ci; npm run build; cd ..\..\..
cargo build --release -p nuvio-desktop
powershell -NoProfile -ExecutionPolicy Bypass -File packaging\windows\build-release.ps1 -SkipBuild -LibMpv <caminho\libmpv-2.dll>
```

Configurações opcionais de build (servidor da conta, repositório de atualizações) estão em [`local.properties.example`](local.properties.example). Publicação e releases automáticas: [`GITHUB_PUBLISH.md`](GITHUB_PUBLISH.md). Detalhes técnicos: [`docs/`](docs/).

### Linux (experimental — fase 1 do port)

O mesmo código agora compila e roda no Linux com WebKitGTK (Tauri 2):

```bash
# dependências do sistema (Debian/Ubuntu)
sudo apt install libwebkit2gtk-4.1-dev build-essential curl wget file libxdo-dev \
    libssl-dev libayatana-appindicator3-dev librsvg2-dev

cargo test --workspace
cd apps/nuvio-desktop/web && npm ci && npm run build && cd ../../..
cargo build --release -p nuvio-desktop
./target/release/nuvio-desktop
```

O que já funciona: interface, catálogos, busca, detalhes, TV ao vivo, biblioteca, perfis,
configurações e conta Nuvio. Os dados ficam em `$XDG_DATA_HOME/NuvioNextGen`
(`~/.local/share/NuvioNextGen` por padrão) e a migração dos dados legados é feita na primeira abertura.

**Player e P2P no Linux:** o player incorpora o libmpv do sistema (`libmpv.so.2`) numa janela filha
X11 (XWayland mesmo em sessões Wayland), com `vo=gpu`, `gpu-context=x11egl` → `x11vk` e decodificação
por VA-API — o mesmo modelo do bridge Linux do Nuvio Desktop original. O motor P2P é o
`libnuvio_engine.so` original (libtorrent), incluído em `apps/nuvio-desktop/native/linux/engine/`;
a ABI C foi verificada contra os 23 símbolos esperados por `nuvio-p2p`.

A interface tem três idiomas (português original, inglês e árabe, com layout RTL). A escolha fica
em **Configurações → Perfil e dados → Idioma da interface** e é lembrada entre execuções; sem
escolha explícita, segue o idioma do sistema.

A sessão da conta é protegida por AES-256-GCM com uma chave por usuário
(`~/.local/share/NuvioAstra/.account.key`, permissão 0600) no lugar da DPAPI. Atualizações
automáticas e pacotes AppImage/.deb vêm na fase 4. Variáveis de ajuste: `NUVIO_LIBMPV`,
`NUVIO_MPV_GPU_CONTEXT`, `NUVIO_MPV_HWDEC`, `NUVIO_ENGINE_DLL`.

## Créditos e direitos autorais

**Nuvio Astra:** modificações e código novo © 2026 [ScottGzin](https://github.com/ScottGzin) e contribuidores, sob a GPL-3.0.

**Projeto original:** o Nuvio Astra é uma versão modificada, iniciada em novembro de 2026, do [Nuvio Desktop](https://github.com/NuvioMedia/NuvioDesktop), © equipe Nuvio / [NuvioMedia](https://github.com/NuvioMedia) e seus contribuidores, licenciado sob a GPL-3.0. Os direitos sobre o código original continuam com seus autores. O nome, a marca e o logotipo **Nuvio** pertencem aos seus respectivos donos.

### Componentes incluídos

| Componente | Autoria | Licença |
|---|---|---|
| [Nuvio Engine](https://github.com/NuvioMedia/nuvio-engine) (`nuvio_engine.dll`) | © NuvioMedia | GPL-3.0-or-later |
| [libtorrent](https://www.libtorrent.org), dentro do Nuvio Engine | © Arvid Norberg e contribuidores | BSD-3-Clause |
| [Boost](https://www.boost.org), dentro do Nuvio Engine | © autores do Boost | BSL-1.0 |
| [OpenSSL](https://www.openssl.org), dentro do Nuvio Engine | © autores do OpenSSL | Apache-2.0 |
| [try_signal](https://github.com/arvidn/try_signal), dentro do Nuvio Engine | © 2016 Arvid Norberg | BSD-3-Clause |
| [mpv / libmpv](https://mpv.io) (`libmpv-2.dll`) | © autores do mpv | GPL-2.0-or-later |
| [FFmpeg](https://ffmpeg.org), dentro do libmpv | © desenvolvedores do FFmpeg | GPL, nesta compilação |
| [Anime4K](https://github.com/bloc97/Anime4K) | © 2019–2021 bloc97 | MIT (`AutoDownscalePre`: Unlicense) |
| [Tauri](https://tauri.app) | © 2017–presente Tauri Apps Contributors | MIT ou Apache-2.0 |
| [React](https://react.dev) | © Meta Platforms, Inc. e afiliadas | MIT |

As demais bibliotecas usam licenças livres, como MIT, Apache-2.0, BSD, ISC, Zlib e Unicode-3.0. Os avisos de terceiros estão em [`packaging/licenses`](packaging/licenses).

### Serviços e dados de terceiros

Estes serviços não fazem parte do Nuvio Astra, não têm vínculo com o projeto e seguem os próprios termos:

- **[IntroDB](https://introdb.app):** tempos de introdução, recapitulação e créditos enviados pela comunidade ([termos](https://introdb.app/docs/terms)).
- **[AniSkip](https://api.aniskip.com/api-docs):** tempos de abertura e encerramento de animes enviados pela comunidade.
- **[ARM](https://github.com/BeeeQueue/arm-server)**, de BeeeQueue, com dados de [Fribb/anime-lists](https://github.com/Fribb/anime-lists): correspondência entre identificadores de animes.
- **YouTube:** reprodução de trailers. YouTube é uma marca da Google LLC.
- **Addons:** catálogos, capas, informações, fontes e legendas pertencem aos seus provedores e autores.

NVIDIA e RTX são marcas da NVIDIA Corporation. Windows e WebView2 são produtos da Microsoft Corporation.

## Aviso legal

- O Nuvio Astra **não é afiliado, patrocinado nem endossado** pela equipe do Nuvio.
- O aplicativo **não hospeda, distribui nem fornece** filmes, séries, canais ou qualquer outro conteúdo. Ele apenas exibe o que os addons escolhidos pelo usuário oferecem. Os addons são de terceiros e não fazem parte deste projeto.
- Cada usuário é responsável pelos addons que instala e pelo conteúdo que acessa, de acordo com as leis do seu país. Ao usar torrent/P2P, o seu endereço IP fica visível para outros participantes da rede.
- Marcas, capas e imagens que aparecem no aplicativo ou nas capturas pertencem aos seus respectivos donos.

## Privacidade

- A senha da conta Nuvio é enviada apenas ao servidor da conta e não é armazenada. A sessão fica criptografada neste computador pelo Windows (DPAPI).
- Com os botões de pular ativados, o app envia ao IntroDB, AniSkip ou ARM o identificador público do título e, para séries, a temporada ou o episódio. Como em qualquer acesso à internet, esses serviços veem o seu IP. Os botões podem ser desligados em **Configurações → Reprodução**.
- Os addons instalados recebem os pedidos de catálogo, fontes e legendas. Para legendas, isso pode incluir o nome, o tamanho e o identificador do arquivo de vídeo.

## Licença

[GNU General Public License v3.0](LICENSE), a mesma do Nuvio Desktop. Você pode usar, estudar, modificar e redistribuir o Nuvio Astra seguindo os termos dessa licença, mantendo os avisos de direitos autorais e disponibilizando o código-fonte. O programa é fornecido **sem qualquer garantia**. O código-fonte completo de cada versão está neste repositório.

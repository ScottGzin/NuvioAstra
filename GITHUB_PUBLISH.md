# Publicar o Nuvio Astra no GitHub — passo a passo

Este guia leva a pasta `Nuvio-Astra-GitHub-Upload` até um repositório público com build automática:
cada `push` na branch `main` compila o Windows x64, testa, assina e publica um **GitHub Release**, e
quem já tem o Nuvio Astra instalado recebe o aviso de atualização dentro do próprio aplicativo.

Nada aqui é feito automaticamente em seu nome: você cria o repositório, envia os arquivos e cadastra
os segredos.

---

## 0. As três pastas

| Pasta | O que é | Vai para o GitHub? |
|---|---|---|
| `Nuvio-Astra-GitHub-Upload` | Código-fonte, documentação, workflows, licenças | **Sim** — é o conteúdo do repositório |
| `Nuvio-Astra-Assets-Manuais` | `libmpv-2.dll` (115 MB, o player) | **Sim, uma vez**, como arquivo de um Release chamado `deps-libmpv` (passo 6) — nunca dentro do código |
| `Nuvio-Astra-ARQUIVOS-PRIVADOS` | Chave **privada** de assinatura das atualizações e a senha dela | **Nunca.** Só o conteúdo, colado nos *Secrets* (passo 3). Guarde uma cópia em local seguro (gerenciador de senhas, pendrive) |

> **Atenção à chave privada.** É ela que prova que uma atualização veio de você. Se for perdida, as
> instalações existentes deixam de aceitar novas versões (seria preciso reinstalar manualmente com uma
> chave nova). Se vazar, alguém poderia assinar uma atualização falsa: nesse caso troque a chave,
> publique uma versão nova e avise os usuários. A chave **não** é gerada de novo a cada versão: é
> sempre a mesma.

---

## 1. Pré-requisitos

- Uma conta no GitHub (https://github.com).
- **GitHub Desktop** (https://desktop.github.com) — o jeito mais simples de enviar a pasta. Quem
  preferir pode usar o `git` pela linha de comando (comandos no passo 4).
- Não é preciso instalar Rust, Node ou NSIS para publicar: a compilação acontece no GitHub Actions.

---

## 2. Criar o repositório público

1. No GitHub, clique em **+** → **New repository**.
2. **Repository name**: por exemplo `Nuvio-Astra`.
3. **Public**.
4. **Não** marque *Add a README*, *.gitignore* nem *license* (a pasta já tem tudo isso).
5. **Create repository**. Anote o endereço: `https://github.com/<seu-usuario>/<nome-do-repositorio>`.

---

## 3. Cadastrar Secrets e Variables (antes do primeiro envio)

No repositório: **Settings → Secrets and variables → Actions**.

### Aba *Secrets* → **New repository secret**

| Nome (exato) | Valor | Obrigatório |
|---|---|---|
| `TAURI_SIGNING_PRIVATE_KEY` | Todo o texto do arquivo `Nuvio-Astra-ARQUIVOS-PRIVADOS\nuvio-astra-updater.key` (abra no Bloco de Notas, copie tudo) | **Sim** |
| `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` | Todo o texto de `Nuvio-Astra-ARQUIVOS-PRIVADOS\nuvio-astra-updater.key.password.txt` | **Sim** |
| `NUVIO_SUPABASE_URL` | O valor de `NUVIO_SUPABASE_URL` do seu `local.properties` | Não — sem ele a build sai sem login/sincronização |
| `NUVIO_SUPABASE_ANON_KEY` | O valor de `NUVIO_SUPABASE_ANON_KEY` do seu `local.properties` (chave **pública** `anon`; a build recusa chaves de serviço) | Não (junto com o anterior) |

### Aba *Variables* → **New repository variable** (todas opcionais)

| Nome | Valor | Para quê |
|---|---|---|
| `NUVIO_SUPABASE_FALLBACK_URL` | O `NUVIO_SUPABASE_FALLBACK_URL` do `local.properties`, se existir | Endereço reserva do servidor da conta |
| `VERSION_PATCH_OFFSET` | Um número, ex.: `2` | Só se um dia precisar pular números de versão |

**Numeração das versões:** cada publicação é um passo depois da maior versão já publicada (ou da
versão em `apps/nuvio-desktop/tauri.conf.json`, se for maior). O último número vai até 9 e depois o
do meio avança: `0.2.8` → `0.2.9` → `0.3.0`. Para pular para uma série nova (ex.: `0.5.x`), coloque
`0.5.0` no `tauri.conf.json`: a próxima sai `0.5.1`.

A chave **pública** já está no código (`apps/nuvio-desktop/tauri.conf.json` → `plugins.updater.pubkey`)
e não precisa ser cadastrada.

### Permissão dos workflows

**Settings → Actions → General → Workflow permissions** → marque **Read and write permissions** →
**Save**. (O workflow também declara `contents: write`; isso garante que a configuração do
repositório não o bloqueie.)

---

## 4. Enviar os arquivos

### Com o GitHub Desktop

1. **File → Add local repository…** → escolha a pasta `Nuvio-Astra-GitHub-Upload`.
2. Ele dirá que a pasta não é um repositório: clique em **create a repository**, mantenha o nome e
   **Git ignore: None**, **License: None** (já existem) → **Create repository**.
3. Confira na lista da esquerda: **não** deve aparecer nenhum `.dll`, `.exe`, `.key`,
   `local.properties` nem pasta `target`/`dist` (o `.gitignore` já os exclui).
4. Escreva o resumo do commit (ex.: `Primeira versão pública`) → **Commit to main**.
5. **Repository → Repository settings → Remote**: cole
   `https://github.com/<seu-usuario>/<nome-do-repositorio>.git` → **Save**.
6. **Push origin** (ou **Publish branch**).

### Com git (alternativa)

```powershell
cd "C:\caminho\para\Nuvio-Astra-GitHub-Upload"
git init -b main
git add -A
git commit -m "Primeira versão pública"
git remote add origin https://github.com/<seu-usuario>/<nome-do-repositorio>.git
git push -u origin main
```

---

## 5. O primeiro workflow (falha esperada)

O envio dispara **Actions → Release (Windows x64)**. Nesta primeira vez ele **para** na etapa
*Native libraries* com a mensagem `libmpv-2.dll not found in release 'deps-libmpv'` — é esperado,
porque o arquivo do player ainda não foi enviado. Nenhuma versão é publicada quando algo falha.

---

## 6. Enviar o player (`libmpv-2.dll`) uma única vez

O arquivo tem 115 MB (acima do limite de 100 MB por arquivo do código no GitHub), então ele fica
como anexo de um Release próprio, que o workflow baixa e confere pelo SHA-256.

1. No repositório: **Releases → Draft a new release**.
2. **Choose a tag** → digite `deps-libmpv` → **Create new tag: deps-libmpv on publish**.
3. **Release title**: `Dependência: libmpv (player)`.
4. Descrição (sugestão): `mpv v0.40.0-465-gf6c116491, a mesma build usada pelo Nuvio Desktop. Usada pela compilação; não é um instalador.`
5. Arraste `Nuvio-Astra-Assets-Manuais\libmpv-2.dll`.
6. Marque **Set as a pre-release** (assim ele nunca vira a "versão mais recente" que o aplicativo
   procura) e **desmarque** *Set as the latest release*.
7. **Publish release**.

SHA-256 esperado (conferido pelo workflow): `07c68bb211f23a218ded0a36eb12207dc3aeb44e5318ffca6ce9dcc7c3173906`.

O motor de torrent (`nuvio_engine.dll`) não precisa de nada: o workflow baixa a versão oficial
v0.1.6 do projeto NuvioMedia/nuvio-engine e confere o SHA-256.

---

## 7. Primeira publicação

1. **Actions → Release (Windows x64)** → abra a execução que falhou → **Re-run all jobs**.
2. Em ~15–25 minutos a execução fica verde. Etapas: segredos, versão, dependências, interface,
   testes, compilação, instalador, assinatura, arquivos da versão, publicação.
3. Em **Releases** aparece **Nuvio Astra 0.1.1** (o número final é o número da execução) com:
   - `Nuvio-Astra-0.1.1-x64-setup.exe` — instalador (também é o pacote de atualização);
   - `Nuvio-Astra-0.1.1-x64-setup.exe.sig` — assinatura;
   - `latest.json` — manifesto lido pelo aplicativo;
   - `SHA256SUMS.txt` — checksums;
   - notas da versão (mensagens dos commits).

Se qualquer etapa falhar, o release fica como rascunho ou nem é criado: nunca sai uma versão
incompleta.

---

## 8. O seu Nuvio Astra 0.1.0 atual

A versão 0.1.0 que você usa hoje foi compilada **antes** do atualizador existir, então ela não
consegue procurar atualizações sozinha. Faça **uma única vez** uma destas opções:

- **Opção A (recomendada):** baixe `Nuvio-Astra-0.1.1-x64-setup.exe` do seu Release e instale por
  cima. Ele usa a mesma pasta de instalação (`%LOCALAPPDATA%\Programs\Nuvio Astra`), não toca em
  `%APPDATA%\NuvioNextGen` e mantém perfis, addons, configurações, biblioteca e a sessão da conta.
- **Opção B (sem baixar do GitHub):** compile localmente uma build que já segue o seu repositório:
  crie `local.properties` a partir de `local.properties.example`, preencha
  `NUVIO_UPDATE_REPOSITORY=<seu-usuario>/<nome-do-repositorio>` (e os valores do Supabase), rode
  `packaging\windows\build-release.ps1` e instale o instalador gerado em `dist\`. Ela continua
  numerada 0.1.0 e recebe a 0.1.1 do GitHub pelo aviso de atualização.

Daí em diante, todas as versões chegam pelo próprio aplicativo.

---

## 9. Como as pessoas recebem as atualizações

- O aplicativo procura uma versão nova logo ao abrir (menos de 1 segundo) e a cada 6 horas
  (`https://github.com/<seu-usuario>/<repo>/releases/latest/download/latest.json`).
- Se houver, aparece um aviso discreto no topo: versão, **Novidades**, **Atualizar agora** e
  **Depois**. Também dá para verificar em **Configurações → Atualizações**.
- **Atualizar agora**: baixa o instalador mostrando o progresso, **confere a assinatura** com a
  chave pública embutida (um arquivo alterado ou assinado por outra chave é recusado e nada é
  instalado), fecha o player salvando a posição, instala por cima e abre o aplicativo de novo.
- A assinatura também amarra o número da versão (`requireSignedVersion`): um `latest.json`
  adulterado não consegue fazer o aplicativo voltar para uma versão antiga.

---

## 10. Publicar atualizações futuras

1. Faça as alterações no código (nesta pasta, que agora é o seu repositório).
2. Envie **tudo num único commit** (pelo site: **Add file → Upload files** e arraste as pastas
   alteradas juntas). Cada commit na `main` dispara uma compilação; vários commits seguidos fazem
   o GitHub cancelar as compilações da fila e publicar só a última.
3. Escreva as novidades no campo **descrição** do commit (*Extended description*), uma por linha:
   ```
   Ícones novos na barra lateral
   Painel de fontes à direita
   Pesquisa em fileiras
   ```
   O título do commit pode ser curto (ex.: `Nova versão`): quando há descrição, só ela entra nas
   notas. Sem descrição, entra o título. Títulos automáticos do GitHub ("Add files via upload",
   "Update …", "Create …", "Delete …") nunca entram.
4. O workflow publica a versão seguinte à última publicada (`0.1.6` → `0.1.7`, sem pular números,
   mesmo que alguma compilação falhe ou seja cancelada), e as notas aparecem no Release e no aviso
   de atualização do aplicativo.

Dicas:
- Para mudar o número "maior" (ex.: 0.2.x), altere `"version"` em
  `apps/nuvio-desktop/tauri.conf.json` para `0.2.0` e faça push: as próximas serão `0.2.<n>`.
- Para testar sem publicar, abra um *pull request*: o workflow **CI** só testa.
- Para publicar de novo sem mudar código: **Actions → Release → Run workflow**.

---

## 11. Problemas comuns

| Mensagem no Actions | O que fazer |
|---|---|
| `Secret TAURI_SIGNING_PRIVATE_KEY is missing` | Passo 3 (nome exato, conteúdo completo do arquivo) |
| `libmpv-2.dll not found in release 'deps-libmpv'` | Passo 6 (tag exatamente `deps-libmpv`) |
| `checksum mismatch` | O arquivo enviado não é o original; envie de novo o da pasta `Nuvio-Astra-Assets-Manuais` |
| `Tag v0.x.y already exists` | Crie a variável `VERSION_PATCH_OFFSET` com um número maior |
| Erro na etapa *Sign* | Senha errada no secret `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` |
| `Resource not accessible by integration` | Passo 3, *Workflow permissions* → Read and write |
| O aplicativo diz "Atualizações automáticas não estão configuradas" | Essa build não foi feita pelo GitHub Actions (passo 8, opção B) |

---

## 12. Licenças

O Nuvio Astra é GPL-3.0 (derivado do Nuvio Desktop, GPL-3.0). O instalador leva a pasta `licenses`
com a licença do projeto e os avisos de terceiros: Nuvio Engine (GPL-3.0, com libtorrent BSD-3,
Boost BSL-1.0, OpenSSL Apache-2.0) e libmpv/mpv (GPL). Como o código completo fica público neste
repositório e o motor/mpv têm fonte pública, as obrigações da GPL são atendidas; mantenha os links
de `packaging/licenses/THIRD_PARTY_NOTICES.md` corretos se trocar alguma dessas bibliotecas.

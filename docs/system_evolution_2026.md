# Atualizações do Sistema e Evolução Técnica (Novembro 2026)

Este documento registra a evolução arquitetural, as correções críticas de topologia transmidiática, o motor de difusão bidirecional em Rust e o sistema interativo de visualização de grafos implementados no `youmightlike`.

---

## 1. Visão Geral dos Avanços

O sistema evoluiu de um protótipo inicial para uma plataforma de recomendação cultural completa de alta fidelidade:
* **Escala do Grafo Semântico:** 3.587.306 arestas semânticas ativas no DuckDB, conectando mais de 34.000 filmes, 20.000 jogos, 38.000 livros, 3.25 milhões de artistas e 40.4 milhões de faixas.
* **Motor Bidirecional em Rust:** Algoritmo Personalized PageRank (PPR) paralelizado com Rayon, operando sobre representação CSR (*Compressed Sparse Row*) com espelhamento simétrico de arestas.
* **Interface Monocromática e Minimalista:** Arquitetura SPA moderna com FastAPI, tipografia JetBrains Mono, paleta sóbria calibrada (Studio Minimalist) e ausência total de emojis e ruídos visuais.
* **Visualização Topológica Interativa:** Renderizador canvas 2D ForceGraph com destaque de vizinhança de 1-hop, seleção múltipla (Shift+Click) e inspeção estrutural de pontes.

---

## 2. Motor de Difusão Bidirecional em Rust (`crates/engine`)

### 2.1 O Problema da Orientação das Arestas
No grafo original, arestas como `movie -> keyword` ou `artist -> tag` eram estritamente direcionadas do item para o conceito. Em caminhadas aleatórias (Random Walk / PageRank):
* A energia partindo de uma semente (ex.: *Donnie Darko*) fluía para a tag (`#cult`, `#soundtrack`).
* Porém, sem a aresta reversa (`tag -> movie`), a probabilidade ficava represada nas folhas conceituais e não retornava para outros filmes, álbuns ou jogos que compartilhavam a mesma tag.

### 2.2 Solução: Espelhamento Simétrico na Matriz CSR
No carregamento do grafo (`crates/engine/src/graph.rs`):
* Todas as arestas dirigidas $(u, v)$ recebem automaticamente o par reverso $(v, u)$ durante a montagem das listas de adjacência.
* A normalização estocástica $\sum_j P_{ij} = 1$ é calculada após a inclusão das arestas reversas, preservando a convergência matemática do PageRank.
* O cálculo do PPR agora difunde energia fluidamente entre itens de diferentes mídias através de conceitos compartilhados.

---

## 3. Topologia Transmidiática e Âncoras Estruturais

### 3.1 Correção do Bug de Desconexão Transmidiática (Ex.: Skyrim + Booka Shade)
* **Sintoma:** Ao combinar itens de domínios diferentes no perfil (ex.: o jogo *The Elder Scrolls V: Skyrim* e a dupla eletrônica *Booka Shade*), o cluster do Skyrim aparecia isolado no canvas, sem nenhuma aresta visível.
* **Causa Raiz:** O backend limitava os nós intermediários do grafo (*bridges*) a 8 slots globais e aceitava apenas os prefixos `keyword:` e `person:`. As tags da cena eletrônica consumiam todos os 8 slots, deixando de fora os nós de estúdio/desenvolvedor (`developer:Bethesda Game Studios`) e as tags do jogo (`keyword:rpg`). Sem a Bethesda no conjunto de nós exibidos, a query de arestas `WHERE source_id IN (nodes) AND target_id IN (nodes)` retornava 0 ligações para o Skyrim.
* **Implementação de Âncoras Estruturais por Semente:**
  - O extrator em `python/src/youmightlike/web/server.py` agora reserva **âncoras dedicadas para cada semente do perfil** (`seeds_resolved`), garantindo que seus conectores fundamentais (estúdio para jogos, diretor/compositor para filmes, gênero primário para música) estejam sempre presentes no subgrafo visual.
  - O parser de pontes foi expandido para suportar prefixos `developer:` e `author:`.
  - Resultado: o cluster do Skyrim conecta-se com precisão à Bethesda Game Studios e aos jogos da mesma árvore genealógica (*Morrowind*, *Oblivion*, *Fallout 3*, *Fallout 4*, *Starfield*).

### 3.2 Vínculos Diretos de Trilha Sonora (*Donnie Darko* ⇄ *The Killing Moon*)
* Criada a tabela canônica `movie_soundtrack_links` no DuckDB.
* O filme *Donnie Darko* (`movie:22697`) foi associado com peso semântico máximo diretamente a todas as instâncias catalogadas de *The Killing Moon* (`track:12883639` e `track:60808`) via aresta `soundtrack_track`, e à banda *Echo & the Bunnymen* (`artist:927`) via `soundtrack_featured`.
* As arestas de trilha sonora possuem prioridade de exibição e ganham realce visual imediato quando ambos os nós são selecionados.

---

## 4. Densificação Semântica de Tags (Meta: $\ge 10$ Tags por Entidade)

Para evitar que entidades de nicho fiquem isoladas ou com pesos desproporcionais, estabeleceu-se a política de enriquecimento sem perda de catálogo ("*apenas ir atrás, sem remover ninguém*"):
1. **Música Eletrônica & Tech House:** Ingestão de tags específicas para DJs e produtores da cena nacional e internacional (ex.: *Mochakk*, *Beltran*, *Vintage Culture*, *Booka Shade*, *Solomun*), garantindo rótulos como `#tech-house`, `#deep-house`, `#groove`, `#minimal`, `#underground`.
2. **Bandas e Artistas Históricos:** Enriquecimento de bandas clássicas com tags de subgênero, época e atmosfera (ex.: *Echo & the Bunnymen* passou de tags esparsas para 40 descritores como `#post-punk`, `#goth`, `#dark wave`, `#80s`, `#melancholic`, `#coldwave`).
3. **Resolução Priorizada de Nomes:** No autocomplete e resolução de sementes, entidades canônicas individuais (ex.: *Massive Attack*) recebem desempate positivo por relevância/grau para evitar que remixes ou colaborações ofusquem a entidade original.

---

## 5. Interface Gráfica e Interatividade do Grafo

### 5.1 Destaque de Vizinhança (Single & Multi-Node Highlighting)
A visualização em `python/src/youmightlike/web/static/index.html` adota uma lógica determinística de foco:
* **Seleção Simples (Clique):**
  - O nó clicado recebe anel duplo de destaque nas cores da paleta (`#e84855` coral e `#182825` ônix) e raio ampliado.
  - Vizinhos diretos (1-hop) permanecem com 100% de opacidade e rótulos de texto visíveis.
  - Todos os nós e arestas desconectados são atenuados para opacidade reduzida (`globalAlpha = 0.12`).
* **Seleção Múltipla (Shift + Clique):**
  - Permite inspecionar múltiplos nós simultaneamente (ex.: comparar *Donnie Darko* e *The Killing Moon* ou *Skyrim* e *Fallout 3*).
  - A união de vizinhos de todos os nós selecionados é preservada.
  - Arestas que conectam nós selecionados diretamente acendem em coral com largura triplicada e partículas animadas aceleradas.
* **Inspector Card:**
  - Painel flutuante no canto superior direito que lista os itens sob inspeção, a contagem de conexões mútuas e atalho para resetar o foco (`Esc` ou botão `[limpar]`).

### 5.2 Filtro por Tipo de Mídia na Busca
O campo de pesquisa do perfil inclui um seletor rápido de tipo de entidade (`all`, `movie`, `game`, `book`, `artist`, `track`), garantindo precisão ao buscar termos ambíguos (ex.: selecionar especificamente o jogo *Skyrim* em vez de faixas de sua trilha sonora).

### 5.3 Paleta de Cores e Tipografia
A interface utiliza uma paleta de 5 tons de alto contraste e sobriedade técnica:
* **Deep Onyx (`#182825`):** Textos principais, anéis estruturais e contornos.
* **Warm Eggshell (`#faf3dd`):** Fundos de cartões, badges e superfícies secundárias.
* **Sage Green (`#4e937a`):** Conectores conceituais, tags e pesos moderados.
* **Accent Coral (`#e84855`):** Destaque de foco, nós semente e arestas de alto fluxo.
* **Steel Blue (`#2660a4`):** Identificadores canônicos e nós de domínio artístico.
* **Tipografia:** `JetBrains Mono` em toda a hierarquia visual.
* **Padronização:** Remoção integral de emojis em logs de console, saídas CLI e elementos visuais, assegurando um padrão minimalista no estilo ferramentas Unix/Apple.

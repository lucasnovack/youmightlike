# Academic Foundations & Prior Art

`youmightlike` stands on the shoulders of seminal research in graph-based recommendation systems, stochastic random walks, link prediction, and high-performance computing. Rather than treating recommendation as a black-box machine learning model, the architecture leverages mathematically interpretable topological diffusion across a heterogeneous cultural graph.

This document formalizes the primary academic papers and industry systems that inspired the algorithms, data structures, and design decisions of `youmightlike`.

---

## 1. Real-Time Graph Walks & Multi-Seed Diffusion

### Pixie: A System for Recommending 3+ Billion Items to 200+ Million Users in Real-Time
* **Authors:** Chantat Eksombatchai, Pranav Jindal, Jerry Zitao Liu, Yuchen Liu, Rahul Sharma, Charles Sugnet, Mark Ulrich, Jure Leskovec (Pinterest & Stanford University)
* **Venue:** *Proceedings of the 2018 World Wide Web Conference (WWW '18)*, pp. 1775–1784.
* **DOI / Paper:** [https://doi.org/10.1145/3178876.3186046](https://doi.org/10.1145/3178876.3186046)
* **Direct Influence on `youmightlike`:**
  * **Multi-Hit Booster:** Pixie introduced non-linear aggregation for candidate nodes visited from multiple starting queries:
    $$\text{Score}(v) = \left( \sum_{q \in Q} \sqrt{p_q(v)} \right)^2$$
    This grants quadratic reinforcement to cultural works that lie at the intersection of diverse taste seeds (e.g. a film connecting *both* a music artist seed and a book seed), prioritizing multifaceted resonances over items strongly connected to only a single seed.
  * **Degree-Scaled Resource Allocation:** Scaling random walk exploration sub-linearly with node degree to prevent popular items from starving niche entities.
  * **In-Memory Contiguous Graph Topology:** Eliminating network hops in distributed clusters by keeping compacted adjacency lists in local RAM for sub-10ms response latencies.

---

## 2. Stochastic Diffusion & Personalized PageRank

### The PageRank Citation Ranking: Bringing Order to the Web
* **Authors:** Lawrence Page, Sergey Brin, Rajeev Motwani, Terry Winograd (Stanford University)
* **Publication:** Stanford InfoLab Technical Report (1999).
* **Paper:** [Stanford InfoLab Publication 1999-66](http://ilpubs.stanford.edu:8090/422/)

### Topic-Sensitive PageRank
* **Author:** Taher H. Haveliwala (Stanford University)
* **Venue:** *Proceedings of the 11th International Conference on World Wide Web (WWW '02)*, pp. 517–526.
* **DOI / Paper:** [https://doi.org/10.1145/511446.511513](https://doi.org/10.1145/511446.511513)

### Scaling Personalized Web Search
* **Authors:** Glen Jeh, Jennifer Widom (Stanford University)
* **Venue:** *Proceedings of the 12th International Conference on World Wide Web (WWW '03)*, pp. 271–279.
* **DOI / Paper:** [https://doi.org/10.1145/775152.775191](https://doi.org/10.1145/775152.775191)
* **Direct Influence on `youmightlike`:**
  * **Personalized Restart Vector ($s$):** Rather than calculating a global static centrality score, `youmightlike` restricts the teleport probability distribution $\alpha \cdot s$ strictly to user-selected taste seeds ($q \in Q$).
  * **Stationary Distribution Convergence:** Solved deterministically via power iteration:
    $$p_{t+1} = \alpha s + (1 - \alpha) p_t P$$
    guaranteeing conservation of stochastic mass ($\sum p_i = 1.0$) across cross-media categories.

---

## 3. Link Weighting & Hub Attenuation

### Friends and Neighbors on the Web
* **Authors:** Lada A. Adamic, Eytan Adar (HP Information Dynamics Lab)
* **Venue:** *Social Networks*, 25(3), pp. 211–230 (2003).
* **DOI / Paper:** [https://doi.org/10.1016/S0378-8733(03)00009-1](https://doi.org/10.1016/S0378-8733(03)00009-1)
* **Direct Influence on `youmightlike`:**
  * **Adamic-Adar / Inverse-Degree Penalization:** Without degree attenuation, ubiquitous nodes (e.g. mainstream genres like *Action*, *Rock*, *Comedy*) act as mathematical probability sinks, dominating recommendations and obscuring subtle cultural connections.
  * `youmightlike` weights transition probabilities inversely by target in-degree:
    $$w(u, v) = \frac{w_{\text{base}}(u, v)}{\ln(1 + \text{degree}(v))}$$
    ensuring that specific, auteur, and niche semantic tags preserve high conductance.

---

## 4. Heterogeneous Information Networks (HIN)

### PathSim: Meta Path-Based Top-K Similarity Search in Heterogeneous Information Networks
* **Authors:** Yizhou Sun, Jiawei Han, Xifeng Yan, Philip S. Yu, Tianyi Wu (UIUC & IBM T.J. Watson Research Center)
* **Venue:** *Proceedings of the VLDB Endowment (PVLDB)*, 4(11), pp. 992–1003 (2011).
* **DOI / Paper:** [https://doi.org/10.14778/3402707.3402736](https://doi.org/10.14778/3402707.3402736)
* **Direct Influence on `youmightlike`:**
  * **Cross-Media Meta-Paths:** Cultural affinity is inherently heterogeneous. `youmightlike` operationalizes multi-hop semantic bridges:
    * $\text{Movie} \xrightarrow{\text{soundtrack}} \text{Track} \xrightarrow{\text{performed\_by}} \text{Artist}$
    * $\text{Movie} \xrightarrow{\text{based\_on}} \text{Book} \xrightarrow{\text{written\_by}} \text{Author}$
    * $\text{Game} \xrightarrow{\text{shares\_tag}} \text{Concept} \xleftarrow{\text{shares\_tag}} \text{Anime}$
  * **Symmetric Adjacency:** Directed bipartite projections receive mirror pairs during CSR construction to enable bidirectional Markovian energy exchange.

---

## 5. Repulsive Random Walks & Signed Graphs

### Combating Web Spam with TrustRank
* **Authors:** Zoltán Gyöngyi, Hector Garcia-Molina, Jan Pedersen (Stanford University & Yahoo!)
* **Venue:** *Proceedings of the 30th International Conference on Very Large Data Bases (VLDB '04)*, pp. 576–587 (2004).
* **Direct Influence on `youmightlike`:**
  * **Negative Seed Damping (Bipolar PPR):** Adapting dual-diffusion concepts from anti-spam and signed graph analysis to user taste. When a user flags an unwanted aesthetic (anti-seed), an independent repulsive walk $p_{\text{neg}}$ is computed and subtracted from the positive preference vector:
    $$p_{\text{final}} = \max\left(0.0,\, p_{\text{pos}} - \beta \cdot p_{\text{neg}}\right)$$
    This zeroes out repulsive aesthetic neighborhoods without distorting orthogonal recommendations.

---

## 6. High-Performance Sparse Linear Algebra (CSR)

### Iterative Methods for Sparse Linear Systems
* **Author:** Yousef Saad (University of Minnesota)
* **Publisher:** Society for Industrial and Applied Mathematics (SIAM), 2nd Edition (2003).
* **Direct Influence on `youmightlike`:**
  * **Compressed Sparse Row (CSR) Representation:** The graph is flattened into contiguous memory arrays (`row_ptr`, `col_indices`, `weights`).
  * **Cache-Conscious Parallel SpMV:** Parallelized with Rayon across CPU worker threads, enabling sub-millisecond sparse matrix-vector multiplication directly against 3.9M+ edges without garbage collection pauses or heap fragmentation.

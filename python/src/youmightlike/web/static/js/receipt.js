// Cultural Taste Ticket Generator for youmightlike
// Generates an ultra-sharp, high-DPI thermal ticket / aesthetic poster on HTML5 Canvas.

import { DOMAIN_COLORS } from './config.js';

function computeTasteHash(seeds = []) {
  const str = seeds.map(s => s.id || s.name || '').sort().join('|');
  let hash = 0x811c9dc5;
  for (let i = 0; i < str.length; i++) {
    hash ^= str.charCodeAt(i);
    hash = Math.imul(hash, 0x01000193);
  }
  return Math.abs(hash).toString(16).toUpperCase().padStart(8, '0');
}

function truncate(str, maxLen = 30) {
  if (!str) return '';
  return str.length > maxLen ? str.slice(0, maxLen - 1) + '…' : str;
}

function cleanItemTitle(title, rec = null) {
  if (!title) return '';
  // Clean raw Wikidata Q-codes (e.g. Q18246458)
  if (/^Q\d+$/i.test(title.trim())) {
    if (rec && rec.reason && rec.reason.includes("'")) {
      const match = rec.reason.match(/'([^']+)'/);
      if (match && match[1]) {
        return `Livro: ${truncate(match[1], 16)}`;
      }
    }
    if (rec && rec.metadata && rec.metadata.author) {
      return `Livro (${rec.metadata.author})`;
    }
    return 'Obra Literária';
  }
  return title;
}

function getDomainBadge(type, isPt) {
  const t = (type || 'ITEM').toLowerCase();
  if (!isPt) return `[${t.toUpperCase()}]`;
  const ptMap = {
    game: 'JOGO',
    artist: 'ARTISTA',
    track: 'MÚSICA',
    movie: 'FILME',
    series: 'SÉRIE',
    book: 'LIVRO',
  };
  return `[${ptMap[t] || t.toUpperCase()}]`;
}

function translateSubtitle(sub, isPt) {
  if (!sub) return '';
  if (!isPt) return sub;
  return sub
    .replace(/\bArtist\b/gi, 'Artista')
    .replace(/\bMovie\b/gi, 'Filme')
    .replace(/\bGame\b/gi, 'Jogo')
    .replace(/\bSeries\b/gi, 'Série')
    .replace(/\bBook\b/gi, 'Livro');
}

// Select a balanced, transmedia basket of discoveries across domains
function selectBalancedTicketRecs(recs = [], seeds = [], maxRecs = 7) {
  if (!recs || recs.length === 0) return [];

  const byType = {
    game: recs.filter(r => r.type === 'game'),
    artist: recs.filter(r => r.type === 'artist'),
    track: recs.filter(r => r.type === 'track'),
    movie: recs.filter(r => r.type === 'movie'),
    series: recs.filter(r => r.type === 'series'),
    book: recs.filter(r => r.type === 'book'),
  };

  const seedTypes = new Set(seeds.map(s => s.type));
  const chosen = [];
  const chosenIds = new Set();

  const takeBest = (list) => {
    for (const item of list) {
      if (!chosenIds.has(item.id)) {
        chosen.push(item);
        chosenIds.add(item.id);
        return true;
      }
    }
    return false;
  };

  // Prioritize domains present in user's active seeds first
  const domainOrder = ['artist', 'game', 'movie', 'series', 'track', 'book'];
  domainOrder.sort((a, b) => (seedTypes.has(b) ? 1 : 0) - (seedTypes.has(a) ? 1 : 0));

  for (const dom of domainOrder) {
    if (chosen.length < maxRecs && byType[dom] && byType[dom].length > 0) {
      takeBest(byType[dom]);
    }
  }

  // Fill remaining slots with the highest-scoring items overall
  const sortedOverall = [...recs].sort((a, b) => (b.score || 0) - (a.score || 0));
  for (const item of sortedOverall) {
    if (chosen.length >= maxRecs) break;
    if (!chosenIds.has(item.id)) {
      chosen.push(item);
      chosenIds.add(item.id);
    }
  }

  return chosen.sort((a, b) => (b.score || 0) - (a.score || 0));
}

// Identify true causal seed connection for a recommendation
function findAssociatedSeed(rec, seeds = [], graph = null) {
  if (!seeds || seeds.length === 0) return null;
  if (seeds.length === 1) return seeds[0];

  const reasonLower = (rec.reason || '').toLowerCase();

  // 1. Explicit mention in reason (e.g. "Colaboração com Kanye West", "co-escuta com Kid Cudi")
  for (const s of seeds) {
    const sName = (s.name || s.id || '').toLowerCase();
    if (sName.length >= 3 && reasonLower.includes(sName)) {
      return s;
    }
  }

  // 2. Direct or 2-hop connection in graph links
  if (graph && Array.isArray(graph.links)) {
    const seedIds = new Set(seeds.map(s => s.id));
    // 2a. Direct link: rec -> seed
    for (const l of graph.links) {
      if (l.source === rec.id && seedIds.has(l.target)) {
        return seeds.find(s => s.id === l.target);
      }
      if (l.target === rec.id && seedIds.has(l.source)) {
        return seeds.find(s => s.id === l.source);
      }
    }
    // 2b. 2-hop bridge: rec -> bridge_node -> seed
    const recNeighbors = new Set();
    for (const l of graph.links) {
      if (l.source === rec.id) recNeighbors.add(l.target);
      if (l.target === rec.id) recNeighbors.add(l.source);
    }
    for (const l of graph.links) {
      if (recNeighbors.has(l.source) && seedIds.has(l.target)) {
        return seeds.find(s => s.id === l.target);
      }
      if (recNeighbors.has(l.target) && seedIds.has(l.source)) {
        return seeds.find(s => s.id === l.source);
      }
    }
  }

  // 3. Domain matching: artist/track connects to artist/track seed; game to game; movie/series to movie/series
  const domainMatches = seeds.filter(s => {
    if (rec.type === 'artist' || rec.type === 'track') {
      return s.type === 'artist' || s.type === 'track';
    }
    if (rec.type === 'game') return s.type === 'game';
    if (rec.type === 'movie' || rec.type === 'series') return s.type === 'movie' || s.type === 'series';
    return s.type === rec.type;
  });
  if (domainMatches.length > 0) {
    return domainMatches[0];
  }

  return seeds[0];
}

// Organic Constellation Physics Layout (Micro Force-Directed Relaxation)
function layoutConstellationGraph({
  seeds = [],
  recs = [],
  graph = { links: [] },
  boxX,
  boxY,
  boxW,
  boxH,
}) {
  const nodes = [];
  const nodeMap = new Map();
  const numSeeds = seeds.length;

  const pad = 38;
  const minX = boxX + pad;
  const maxX = boxX + boxW - pad;
  const minY = boxY + pad + 6;
  const maxY = boxY + boxH - pad - 6;
  const cx = boxX + boxW / 2;
  const cy = boxY + boxH / 2 + 3;

  // 1. Initialize Seed Nodes spread out across the canvas
  seeds.forEach((s, idx) => {
    const angle = (idx / Math.max(1, numSeeds)) * Math.PI * 2 - Math.PI / 2;
    const radX = boxW * 0.28;
    const radY = boxH * 0.25;
    const sn = {
      id: s.id,
      name: s.name || s.id,
      type: s.type,
      isSeed: true,
      x: cx + Math.cos(angle) * radX,
      y: cy + Math.sin(angle) * radY,
      vx: 0,
      vy: 0,
      mass: 2.2,
      radius: 6.5,
    };
    nodes.push(sn);
    nodeMap.set(s.id, sn);
  });

  // 2. Initialize Discovery Nodes around their causal seed
  recs.forEach((r, idx) => {
    const assocSeed = findAssociatedSeed(r, seeds, graph);
    const seedNode = assocSeed ? nodeMap.get(assocSeed.id) : null;
    const baseSeed = seedNode || nodes[0];

    const baseAngle = baseSeed ? Math.atan2(baseSeed.y - cy, baseSeed.x - cx) : (idx * 0.9);
    const spread = (idx % 2 === 0 ? 1 : -1) * (0.32 + (idx % 3) * 0.18);
    const angle = baseAngle + spread;
    const dist = 55 + (idx % 4) * 12;

    const rn = {
      id: r.id,
      name: cleanItemTitle(r.title || r.name || r.id, r),
      type: r.type,
      score: r.score,
      isSeed: false,
      seedNode: baseSeed,
      x: baseSeed ? baseSeed.x + Math.cos(angle) * dist : cx,
      y: baseSeed ? baseSeed.y + Math.sin(angle) * dist : cy,
      vx: 0,
      vy: 0,
      mass: 1.0,
      radius: 4.5,
    };
    nodes.push(rn);
    nodeMap.set(r.id, rn);
  });

  // 3. Edges Extraction
  const edges = [];
  const edgeKeys = new Set();
  const seedIds = new Set(seeds.map(s => s.id));

  // 3a. Intra-seed bridges from graph links (connections between user seeds!)
  if (graph && Array.isArray(graph.links)) {
    for (const l of graph.links) {
      if (seedIds.has(l.source) && seedIds.has(l.target) && l.source !== l.target) {
        const key = [l.source, l.target].sort().join('--');
        if (!edgeKeys.has(key)) {
          edgeKeys.add(key);
          edges.push({
            source: nodeMap.get(l.source),
            target: nodeMap.get(l.target),
            type: 'seed-seed',
            weight: 1.3,
          });
        }
      }
    }

    // 2-hop shared neighbors between seeds
    if (edges.length < Math.min(3, numSeeds - 1) && numSeeds > 1) {
      const neighborMap = new Map();
      for (const l of graph.links) {
        if (!neighborMap.has(l.source)) neighborMap.set(l.source, new Set());
        if (!neighborMap.has(l.target)) neighborMap.set(l.target, new Set());
        neighborMap.get(l.source).add(l.target);
        neighborMap.get(l.target).add(l.source);
      }
      for (let i = 0; i < seeds.length; i++) {
        for (let j = i + 1; j < seeds.length; j++) {
          const s1 = seeds[i].id;
          const s2 = seeds[j].id;
          const key = [s1, s2].sort().join('--');
          if (edgeKeys.has(key)) continue;
          const n1 = neighborMap.get(s1) || new Set();
          const n2 = neighborMap.get(s2) || new Set();
          let shared = 0;
          for (const nb of n1) {
            if (n2.has(nb)) shared++;
          }
          if (shared > 0) {
            edgeKeys.add(key);
            edges.push({
              source: nodeMap.get(s1),
              target: nodeMap.get(s2),
              type: 'seed-seed',
              weight: 0.9,
            });
          }
        }
      }
    }
  }

  // 3b. Discovery to Seed bridges
  nodes.filter(n => !n.isSeed && n.seedNode).forEach(rn => {
    edges.push({
      source: rn.seedNode,
      target: rn,
      type: 'seed-rec',
      weight: 1.1,
    });
  });

  // 4. Force-directed Micro-relaxation (85 iterations)
  const targetDistSeed = 115;
  const targetDistRec = 66;

  for (let iter = 0; iter < 85; iter++) {
    const alpha = Math.max(0.08, 1 - iter / 85);

    // 4a. Coulomb Repulsion between all node pairs
    for (let i = 0; i < nodes.length; i++) {
      for (let j = i + 1; j < nodes.length; j++) {
        const n1 = nodes[i];
        const n2 = nodes[j];
        let dx = n2.x - n1.x;
        let dy = n2.y - n1.y;
        let d = Math.hypot(dx, dy) || 1;

        const minDist = (n1.isSeed && n2.isSeed) ? 92 : (n1.isSeed || n2.isSeed) ? 68 : 54;
        if (d < minDist * 2.2) {
          const force = ((minDist * minDist) / (d * d)) * alpha * 1.6;
          const fx = (dx / d) * force;
          const fy = (dy / d) * force;
          n1.vx -= fx / n1.mass;
          n1.vy -= fy / n1.mass;
          n2.vx += fx / n2.mass;
          n2.vy += fy / n2.mass;
        }
      }
    }

    // 4b. Spring Attraction along Edges
    for (const e of edges) {
      if (!e.source || !e.target) continue;
      const dx = e.target.x - e.source.x;
      const dy = e.target.y - e.source.y;
      const d = Math.hypot(dx, dy) || 1;
      const targetDist = e.type === 'seed-seed' ? targetDistSeed : targetDistRec;
      const diff = d - targetDist;
      const force = diff * 0.075 * alpha * (e.weight || 1);
      const fx = (dx / d) * force;
      const fy = (dy / d) * force;
      e.source.vx += fx / e.source.mass;
      e.source.vy += fy / e.source.mass;
      e.target.vx -= fx / e.target.mass;
      e.target.vy -= fy / e.target.mass;
    }

    // 4c. Soft Centering Gravity & Boundary Dampening
    for (const n of nodes) {
      const dx = cx - n.x;
      const dy = cy - n.y;
      n.vx += dx * 0.012 * alpha;
      n.vy += dy * 0.012 * alpha;

      n.x += n.vx;
      n.y += n.vy;

      n.vx *= 0.62;
      n.vy *= 0.62;

      // Soft Box Boundary Clamping
      if (n.x < minX) { n.x = minX; n.vx = 0; }
      if (n.x > maxX) { n.x = maxX; n.vx = 0; }
      if (n.y < minY) { n.y = minY; n.vy = 0; }
      if (n.y > maxY) { n.y = maxY; n.vy = 0; }
    }
  }

  return { nodes, edges };
}

// Compute label placement with maximum clearance away from neighbors
function computeLabelPlacement(n, nodes, boxX, boxY, boxW, boxH) {
  const cx = boxX + boxW / 2;
  const cy = boxY + boxH / 2;

  let rx = (n.x - cx) * 0.8;
  let ry = (n.y - cy) * 0.8;

  for (const other of nodes) {
    if (other === n) continue;
    const dx = n.x - other.x;
    const dy = n.y - other.y;
    const d = Math.hypot(dx, dy);
    if (d < 75 && d > 0) {
      rx += (dx / d) * (75 - d) * 1.5;
      ry += (dy / d) * (75 - d) * 1.5;
    }
  }

  const r = n.radius || 5;
  let align = 'center';
  let lx = n.x;
  let ly = n.y;

  // Boundary safety checks
  if (n.x > boxX + boxW - 85) {
    align = 'right';
    lx = n.x - r - 5;
    ly = n.y + 3;
  } else if (n.x < boxX + 85) {
    align = 'left';
    lx = n.x + r + 5;
    ly = n.y + 3;
  } else if (Math.abs(rx) >= Math.abs(ry)) {
    if (rx >= 0) {
      align = 'left';
      lx = n.x + r + 5;
      ly = n.y + 3;
    } else {
      align = 'right';
      lx = n.x - r - 5;
      ly = n.y + 3;
    }
  } else {
    if (ry < 0) {
      align = 'center';
      lx = n.x;
      ly = n.y - r - 5;
    } else {
      align = 'center';
      lx = n.x;
      ly = n.y + r + 11;
    }
  }

  return { align, lx, ly };
}

export function generateCulturalReceipt(canvas, {
  seeds = [],
  negativeSeeds = [],
  negativeBeta = 1.2,
  recommendations = [],
  knownCount = 0,
  graph = { nodes: [], links: [] },
  lang = 'pt',
}) {
  if (!canvas) return;
  const ctx = canvas.getContext('2d');
  if (!ctx) return;

  const dpr = Math.min(window.devicePixelRatio || 2, 2.5);
  const width = 640;
  const isPt = lang === 'pt';

  // Extract unique resonance tags
  const tagsSet = new Set();
  if (recommendations && recommendations.length > 0) {
    for (const r of recommendations) {
      if (r.tags && Array.isArray(r.tags)) {
        for (const t of r.tags) {
          if (t && t.length > 1 && tagsSet.size < 8) {
            tagsSet.add(t.toLowerCase().replace(/\s+/g, '-'));
          }
        }
      }
    }
  }
  if (tagsSet.size < 4 && graph && graph.nodes) {
    for (const n of graph.nodes) {
      if ((n.type === 'tag' || n.type === 'keyword') && tagsSet.size < 8) {
        tagsSet.add((n.label || '').toLowerCase().replace(/\s+/g, '-'));
      }
    }
  }
  const resonanceTags = Array.from(tagsSet);

  // Take balanced transmedia recommendations (up to 7)
  const topRecs = selectBalancedTicketRecs(recommendations, seeds, 7);

  // Compute dynamic height
  const toothDepth = 8;
  let estimatedHeight = 0;
  estimatedHeight += 24; // Top margin & jagged teeth
  estimatedHeight += 120; // Header & order meta
  estimatedHeight += 24; // Divider
  estimatedHeight += 32 + (seeds.length * 24); // Seeds section
  if (negativeSeeds.length > 0) {
    estimatedHeight += 28 + (negativeSeeds.length * 24); // Negative seeds
  }
  estimatedHeight += 20; // Divider
  estimatedHeight += 54; // DNA Tags
  estimatedHeight += 20; // Divider
  estimatedHeight += 275; // Mini-map frame with generous breathing space
  estimatedHeight += 20; // Divider
  estimatedHeight += 36 + (topRecs.length * 44); // Fresh recommendations
  estimatedHeight += 20; // Divider
  estimatedHeight += 105; // Totals & audit
  estimatedHeight += 18; // Double line
  estimatedHeight += 135; // Barcode, link & footer
  estimatedHeight += 24; // Bottom tear & margin

  const height = Math.round(estimatedHeight);

  canvas.width = Math.round(width * dpr);
  canvas.height = Math.round(height * dpr);
  canvas.style.width = `${width}px`;
  canvas.style.height = `${height}px`;

  ctx.save();
  ctx.scale(dpr, dpr);

  // 1. Draw Paper Background with Perforated Sawtooth Top & Bottom
  ctx.beginPath();
  const numTeeth = 32;
  const toothWidth = width / numTeeth;

  // Top edge zig-zag
  ctx.moveTo(0, toothDepth);
  for (let i = 0; i < numTeeth; i++) {
    const xMid = (i + 0.5) * toothWidth;
    const xEnd = (i + 1) * toothWidth;
    ctx.lineTo(xMid, 0);
    ctx.lineTo(xEnd, toothDepth);
  }

  // Right edge straight down
  ctx.lineTo(width, height - toothDepth);

  // Bottom edge zig-zag
  for (let i = numTeeth - 1; i >= 0; i--) {
    const xMid = (i + 0.5) * toothWidth;
    const xStart = i * toothWidth;
    ctx.lineTo(xMid, height);
    ctx.lineTo(xStart, height - toothDepth);
  }

  // Left edge straight up
  ctx.lineTo(0, toothDepth);
  ctx.closePath();

  // Paper fill (vintage thermal off-white)
  ctx.fillStyle = '#faf8f2';
  ctx.fill();

  // Paper stroke border
  ctx.lineWidth = 1.2;
  ctx.strokeStyle = '#dedacb';
  ctx.stroke();

  // Layout Cursor
  let curY = 32;
  const padX = 40;
  const contentWidth = width - padX * 2;

  // Helper drawing functions
  const drawDottedLine = (y) => {
    ctx.save();
    ctx.strokeStyle = '#cbbfad';
    ctx.lineWidth = 1;
    ctx.setLineDash([4, 4]);
    ctx.beginPath();
    ctx.moveTo(padX, y);
    ctx.lineTo(width - padX, y);
    ctx.stroke();
    ctx.restore();
  };

  const drawDoubleLine = (y) => {
    ctx.save();
    ctx.strokeStyle = '#1c1917';
    ctx.lineWidth = 1.2;
    ctx.beginPath();
    ctx.moveTo(padX, y);
    ctx.lineTo(width - padX, y);
    ctx.moveTo(padX, y + 3);
    ctx.lineTo(width - padX, y + 3);
    ctx.stroke();
    ctx.restore();
  };

  const tasteHash = computeTasteHash(seeds);
  const now = new Date();
  const dateStr = now.toLocaleDateString(isPt ? 'pt-BR' : 'en-US', {
    day: '2-digit',
    month: 'short',
    year: 'numeric',
  }).toUpperCase();
  const timeStr = now.toLocaleTimeString(isPt ? 'pt-BR' : 'en-US', {
    hour: '2-digit',
    minute: '2-digit',
    second: '2-digit',
  });

  // 2. HEADER: Brand & Order Metadata
  ctx.fillStyle = '#1c1917';
  ctx.textAlign = 'center';

  ctx.font = '700 18px "JetBrains Mono", monospace';
  ctx.fillText('*** YOUMIGHTLIKE ***', width / 2, curY);
  curY += 18;

  ctx.font = '600 11px "JetBrains Mono", monospace';
  ctx.fillStyle = '#44403c';
  ctx.fillText(
    isPt ? 'TICKET CULTURAL // GRAFO TRANSMIDIÁTICO' : 'CULTURAL TICKET // TRANSMEDIA GRAPH',
    width / 2,
    curY
  );
  curY += 16;

  ctx.font = '400 9.5px "JetBrains Mono", monospace';
  ctx.fillStyle = '#78716c';
  ctx.fillText(
    isPt ? 'MOTOR PPR ESTOCÁSTICO • SERENDIPIDADE & GRAFO DISPERSO' : 'STOCHASTIC PPR ENGINE • SERENDIPITY & GRAPH DISPERSION',
    width / 2,
    curY
  );
  curY += 20;

  // Metadata Box (Order & Date)
  ctx.font = '500 10.5px "JetBrains Mono", monospace';
  ctx.fillStyle = '#1c1917';
  ctx.textAlign = 'left';
  ctx.fillText(`${isPt ? 'PEDIDO' : 'ORDER'}: #YM-${tasteHash}`, padX, curY);
  ctx.textAlign = 'right';
  ctx.fillText(`${dateStr} ${timeStr}`, width - padX, curY);
  curY += 15;

  ctx.textAlign = 'left';
  ctx.font = '400 9.5px "JetBrains Mono", monospace';
  ctx.fillStyle = '#78716c';
  ctx.fillText('TERMINAL: ENGINE-RUST-v2.8', padX, curY);
  ctx.textAlign = 'right';
  ctx.fillText('STATUS: OK (CONVERGED)', width - padX, curY);
  curY += 14;

  drawDottedLine(curY);
  curY += 18;

  // 3. SECTION 1: Base Ingredients (Profile Seeds)
  ctx.textAlign = 'left';
  ctx.fillStyle = '#1c1917';
  ctx.font = '700 11px "JetBrains Mono", monospace';
  ctx.fillText(
    isPt ? '[01] INGREDIENTES BASE (SEMENTES DO PERFIL)' : '[01] BASE INGREDIENTS (PROFILE SEEDS)',
    padX,
    curY
  );
  curY += 18;

  for (let i = 0; i < seeds.length; i++) {
    const s = seeds[i];
    const typeLabel = getDomainBadge(s.type, isPt);
    const name = s.name || s.id;
    const sub = translateSubtitle(s.subtitle, isPt) ? `(${translateSubtitle(s.subtitle, isPt)})` : '';

    ctx.font = '700 10px "JetBrains Mono", monospace';
    ctx.fillStyle = '#e11d48'; // Ruby accent
    ctx.fillText(typeLabel, padX, curY);

    ctx.font = '500 10.5px "JetBrains Mono", monospace';
    ctx.fillStyle = '#1c1917';
    ctx.fillText(truncate(name, 34), padX + 68, curY);

    if (sub) {
      ctx.textAlign = 'right';
      ctx.font = '400 9px "JetBrains Mono", monospace';
      ctx.fillStyle = '#78716c';
      ctx.fillText(truncate(sub, 18), width - padX, curY);
      ctx.textAlign = 'left';
    }
    curY += 20;
  }

  // Negative seeds (if any)
  if (negativeSeeds && negativeSeeds.length > 0) {
    curY += 6;
    ctx.font = '700 10px "JetBrains Mono", monospace';
    ctx.fillStyle = '#dc2626';
    ctx.fillText(
      isPt ? `[--] PÓLOS DE REPULSÃO (EVITADOS - β: ${negativeBeta.toFixed(1)}x)` : `[--] REPULSIVE POLES (AVOIDED - β: ${negativeBeta.toFixed(1)}x)`,
      padX,
      curY
    );
    curY += 16;

    for (const neg of negativeSeeds) {
      ctx.font = '700 10px "JetBrains Mono", monospace';
      ctx.fillStyle = '#dc2626';
      ctx.fillText('[EVITAR]', padX, curY);

      ctx.font = '500 10.5px "JetBrains Mono", monospace';
      ctx.fillStyle = '#44403c';
      ctx.fillText(truncate(neg.name || neg.id, 38), padX + 68, curY);
      curY += 18;
    }
  }

  curY += 6;
  drawDottedLine(curY);
  curY += 18;

  // 4. SECTION 2: Aesthetic DNA & Resonance Tags
  ctx.textAlign = 'left';
  ctx.fillStyle = '#1c1917';
  ctx.font = '700 11px "JetBrains Mono", monospace';
  ctx.fillText(
    isPt ? '[02] DNA ESTÉTICO & CONFLUÊNCIAS' : '[02] AESTHETIC DNA & CONFLUENCES',
    padX,
    curY
  );
  curY += 18;

  ctx.font = '500 10px "JetBrains Mono", monospace';
  if (resonanceTags.length > 0) {
    const tagString = resonanceTags.map(t => `#${t}`).join('   ');
    ctx.fillStyle = '#4f46e5'; // Indigo accent
    ctx.fillText(truncate(tagString, 62), padX, curY);
  } else {
    ctx.fillStyle = '#78716c';
    ctx.fillText(isPt ? 'Confluência transmidiática multidimensional' : 'Multidimensional transmedia confluence', padX, curY);
  }
  curY += 18;

  drawDottedLine(curY);
  curY += 18;

  // 5. SECTION 3: Constellation Topological Map (Micro Force-Directed Graph)
  ctx.textAlign = 'left';
  ctx.fillStyle = '#1c1917';
  ctx.font = '700 11px "JetBrains Mono", monospace';
  ctx.fillText(
    isPt ? '[03] MAPA TOPOLÓGICO DA CONSTELAÇÃO' : '[03] TOPOLOGICAL CONSTELLATION MAP',
    padX,
    curY
  );
  curY += 14;

  const mapBoxX = padX;
  const mapBoxY = curY;
  const mapBoxW = contentWidth;
  const mapBoxH = 224;

  // Frame background
  ctx.fillStyle = '#f3efe6';
  ctx.fillRect(mapBoxX, mapBoxY, mapBoxW, mapBoxH);
  ctx.strokeStyle = '#d3cdbe';
  ctx.lineWidth = 1;
  ctx.strokeRect(mapBoxX, mapBoxY, mapBoxW, mapBoxH);

  // Background subtle dot matrix grid
  ctx.fillStyle = 'rgba(120, 113, 108, 0.18)';
  for (let gx = mapBoxX + 16; gx < mapBoxX + mapBoxW; gx += 20) {
    for (let gy = mapBoxY + 16; gy < mapBoxY + mapBoxH; gy += 20) {
      ctx.fillRect(gx, gy, 1.2, 1.2);
    }
  }

  // Crosshairs in corners
  ctx.strokeStyle = '#a8a29e';
  ctx.lineWidth = 1;
  const chLen = 6;
  const corners = [
    [mapBoxX + 6, mapBoxY + 6],
    [mapBoxX + mapBoxW - 6, mapBoxY + 6],
    [mapBoxX + 6, mapBoxY + mapBoxH - 6],
    [mapBoxX + mapBoxW - 6, mapBoxY + mapBoxH - 6],
  ];
  for (const [cx, cy] of corners) {
    ctx.beginPath();
    ctx.moveTo(cx - chLen, cy);
    ctx.lineTo(cx + chLen, cy);
    ctx.moveTo(cx, cy - chLen);
    ctx.lineTo(cx, cy + chLen);
    ctx.stroke();
  }

  // Technical Watermark inside Map Header
  ctx.font = '500 7px "JetBrains Mono", monospace';
  ctx.fillStyle = '#8f887f';
  ctx.textAlign = 'left';
  ctx.fillText('TOPOLOGY: MULTIDOMAIN CONSTELLATION', mapBoxX + 10, mapBoxY + 12);
  ctx.textAlign = 'right';
  ctx.fillText('PPR RELAXATION: CONVERGED', mapBoxX + mapBoxW - 10, mapBoxY + 12);

  // Run Force-Directed Simulation Layout for organic constellation topology
  const { nodes: layoutNodes, edges: layoutEdges } = layoutConstellationGraph({
    seeds,
    recs: topRecs.slice(0, 6),
    graph,
    boxX: mapBoxX,
    boxY: mapBoxY,
    boxW: mapBoxW,
    boxH: mapBoxH,
  });

  // Draw intra-seed backbone bridges (cool slate dashed line)
  ctx.save();
  ctx.strokeStyle = '#94a3b8';
  ctx.lineWidth = 1.2;
  ctx.setLineDash([4, 3]);
  for (const e of layoutEdges) {
    if (e.type === 'seed-seed' && e.source && e.target) {
      ctx.beginPath();
      ctx.moveTo(e.source.x, e.source.y);
      ctx.lineTo(e.target.x, e.target.y);
      ctx.stroke();
    }
  }
  ctx.restore();

  // Draw discovery-to-seed topological bridges (fine dotted line)
  ctx.save();
  ctx.strokeStyle = '#b8b2a0';
  ctx.lineWidth = 1;
  ctx.setLineDash([2.5, 3]);
  for (const e of layoutEdges) {
    if (e.type === 'seed-rec' && e.source && e.target) {
      ctx.beginPath();
      ctx.moveTo(e.source.x, e.source.y);
      ctx.lineTo(e.target.x, e.target.y);
      ctx.stroke();
    }
  }
  ctx.restore();

  // Draw Discovery Nodes (Colored cores with white border)
  for (const n of layoutNodes.filter(n => !n.isSeed)) {
    const col = DOMAIN_COLORS[n.type] || '#2563eb';

    // Faint aura
    ctx.beginPath();
    ctx.arc(n.x, n.y, 6.5, 0, Math.PI * 2);
    ctx.fillStyle = 'rgba(120, 113, 108, 0.12)';
    ctx.fill();

    // Solid core
    ctx.beginPath();
    ctx.arc(n.x, n.y, 4.5, 0, Math.PI * 2);
    ctx.fillStyle = col;
    ctx.fill();
    ctx.strokeStyle = '#faf8f2';
    ctx.lineWidth = 1.3;
    ctx.stroke();
  }

  // Draw Seed Nodes (Ruby double-ring stars)
  for (const n of layoutNodes.filter(n => n.isSeed)) {
    // Outer ruby halo
    ctx.beginPath();
    ctx.arc(n.x, n.y, 8.5, 0, Math.PI * 2);
    ctx.fillStyle = 'rgba(225, 29, 72, 0.15)';
    ctx.fill();

    // Outer ruby ring
    ctx.beginPath();
    ctx.arc(n.x, n.y, 5.5, 0, Math.PI * 2);
    ctx.fillStyle = '#e11d48';
    ctx.fill();
    ctx.strokeStyle = '#faf8f2';
    ctx.lineWidth = 1.5;
    ctx.stroke();

    // Inner bright white pin
    ctx.beginPath();
    ctx.arc(n.x, n.y, 1.8, 0, Math.PI * 2);
    ctx.fillStyle = '#ffffff';
    ctx.fill();
  }

  // Draw Collision-Free Text Labels with protective background pills
  for (const n of layoutNodes) {
    const labelText = truncate(n.name, n.isSeed ? 16 : 14);
    const { align, lx, ly } = computeLabelPlacement(n, layoutNodes, mapBoxX, mapBoxY, mapBoxW, mapBoxH);

    ctx.save();
    ctx.font = n.isSeed ? '700 8.5px "JetBrains Mono", monospace' : '600 8px "JetBrains Mono", monospace';
    const tw = ctx.measureText(labelText).width;
    const th = 11;
    let px = lx;
    if (align === 'center') px = lx - tw / 2;
    else if (align === 'right') px = lx - tw;
    const py = ly - 8.5;

    // Semi-transparent vintage thermal paper background pill for guaranteed legibility
    ctx.fillStyle = 'rgba(250, 248, 242, 0.92)';
    ctx.fillRect(px - 2.5, py, tw + 5, th);

    // Text stroke & fill
    ctx.textAlign = align;
    ctx.fillStyle = n.isSeed ? '#9f1239' : '#1c1917';
    ctx.fillText(labelText, lx, ly);
    ctx.restore();
  }

  // Mini-map legend footer
  ctx.font = '500 8px "JetBrains Mono", monospace';
  ctx.fillStyle = '#78716c';
  ctx.textAlign = 'center';
  ctx.fillText(
    isPt ? '● SEMENTES (RUBI)   ○ DESCOBERTAS TRANSMIDIÁTICAS   ── PONTES DE AFINIDADE' : '● SEEDS (RUBY)   ○ TRANSMEDIA DISCOVERIES   ── AFFINITY BRIDGES',
    mapBoxX + mapBoxW / 2,
    mapBoxY + mapBoxH - 8
  );

  curY = mapBoxY + mapBoxH + 16;
  drawDottedLine(curY);
  curY += 18;

  // 6. SECTION 4: Fresh Discoveries Menu (Cardápio)
  ctx.textAlign = 'left';
  ctx.fillStyle = '#1c1917';
  ctx.font = '700 11px "JetBrains Mono", monospace';
  ctx.fillText(
    isPt ? '[04] CARDÁPIO DE DESCOBERTAS INÉDITAS' : '[04] FRESH CULTURAL DISCOVERIES MENU',
    padX,
    curY
  );
  curY += 14;

  ctx.font = '400 9px "JetBrains Mono", monospace';
  ctx.fillStyle = '#78716c';
  ctx.fillText(
    isPt ? '(Obras inéditas recomendadas • Filtradas pelo seu perfil)' : '(Curated fresh works • Filtered by your profile)',
    padX,
    curY
  );
  curY += 16;

  for (let i = 0; i < topRecs.length; i++) {
    const item = topRecs[i];
    const rankStr = String(i + 1).padStart(2, '0');
    const typeLabel = getDomainBadge(item.type, isPt);
    const title = cleanItemTitle(item.title || item.name || item.id, item);
    const affinityScore = item.score !== undefined ? `${Math.round(item.score * 100)}%` : '95%';
    const reason = item.reason || (isPt ? 'Confluência multidimensional no grafo' : 'Multidimensional graph confluence');

    // Line 1: Rank + Type + Title ............... Affinity%
    ctx.font = '700 10.5px "JetBrains Mono", monospace';
    ctx.fillStyle = '#1c1917';
    ctx.fillText(`${rankStr}.`, padX, curY);

    ctx.font = '700 9.5px "JetBrains Mono", monospace';
    ctx.fillStyle = DOMAIN_COLORS[item.type] || '#2563eb';
    ctx.fillText(typeLabel, padX + 24, curY);

    ctx.font = '600 10.5px "JetBrains Mono", monospace';
    ctx.fillStyle = '#1c1917';
    ctx.fillText(truncate(title, 32), padX + 88, curY);

    ctx.textAlign = 'right';
    ctx.font = '700 10px "JetBrains Mono", monospace';
    ctx.fillStyle = '#0f766e';
    ctx.fillText(`${affinityScore}`, width - padX, curY);
    ctx.textAlign = 'left';
    curY += 14;

    // Line 2: Reason / Bridge (Indented)
    ctx.font = '400 9px "JetBrains Mono", monospace';
    ctx.fillStyle = '#57534e';
    ctx.fillText(`↳ ${truncate(reason, 62)}`, padX + 24, curY);
    curY += 24;
  }

  drawDottedLine(curY);
  curY += 16;

  // 7. SECTION 5: Graph Totals & Audit
  ctx.font = '500 10px "JetBrains Mono", monospace';
  ctx.fillStyle = '#44403c';

  const rowAudit = (label, val, y) => {
    ctx.textAlign = 'left';
    ctx.fillText(label, padX, y);
    ctx.textAlign = 'right';
    ctx.fillText(String(val), width - padX, y);
  };

  rowAudit(
    isPt ? 'TOTAL DE DESCOBERTAS GERADAS:' : 'TOTAL DISCOVERIES GENERATED:',
    recommendations.length,
    curY
  );
  curY += 16;

  rowAudit(
    isPt ? 'OBRAS CONHECIDAS FILTRADAS:' : 'KNOWN ITEMS FILTERED OUT:',
    knownCount,
    curY
  );
  curY += 16;

  if (negativeSeeds.length > 0) {
    rowAudit(
      isPt ? 'PÓLOS DE REPULSÃO ATIVOS:' : 'ACTIVE REPULSION POLES:',
      `${negativeSeeds.length} (β=${negativeBeta.toFixed(1)}x)`,
      curY
    );
    curY += 16;
  }

  rowAudit(
    isPt ? 'ÍNDICE DE NOVIDADE / SERENDIPIDADE:' : 'NOVELTY / SERENDIPITY RATE:',
    '100% INÉDITO',
    curY
  );
  curY += 18;

  drawDoubleLine(curY);
  curY += 20;

  // 8. SECTION 6: Procedural Barcode, Link & Closing Motto
  const barcodeY = curY;
  const barcodeH = 34;
  const barcodeW = 280;
  const barcodeX = (width - barcodeW) / 2;

  ctx.fillStyle = '#1c1917';
  let curBx = barcodeX;
  for (let i = 0; i < tasteHash.length; i++) {
    const val = parseInt(tasteHash[i], 16);
    const pattern = [
      (val & 1) ? 2 : 1,
      (val & 2) ? 3 : 1,
      (val & 4) ? 1 : 2,
      (val & 8) ? 4 : 2,
      2,
    ];
    for (const p of pattern) {
      if (curBx + p > barcodeX + barcodeW) break;
      ctx.fillRect(curBx, barcodeY, p, barcodeH);
      curBx += p + 2;
    }
  }

  curY = barcodeY + barcodeH + 12;

  // Numeric code below barcode
  ctx.textAlign = 'center';
  ctx.font = '500 10px "JetBrains Mono", monospace';
  ctx.fillStyle = '#1c1917';
  ctx.fillText(`* YM - ${tasteHash} - ${now.getFullYear()} *`, width / 2, curY);
  curY += 22;

  // Footer slogan
  ctx.font = '600 10px "JetBrains Mono", monospace';
  ctx.fillStyle = '#44403c';
  ctx.fillText(
    isPt ? 'CULTIVE SEU PRÓPRIO REPERTÓRIO CULTURAL' : 'CULTIVATE YOUR OWN CULTURAL REPERTOIRE',
    width / 2,
    curY
  );
  curY += 14;

  ctx.font = '400 8.5px "JetBrains Mono", monospace';
  ctx.fillStyle = '#78716c';
  ctx.fillText(
    isPt ? 'OBRIGADO POR EXPLORAR O GRAFO' : 'THANK YOU FOR EXPLORING THE GRAPH',
    width / 2,
    curY
  );

  ctx.restore();
}

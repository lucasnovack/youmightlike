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

const APPLE_FONTS = {
  sans: '-apple-system, BlinkMacSystemFont, "SF Pro Text", "SF Pro Display", "Inter", system-ui, sans-serif',
  mono: '"SF Mono", "JetBrains Mono", ui-monospace, Menlo, monospace',
};

const APPLE_BADGES = {
  game: { labelPt: 'JOGO', labelEn: 'GAME', bg: '#ede9fe', text: '#6d28d9', border: 'rgba(109, 40, 217, 0.2)' },
  artist: { labelPt: 'ARTISTA', labelEn: 'ARTIST', bg: '#ffedd5', text: '#c2410c', border: 'rgba(194, 65, 12, 0.2)' },
  track: { labelPt: 'MÚSICA', labelEn: 'TRACK', bg: '#e0f2fe', text: '#0369a1', border: 'rgba(3, 105, 161, 0.2)' },
  movie: { labelPt: 'FILME', labelEn: 'CINEMA', bg: '#fef3c7', text: '#b45309', border: 'rgba(180, 83, 9, 0.2)' },
  series: { labelPt: 'SÉRIE', labelEn: 'SERIES', bg: '#fae8ff', text: '#7e22ce', border: 'rgba(126, 34, 206, 0.2)' },
  book: { labelPt: 'LIVRO', labelEn: 'BOOK', bg: '#d1fae5', text: '#047857', border: 'rgba(4, 120, 87, 0.2)' },
  default: { labelPt: 'OBRA', labelEn: 'WORK', bg: '#f2f2f7', text: '#475569', border: 'rgba(71, 85, 105, 0.2)' },
};

function getBadgeInfo(type) {
  const t = (type || '').toLowerCase();
  return APPLE_BADGES[t] || APPLE_BADGES.default;
}

function drawRoundedRect(ctx, x, y, w, h, r, fill = null, stroke = null, strokeWidth = 1) {
  ctx.beginPath();
  if (ctx.roundRect) {
    ctx.roundRect(x, y, w, h, r);
  } else {
    const rad = Math.min(r, w / 2, h / 2);
    ctx.moveTo(x + rad, y);
    ctx.lineTo(x + w - rad, y);
    ctx.arcTo(x + w, y, x + w, y + rad, rad);
    ctx.lineTo(x + w, y + h - rad);
    ctx.arcTo(x + w, y + h, x + w - rad, y + h, rad);
    ctx.lineTo(x + rad, y + h);
    ctx.arcTo(x, y + h, x, y + h - rad, rad);
    ctx.lineTo(x, y + rad);
    ctx.arcTo(x, y, x + rad, y, rad);
    ctx.closePath();
  }
  if (fill) {
    ctx.fillStyle = fill;
    ctx.fill();
  }
  if (stroke) {
    ctx.strokeStyle = stroke;
    ctx.lineWidth = strokeWidth;
    ctx.stroke();
  }
}

function drawPillBadge(ctx, text, x, y, badgeConf) {
  ctx.save();
  ctx.font = `600 8.5px ${APPLE_FONTS.sans}`;
  const padX = 7;
  const metrics = ctx.measureText(text);
  const w = metrics.width + padX * 2;
  const h = 17;
  const rx = x;
  const ry = y - 12;

  drawRoundedRect(ctx, rx, ry, w, h, 5, badgeConf.bg, badgeConf.border, 0.8);

  ctx.fillStyle = badgeConf.text;
  ctx.textAlign = 'left';
  ctx.textBaseline = 'middle';
  ctx.fillText(text, rx + padX, ry + h / 2);
  ctx.restore();
  return w;
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
          if (t && t.length > 1 && tagsSet.size < 7) {
            tagsSet.add(t.toLowerCase().replace(/\s+/g, '-'));
          }
        }
      }
    }
  }
  if (tagsSet.size < 4 && graph && graph.nodes) {
    for (const n of graph.nodes) {
      if ((n.type === 'tag' || n.type === 'keyword') && tagsSet.size < 7) {
        tagsSet.add((n.label || '').toLowerCase().replace(/\s+/g, '-'));
      }
    }
  }
  const resonanceTags = Array.from(tagsSet);

  // Take balanced transmedia recommendations (up to 7)
  const topRecs = selectBalancedTicketRecs(recommendations, seeds, 7);

  // Compute dynamic height
  let estimatedHeight = 0;
  estimatedHeight += 36; // Top padding
  estimatedHeight += 90; // Header & order meta
  estimatedHeight += 24; // Divider
  estimatedHeight += 32 + (seeds.length * 36); // Seeds section
  if (negativeSeeds.length > 0) {
    estimatedHeight += 28 + (negativeSeeds.length * 32); // Negative seeds
  }
  estimatedHeight += 22; // Divider
  estimatedHeight += 58; // DNA Tags
  estimatedHeight += 24; // Divider
  estimatedHeight += 270; // Mini-map frame with generous breathing space
  estimatedHeight += 24; // Divider
  estimatedHeight += 38 + (topRecs.length * 48); // Fresh recommendations
  estimatedHeight += 24; // Divider
  estimatedHeight += 95; // Totals metric cards
  estimatedHeight += 24; // Divider
  estimatedHeight += 110; // Barcode, link & footer
  estimatedHeight += 36; // Bottom margin

  const height = Math.round(estimatedHeight);

  canvas.width = Math.round(width * dpr);
  canvas.height = Math.round(height * dpr);
  canvas.style.width = `${width}px`;
  canvas.style.height = `${height}px`;

  ctx.save();
  ctx.scale(dpr, dpr);

  // 1. Draw Apple Card Canvas Background (Porcelain White with Soft Outer Border)
  drawRoundedRect(ctx, 1.5, 1.5, width - 3, height - 3, 24, '#ffffff', '#e5e5ea', 1.5);

  // Layout Cursor
  let curY = 36;
  const padX = 40;
  const contentWidth = width - padX * 2;

  const drawDivider = (y) => {
    ctx.save();
    ctx.strokeStyle = '#f2f2f7';
    ctx.lineWidth = 1;
    ctx.beginPath();
    ctx.moveTo(padX, y);
    ctx.lineTo(width - padX, y);
    ctx.stroke();
    ctx.restore();
  };

  const tasteHash = computeTasteHash(seeds);
  const now = new Date();
  const dateStr = now.toLocaleDateString(isPt ? 'pt-BR' : 'en-US', {
    day: '2-digit',
    month: 'short',
    year: 'numeric',
  });
  const timeStr = now.toLocaleTimeString(isPt ? 'pt-BR' : 'en-US', {
    hour: '2-digit',
    minute: '2-digit',
  });

  // 2. HEADER: Brand & Order Metadata (Apple Passbook Style)
  // Monogram circle
  ctx.beginPath();
  ctx.arc(padX + 18, curY + 18, 18, 0, Math.PI * 2);
  ctx.fillStyle = '#1d1d1f';
  ctx.fill();

  ctx.fillStyle = '#ffffff';
  ctx.font = `700 13px ${APPLE_FONTS.sans}`;
  ctx.textAlign = 'center';
  ctx.textBaseline = 'middle';
  ctx.fillText('Y', padX + 18, curY + 18);

  ctx.textAlign = 'left';
  ctx.textBaseline = 'alphabetic';
  ctx.font = `700 16px ${APPLE_FONTS.sans}`;
  ctx.fillStyle = '#1d1d1f';
  ctx.fillText('youmightlike', padX + 46, curY + 16);

  ctx.font = `500 11px ${APPLE_FONTS.sans}`;
  ctx.fillStyle = '#86868b';
  ctx.fillText(
    isPt ? 'Passaporte de Afinidade Cultural' : 'Cultural Affinity Passport',
    padX + 46,
    curY + 31
  );

  // Top-right pill for Order / Date
  const metaCode = `#YM-${tasteHash}`;
  ctx.font = `600 10px ${APPLE_FONTS.mono}`;
  const metaW = ctx.measureText(metaCode).width + 20;
  drawRoundedRect(ctx, width - padX - metaW, curY + 4, metaW, 24, 12, '#f2f2f7', '#e5e5ea', 1);

  ctx.fillStyle = '#1d1d1f';
  ctx.textAlign = 'center';
  ctx.fillText(metaCode, width - padX - metaW / 2, curY + 20);

  curY += 48;

  // Subheader line: Date & Engine Specs
  ctx.textAlign = 'left';
  ctx.font = `500 10px ${APPLE_FONTS.sans}`;
  ctx.fillStyle = '#86868b';
  ctx.fillText(`${dateStr} • ${timeStr}`, padX, curY);

  ctx.textAlign = 'right';
  ctx.fillText('Engine: Stochastic PPR • Transmedia Graph', width - padX, curY);
  curY += 16;

  drawDivider(curY);
  curY += 24;

  // 3. SECTION 1: Base Ingredients (Profile Seeds)
  ctx.textAlign = 'left';
  ctx.fillStyle = '#1d1d1f';
  ctx.font = `700 12px ${APPLE_FONTS.sans}`;
  ctx.fillText(
    isPt ? 'Sementes do Perfil (Atração)' : 'Profile Seeds (Attraction Poles)',
    padX,
    curY
  );
  curY += 20;

  for (let i = 0; i < seeds.length; i++) {
    const s = seeds[i];
    const bInfo = getBadgeInfo(s.type);
    const bLabel = isPt ? bInfo.labelPt : bInfo.labelEn;
    const name = s.name || s.id;
    const sub = translateSubtitle(s.subtitle, isPt);

    const badgeW = drawPillBadge(ctx, bLabel, padX, curY, bInfo);

    ctx.font = `600 12px ${APPLE_FONTS.sans}`;
    ctx.fillStyle = '#1d1d1f';
    ctx.fillText(truncate(name, 34), padX + badgeW + 10, curY);

    if (sub) {
      ctx.textAlign = 'right';
      ctx.font = `400 11px ${APPLE_FONTS.sans}`;
      ctx.fillStyle = '#86868b';
      ctx.fillText(truncate(sub, 22), width - padX, curY);
      ctx.textAlign = 'left';
    }
    curY += 28;
  }

  // Negative seeds (if any)
  if (negativeSeeds && negativeSeeds.length > 0) {
    curY += 8;
    ctx.font = `700 11px ${APPLE_FONTS.sans}`;
    ctx.fillStyle = '#e11d48';
    ctx.fillText(
      isPt ? `Pólos de Repulsão (Evitados • β: ${negativeBeta.toFixed(1)}x)` : `Repulsive Poles (Avoided • β: ${negativeBeta.toFixed(1)}x)`,
      padX,
      curY
    );
    curY += 18;

    for (const neg of negativeSeeds) {
      const negBadge = { bg: '#fee2e2', text: '#991b1b', border: 'rgba(239, 68, 68, 0.25)' };
      const bW = drawPillBadge(ctx, isPt ? 'EVITAR' : 'AVOID', padX, curY, negBadge);

      ctx.font = `500 11.5px ${APPLE_FONTS.sans}`;
      ctx.fillStyle = '#475569';
      ctx.fillText(truncate(neg.name || neg.id, 40), padX + bW + 10, curY);
      curY += 24;
    }
  }

  curY += 6;
  drawDivider(curY);
  curY += 22;

  // 4. SECTION 2: Aesthetic DNA & Resonance Tags (Pill Capsules)
  ctx.textAlign = 'left';
  ctx.fillStyle = '#1d1d1f';
  ctx.font = `700 12px ${APPLE_FONTS.sans}`;
  ctx.fillText(
    isPt ? 'DNA Estético & Confluências' : 'Aesthetic DNA & Confluences',
    padX,
    curY
  );
  curY += 18;

  if (resonanceTags.length > 0) {
    let tagCursorX = padX;
    ctx.font = `500 10px ${APPLE_FONTS.sans}`;
    for (const tag of resonanceTags) {
      const tagText = `#${tag}`;
      const tMetrics = ctx.measureText(tagText);
      const tagW = tMetrics.width + 16;
      if (tagCursorX + tagW > width - padX) break;

      drawRoundedRect(ctx, tagCursorX, curY - 12, tagW, 20, 10, '#f2f2f7', '#e5e5ea', 0.8);
      ctx.fillStyle = '#475569';
      ctx.fillText(tagText, tagCursorX + 8, curY + 2);
      tagCursorX += tagW + 6;
    }
  } else {
    ctx.font = `400 11px ${APPLE_FONTS.sans}`;
    ctx.fillStyle = '#86868b';
    ctx.fillText(isPt ? 'Confluência transmidiática multidimensional' : 'Multidimensional transmedia confluence', padX, curY);
  }
  curY += 24;

  drawDivider(curY);
  curY += 22;

  // 5. SECTION 3: Constellation Topological Map (Apple Canvas View)
  ctx.textAlign = 'left';
  ctx.fillStyle = '#1d1d1f';
  ctx.font = `700 12px ${APPLE_FONTS.sans}`;
  ctx.fillText(
    isPt ? 'Mapa Topológico da Constelação' : 'Topological Constellation Map',
    padX,
    curY
  );
  curY += 14;

  const mapBoxX = padX;
  const mapBoxY = curY;
  const mapBoxW = contentWidth;
  const mapBoxH = 210;

  // Rounded container for graph
  drawRoundedRect(ctx, mapBoxX, mapBoxY, mapBoxW, mapBoxH, 16, '#fbfbfd', '#e5e5ea', 1);

  // Background subtle dot matrix grid
  ctx.fillStyle = 'rgba(0, 0, 0, 0.04)';
  for (let gx = mapBoxX + 16; gx < mapBoxX + mapBoxW; gx += 20) {
    for (let gy = mapBoxY + 16; gy < mapBoxY + mapBoxH; gy += 20) {
      ctx.beginPath();
      ctx.arc(gx, gy, 1.0, 0, Math.PI * 2);
      ctx.fill();
    }
  }

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

  // Draw intra-seed backbone bridges (smooth blue line)
  ctx.save();
  ctx.strokeStyle = 'rgba(0, 113, 227, 0.35)';
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

  // Draw discovery-to-seed topological bridges (fine subtle line)
  ctx.save();
  ctx.strokeStyle = 'rgba(148, 163, 184, 0.40)';
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

  // Draw Discovery Nodes (Colored cores with white ring)
  for (const n of layoutNodes.filter(n => !n.isSeed)) {
    const col = DOMAIN_COLORS[n.type] || '#0071e3';

    // Faint halo
    ctx.beginPath();
    ctx.arc(n.x, n.y, 6.5, 0, Math.PI * 2);
    ctx.fillStyle = 'rgba(0, 0, 0, 0.04)';
    ctx.fill();

    // Solid core
    ctx.beginPath();
    ctx.arc(n.x, n.y, 4.5, 0, Math.PI * 2);
    ctx.fillStyle = col;
    ctx.fill();
    ctx.strokeStyle = '#ffffff';
    ctx.lineWidth = 1.5;
    ctx.stroke();
  }

  // Draw Seed Nodes (Apple Blue Double-Ring Stars)
  for (const n of layoutNodes.filter(n => n.isSeed)) {
    // Outer blue halo
    ctx.beginPath();
    ctx.arc(n.x, n.y, 9, 0, Math.PI * 2);
    ctx.fillStyle = 'rgba(0, 113, 227, 0.15)';
    ctx.fill();

    // Outer ring
    ctx.beginPath();
    ctx.arc(n.x, n.y, 5.5, 0, Math.PI * 2);
    ctx.fillStyle = '#0071e3';
    ctx.fill();
    ctx.strokeStyle = '#ffffff';
    ctx.lineWidth = 1.5;
    ctx.stroke();

    // Inner bright white pin
    ctx.beginPath();
    ctx.arc(n.x, n.y, 2.0, 0, Math.PI * 2);
    ctx.fillStyle = '#ffffff';
    ctx.fill();
  }

  // Draw Collision-Free Text Labels with protective white background pills
  for (const n of layoutNodes) {
    const labelText = truncate(n.name, n.isSeed ? 16 : 14);
    const { align, lx, ly } = computeLabelPlacement(n, layoutNodes, mapBoxX, mapBoxY, mapBoxW, mapBoxH);

    ctx.save();
    ctx.font = n.isSeed ? `600 9px ${APPLE_FONTS.sans}` : `500 8.5px ${APPLE_FONTS.sans}`;
    const tw = ctx.measureText(labelText).width;
    const th = 13;
    let px = lx;
    if (align === 'center') px = lx - tw / 2;
    else if (align === 'right') px = lx - tw;
    const py = ly - 9.5;

    // Semi-transparent pill background for guaranteed legibility
    drawRoundedRect(ctx, px - 3, py, tw + 6, th, 4, 'rgba(255, 255, 255, 0.95)', '#e5e5ea', 0.6);

    // Text stroke & fill
    ctx.textAlign = align;
    ctx.fillStyle = n.isSeed ? '#0071e3' : '#1d1d1f';
    ctx.fillText(labelText, lx, ly);
    ctx.restore();
  }

  // Mini-map legend footer
  ctx.font = `500 9px ${APPLE_FONTS.sans}`;
  ctx.fillStyle = '#86868b';
  ctx.textAlign = 'center';
  ctx.fillText(
    isPt ? '● Sementes   ○ Descobertas Transmidiáticas   ── Pontes de Confluência' : '● Seeds   ○ Transmedia Discoveries   ── Confluence Bridges',
    mapBoxX + mapBoxW / 2,
    mapBoxY + mapBoxH - 9
  );

  curY = mapBoxY + mapBoxH + 18;
  drawDivider(curY);
  curY += 22;

  // 6. SECTION 4: Fresh Discoveries Menu (Apple Table Row Style)
  ctx.textAlign = 'left';
  ctx.fillStyle = '#1d1d1f';
  ctx.font = `700 12px ${APPLE_FONTS.sans}`;
  ctx.fillText(
    isPt ? 'Descobertas Recomendadas' : 'Curated Fresh Discoveries',
    padX,
    curY
  );
  curY += 16;

  ctx.font = `400 10.5px ${APPLE_FONTS.sans}`;
  ctx.fillStyle = '#86868b';
  ctx.fillText(
    isPt ? 'Obras inéditas selecionadas pelo grafo para expandir seu repertório' : 'Fresh works selected by the graph to expand your repertoire',
    padX,
    curY
  );
  curY += 18;

  const maxRecScore = topRecs.reduce((m, r) => Math.max(m, Number(r.score) || 0), 1);

  for (let i = 0; i < topRecs.length; i++) {
    const item = topRecs[i];
    const rankStr = String(i + 1).padStart(2, '0');
    const bInfo = getBadgeInfo(item.type);
    const bLabel = isPt ? bInfo.labelPt : bInfo.labelEn;
    const title = cleanItemTitle(item.title || item.name || item.id, item);

    // Intuitive log-normalized match percentage (68% to 99% Match) instead of raw 25000%
    const rawS = Number(item.score) || 0;
    const ratio = maxRecScore > 0 ? Math.max(0, Math.min(1, Math.log10(1 + rawS) / Math.log10(1 + maxRecScore))) : 0.8;
    const matchPct = Math.round(68 + ratio * 31);
    const affinityLabel = `${matchPct}% Match`;
    const reason = item.reason || (isPt ? 'Confluência multidimensional no grafo' : 'Multidimensional graph confluence');

    // Line 1: Rank + Type Pill + Title ............... Affinity Match%
    ctx.font = `500 11px ${APPLE_FONTS.mono}`;
    ctx.fillStyle = '#86868b';
    ctx.fillText(rankStr, padX, curY);

    const bW = drawPillBadge(ctx, bLabel, padX + 22, curY, bInfo);

    // Affinity pill in emerald green
    ctx.font = `600 9px ${APPLE_FONTS.sans}`;
    const affMetrics = ctx.measureText(affinityLabel);
    const affW = affMetrics.width + 14;
    drawRoundedRect(ctx, width - padX - affW, curY - 11, affW, 17, 8.5, '#ecfdf5', 'rgba(16, 185, 129, 0.25)', 0.8);
    ctx.fillStyle = '#047857';
    ctx.textAlign = 'center';
    ctx.fillText(affinityLabel, width - padX - affW / 2, curY + 1);
    ctx.textAlign = 'left';

    // Dynamic title truncation so it never touches the affinity badge
    const availTitleW = width - padX * 2 - bW - affW - 36;
    ctx.font = `600 12px ${APPLE_FONTS.sans}`;
    let displayTitle = title;
    while (ctx.measureText(displayTitle).width > availTitleW && displayTitle.length > 5) {
      displayTitle = displayTitle.slice(0, -2) + '…';
    }
    ctx.fillStyle = '#1d1d1f';
    ctx.fillText(displayTitle, padX + 26 + bW, curY);

    curY += 16;

    // Line 2: Reason / Bridge (Indented)
    ctx.font = `400 10px ${APPLE_FONTS.sans}`;
    ctx.fillStyle = '#86868b';
    ctx.fillText(`↳ ${truncate(reason, 64)}`, padX + 24, curY);
    curY += 26;
  }

  drawDivider(curY);
  curY += 20;

  // 7. SECTION 5: Graph Totals & Audit (3 Metrics Cards)
  const cardGap = 12;
  const cardW = (contentWidth - cardGap * 2) / 3;
  const cardH = 58;

  const drawMetricCard = (x, y, label, value, sub = '') => {
    drawRoundedRect(ctx, x, y, cardW, cardH, 12, '#fbfbfd', '#e5e5ea', 1);
    ctx.font = `500 9.5px ${APPLE_FONTS.sans}`;
    ctx.fillStyle = '#86868b';
    ctx.textAlign = 'center';
    ctx.fillText(label, x + cardW / 2, y + 18);

    ctx.font = `700 15px ${APPLE_FONTS.sans}`;
    ctx.fillStyle = '#1d1d1f';
    ctx.fillText(String(value), x + cardW / 2, y + 38);

    if (sub) {
      ctx.font = `400 8.5px ${APPLE_FONTS.sans}`;
      ctx.fillStyle = '#86868b';
      ctx.fillText(sub, x + cardW / 2, y + 50);
    }
  };

  drawMetricCard(padX, curY, isPt ? 'Descobertas' : 'Discoveries', recommendations.length);
  drawMetricCard(padX + cardW + cardGap, curY, isPt ? 'Conhecidas' : 'Known Items', knownCount);
  drawMetricCard(padX + (cardW + cardGap) * 2, curY, isPt ? 'Serendipidade' : 'Serendipity', '100%', isPt ? 'Inédito' : 'Fresh');

  curY += cardH + 22;
  drawDivider(curY);
  curY += 24;

  // 8. SECTION 6: Procedural Barcode, Link & Closing Motto
  const barcodeY = curY;
  const barcodeH = 30;
  const barcodeW = 240;
  const barcodeX = (width - barcodeW) / 2;

  ctx.fillStyle = '#1d1d1f';
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
  ctx.font = `500 10px ${APPLE_FONTS.mono}`;
  ctx.fillStyle = '#86868b';
  ctx.fillText(`YM - ${tasteHash} - ${now.getFullYear()}`, width / 2, curY);
  curY += 20;

  // Footer slogan
  ctx.font = `600 10px ${APPLE_FONTS.sans}`;
  ctx.fillStyle = '#1d1d1f';
  ctx.fillText(
    isPt ? 'CULTIVE SEU PRÓPRIO REPERTÓRIO CULTURAL' : 'CULTIVATE YOUR OWN CULTURAL REPERTOIRE',
    width / 2,
    curY
  );
  curY += 14;

  ctx.font = `400 9px ${APPLE_FONTS.sans}`;
  ctx.fillStyle = '#86868b';
  ctx.fillText(
    'youmightlike • transmedia discovery engine',
    width / 2,
    curY
  );

  ctx.restore();
}

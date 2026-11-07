// Cultural Taste Receipt Generator for youmightlike
// Generates an ultra-sharp, high-DPI thermal receipt / aesthetic poster on HTML5 Canvas.

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

  // Take top fresh recommendations (up to 7)
  const topRecs = recommendations.slice(0, 7);

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
  estimatedHeight += 250; // Mini-map frame
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
    isPt ? 'RECEITA DE GOSTO CULTURAL // GRAFO TRANSMIDIÁTICO' : 'CULTURAL TASTE RECEIPT // TRANSMEDIA GRAPH',
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
    const typeLabel = `[${(s.type || 'SEED').toUpperCase()}]`;
    const name = s.name || s.id;
    const sub = s.subtitle ? `(${s.subtitle})` : '';

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

  // 5. SECTION 3: Constellation Topological Mini-Map (Vector Graph)
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
  const mapBoxH = 195;

  // Frame background
  ctx.fillStyle = '#f3efe6';
  ctx.fillRect(mapBoxX, mapBoxY, mapBoxW, mapBoxH);
  ctx.strokeStyle = '#d3cdbe';
  ctx.lineWidth = 1;
  ctx.strokeRect(mapBoxX, mapBoxY, mapBoxW, mapBoxH);

  // Background subtle dot matrix grid
  ctx.fillStyle = 'rgba(120, 113, 108, 0.2)';
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

  // Layout nodes inside the mini-map box
  const mcx = mapBoxX + mapBoxW / 2;
  const mcy = mapBoxY + mapBoxH / 2 - 4;

  // Position seeds
  const seedNodes = [];
  const numSeeds = seeds.length;
  if (numSeeds === 1) {
    seedNodes.push({ x: mcx, y: mcy, name: seeds[0].name || seeds[0].id, type: seeds[0].type });
  } else {
    const sRadX = Math.min(65, 25 + numSeeds * 10);
    const sRadY = Math.min(45, 18 + numSeeds * 7);
    for (let i = 0; i < numSeeds; i++) {
      const angle = (i / numSeeds) * Math.PI * 2 - Math.PI / 2;
      seedNodes.push({
        x: mcx + Math.cos(angle) * sRadX,
        y: mcy + Math.sin(angle) * sRadY,
        name: seeds[i].name || seeds[i].id,
        type: seeds[i].type,
      });
    }
  }

  // Position recommendations on outer ellipse
  const recNodes = [];
  const numRecs = Math.min(topRecs.length, 6);
  const rRadX = 210;
  const rRadY = 68;

  for (let i = 0; i < numRecs; i++) {
    const angle = (i / numRecs) * Math.PI * 2 - Math.PI / 2 + 0.35;
    const rx = mcx + Math.cos(angle) * rRadX;
    const ry = mcy + Math.sin(angle) * rRadY;
    recNodes.push({
      x: rx,
      y: ry,
      name: topRecs[i].title || topRecs[i].name || topRecs[i].id,
      type: topRecs[i].type,
      score: topRecs[i].score,
    });
  }

  // Draw connecting bridges between closest seed and recs
  ctx.save();
  ctx.setLineDash([2, 3]);
  ctx.strokeStyle = '#b8b2a0';
  ctx.lineWidth = 1;
  for (const rn of recNodes) {
    let closest = seedNodes[0];
    let minDist = Infinity;
    for (const sn of seedNodes) {
      const d = Math.hypot(rn.x - sn.x, rn.y - sn.y);
      if (d < minDist) {
        minDist = d;
        closest = sn;
      }
    }
    if (closest) {
      ctx.beginPath();
      ctx.moveTo(closest.x, closest.y);
      ctx.lineTo(rn.x, rn.y);
      ctx.stroke();
    }
  }
  ctx.restore();

  // Draw Recommended nodes (colored dots + labels)
  for (const rn of recNodes) {
    const col = DOMAIN_COLORS[rn.type] || '#2563eb';

    // Dot glow & core
    ctx.beginPath();
    ctx.arc(rn.x, rn.y, 4.5, 0, Math.PI * 2);
    ctx.fillStyle = col;
    ctx.fill();
    ctx.strokeStyle = '#faf8f2';
    ctx.lineWidth = 1.2;
    ctx.stroke();

    // Small title label
    ctx.font = '600 8.5px "JetBrains Mono", monospace';
    ctx.fillStyle = '#1c1917';
    ctx.textAlign = rn.x >= mcx ? 'left' : 'right';
    const textOffset = rn.x >= mcx ? 7 : -7;
    ctx.fillText(truncate(rn.name, 14), rn.x + textOffset, rn.y + 3);
  }

  // Draw Seed nodes (Ruby double rings + labels)
  for (const sn of seedNodes) {
    ctx.beginPath();
    ctx.arc(sn.x, sn.y, 8, 0, Math.PI * 2);
    ctx.fillStyle = 'rgba(225, 29, 72, 0.2)';
    ctx.fill();

    ctx.beginPath();
    ctx.arc(sn.x, sn.y, 5, 0, Math.PI * 2);
    ctx.fillStyle = '#e11d48';
    ctx.fill();
    ctx.strokeStyle = '#faf8f2';
    ctx.lineWidth = 1.5;
    ctx.stroke();

    ctx.font = '700 8.5px "JetBrains Mono", monospace';
    ctx.fillStyle = '#9f1239';
    ctx.textAlign = 'center';
    ctx.fillText(truncate(sn.name, 12), sn.x, sn.y - 9);
  }

  // Mini-map legend footer
  ctx.font = '500 8px "JetBrains Mono", monospace';
  ctx.fillStyle = '#78716c';
  ctx.textAlign = 'center';
  ctx.fillText(
    isPt ? '● SEMENTES (RUBI)   ○ DESCOBERTAS TRANSMIDIÁTICAS   ··· PONTES TOPOLÓGICAS' : '● SEEDS (RUBY)   ○ TRANSMEDIA DISCOVERIES   ··· TOPOLOGICAL BRIDGES',
    mcx,
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
    const typeLabel = `[${(item.type || 'ITEM').toUpperCase()}]`;
    const title = item.title || item.name || item.id;
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
  // Generate barcode bars from taste hash
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

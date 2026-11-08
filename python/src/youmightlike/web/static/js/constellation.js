// Dual-Mode Visualization Engine: Galaxy (Obsidian) & Flow (Sankey)
// Formatted with Apple HIG Typography, floating cards, and dot grid canvas
import { DOMAIN_COLORS, getRawType } from './config.js';

function drawRoundRect(ctx, x, y, w, h, r) {
  if (typeof ctx.roundRect === 'function') {
    ctx.roundRect(x, y, w, h, r);
  } else {
    ctx.moveTo(x + r, y);
    ctx.lineTo(x + w - r, y);
    ctx.arcTo(x + w, y, x + w, y + r, r);
    ctx.lineTo(x + w, y + h - r);
    ctx.arcTo(x + w, y + h, x + w - r, y + h, r);
    ctx.lineTo(x + r, y + h);
    ctx.arcTo(x, y + h, x, y + h - r, r);
    ctx.lineTo(x, y + r);
    ctx.arcTo(x, y, x + r, y, r);
  }
}

function truncateText(ctx, text, maxWidth) {
  if (!text) return '';
  if (ctx.measureText(text).width <= maxWidth) return text;
  let low = 0, high = text.length;
  while (low < high) {
    const mid = Math.floor((low + high + 1) / 2);
    if (ctx.measureText(text.slice(0, mid) + '…').width <= maxWidth) {
      low = mid;
    } else {
      high = mid - 1;
    }
  }
  return text.slice(0, low) + '…';
}

function getDomainBadgeInfo(rawType, isSeed, isAntiSeed) {
  if (isAntiSeed) {
    return { text: 'REPULSION', bg: '#fee2e2', color: '#b91c1c' };
  }
  if (isSeed) {
    return { text: 'SEED', bg: '#1d1d1f', color: '#ffffff' };
  }
  switch (rawType) {
    case 'movie':
      return { text: 'CINEMA', bg: '#fef3c7', color: '#b45309' };
    case 'artist':
      return { text: 'ARTIST', bg: '#ffedd5', color: '#c2410c' };
    case 'track':
      return { text: 'TRACK', bg: '#e0f2fe', color: '#0369a1' };
    case 'game':
      return { text: 'GAME', bg: '#ede9fe', color: '#6d28d9' };
    case 'series':
      return { text: 'SERIES', bg: '#fae8ff', color: '#7e22ce' };
    default:
      return { text: 'WORK', bg: '#f1f5f9', color: '#475569' };
  }
}

export class ConstellationEngine {
  constructor(canvasEl, callbacks = {}) {
    this.canvasEl = canvasEl;
    this.ctx = canvasEl ? canvasEl.getContext('2d') : null;
    this.callbacks = {
      onNodeClick: callbacks.onNodeClick || (() => {}),
      onNodeDoubleClick: callbacks.onNodeDoubleClick || (() => {}),
      onHoverChange: callbacks.onHoverChange || (() => {}),
      onBackgroundClick: callbacks.onBackgroundClick || (() => {}),
    };

    // Graph state
    this.fullServerGraph = { nodes: [], links: [] };
    this.profileSeeds = [];
    this.options = {
      layoutMode: 'galaxy', // 'galaxy' (Obsidian) | 'flow' (Sankey)
      onlySharedBridges: false,
      hideTrackNodes: false,
      spacingScale: 1.15,
    };

    this.canvasNodes = [];
    this.canvasLinks = [];
    this.canvasAdj = new Map();
    this.nodeMap = new Map();

    // Viewport transform (pan & zoom)
    this.viewTransform = { x: 0, y: 0, k: 1 };
    this.selectedNodeIds = [];
    this.hoveredId = null;

    // Interaction state
    this.draggingNode = null;
    this.isPanning = false;
    this.panStartX = 0;
    this.panStartY = 0;
    this.pointerDownX = 0;
    this.pointerDownY = 0;
    this.lastClickTime = 0;
    this.lastClickedNodeId = null;
    this.rafPending = false;

    // Organic Physics Engine state (zero-freeze, zero-dependency)
    this.alpha = 0;
    this.alphaMin = 0.002;
    this.alphaDecay = 0.965;
    this.simRunning = false;
    this.seedPositions = new Map();
    this.nodeCluster = new Map();

    this.initEvents();
  }

  setOptions(options = {}) {
    this.options = { ...this.options, ...options };
    this.rebuild();
  }

  setGraphData(fullServerGraph, profileSeeds, options = {}) {
    this.fullServerGraph = fullServerGraph || { nodes: [], links: [] };
    this.profileSeeds = profileSeeds || [];
    this.options = { ...this.options, ...options };
    this.rebuild();
  }

  setSelectedNodes(selectedIds) {
    this.selectedNodeIds = [...selectedIds];
    this.scheduleDraw();
  }

  setHoveredNode(id) {
    if (this.hoveredId !== id) {
      this.hoveredId = id;
      this.scheduleDraw();
    }
  }

  hasNode(id) {
    return this.nodeMap ? this.nodeMap.has(id) : false;
  }

  scheduleDraw() {
    if (this.rafPending) return;
    this.rafPending = true;
    requestAnimationFrame(() => {
      this.rafPending = false;
      this.draw();
    });
  }

  wake(alpha = 0.4) {
    if (this.options.layoutMode !== 'galaxy') {
      this.scheduleDraw();
      return;
    }
    const n = this.canvasNodes.length;
    // Adapt alphaDecay dynamically: when graph has > 50 nodes, cool down faster
    // so physics stabilizes within ~1.2s instead of consuming CPU for 3+ seconds.
    this.alphaDecay = n > 70 ? 0.93 : (n > 40 ? 0.95 : 0.965);
    this.alpha = Math.max(this.alpha, alpha);
    if (!this.simRunning && this.canvasNodes.length > 0) {
      this.simRunning = true;
      this.tick();
    }
  }

  tick() {
    if (!this.simRunning) return;
    if (this.alpha < this.alphaMin || this.canvasNodes.length === 0) {
      this.simRunning = false;
      this.scheduleDraw();
      return;
    }

    this.stepPhysics();
    this.draw();
    this.alpha *= this.alphaDecay;

    requestAnimationFrame(() => this.tick());
  }

  stepPhysics() {
    const scale = this.options.spacingScale || 1.15;
    const idealDist = 150 * scale;
    const springStrength = 0.16 * this.alpha;

    // 1. Link springs (Hooke's Law - elastic pull between connected neighbors)
    for (const l of this.canvasLinks) {
      const s = l.source;
      const t = l.target;
      if (!s || !t) continue;
      const dx = t.x - s.x;
      const dy = t.y - s.y;
      const dist = Math.hypot(dx, dy) || 1;
      const diff = (dist - idealDist) / dist;
      const fx = dx * diff * springStrength;
      const fy = dy * diff * springStrength;

      if (s !== this.draggingNode && !s.isSeed) {
        s.vx += fx;
        s.vy += fy;
      }
      if (t !== this.draggingNode && !t.isSeed) {
        t.vx -= fx;
        t.vy -= fy;
      }
    }

    // 2. Collision / Elliptical Repulsion between pairs (Apple HIG Card Separation)
    const minSepX = 220 * scale;
    const minSepY = 80 * scale;
    const repStrength = 0.50 * this.alpha;

    for (let i = 0; i < this.canvasNodes.length; i++) {
      const a = this.canvasNodes[i];
      for (let j = i + 1; j < this.canvasNodes.length; j++) {
        const b = this.canvasNodes[j];
        let dx = b.x - a.x;
        let dy = b.y - a.y;
        let nx = dx / minSepX;
        let ny = dy / minSepY;
        let normDist = Math.hypot(nx, ny);
        if (normDist < 1.0) {
          if (normDist < 0.04) {
            dx = (i - j) * 1.5;
            dy = ((i + j) % 3 - 1) * 1.5;
            nx = dx / minSepX;
            ny = dy / minSepY;
            normDist = Math.hypot(nx, ny) || 0.1;
          }
          const push = (1.0 - normDist) * repStrength;
          const px = (nx / normDist) * push * (minSepX * 0.5);
          const py = (ny / normDist) * push * (minSepY * 0.5);

          if (a !== this.draggingNode && !a.isSeed) {
            a.vx -= px;
            a.vy -= py;
          }
          if (b !== this.draggingNode && !b.isSeed) {
            b.vx += px;
            b.vy += py;
          }
        }
      }
    }

    // 3. Tether to parent seed & corridor bounds
    const maxClusterRadius = Math.max(500, 260 + Math.sqrt(this.canvasNodes.length) * 45) * scale;
    for (const n of this.canvasNodes) {
      if (n === this.draggingNode || n.isSeed) continue;

      if (n.isBridge && n.seedNbrs && n.seedNbrs.length >= 2) {
        // Corridor bridge: attract towards center X = 0
        n.vx -= n.x * 0.06 * this.alpha;
        if (Math.abs(n.x) > 130 * scale) {
          n.x = Math.sign(n.x) * 130 * scale;
          n.vx *= 0.5;
        }
      } else {
        const assignedSeedId = this.nodeCluster.get(n.id);
        const pSeedPos = assignedSeedId ? this.seedPositions.get(assignedSeedId) : null;
        if (pSeedPos) {
          const dx = n.x - pSeedPos.x;
          const dy = n.y - pSeedPos.y;
          const dist = Math.hypot(dx, dy) || 1;
          if (dist > maxClusterRadius * 0.75) {
            const pull = (dist - maxClusterRadius * 0.75) * 0.035 * this.alpha;
            n.vx -= (dx / dist) * pull;
            n.vy -= (dy / dist) * pull;
          }
          if (dist > maxClusterRadius) {
            n.x = pSeedPos.x + (dx / dist) * maxClusterRadius;
            n.y = pSeedPos.y + (dy / dist) * maxClusterRadius;
            n.vx *= 0.5;
            n.vy *= 0.5;
          }
        }
      }
    }

    // 4. Hydrodynamic Friction & Velocity Integration
    const friction = 0.75;
    for (const n of this.canvasNodes) {
      if (n === this.draggingNode) continue;
      n.vx *= friction;
      n.vy *= friction;
      n.vx = Math.max(-18, Math.min(18, n.vx));
      n.vy = Math.max(-18, Math.min(18, n.vy));
      n.x += n.vx;
      n.y += n.vy;
    }
  }

  rebuild() {
    const rawNodes = this.fullServerGraph.nodes || [];
    const rawLinks = this.fullServerGraph.links || [];

    if (rawNodes.length === 0) {
      this.canvasNodes = [];
      this.canvasLinks = [];
      this.canvasAdj = new Map();
      this.nodeMap = new Map();
      this.scheduleDraw();
      return;
    }

    const scale = this.options.spacingScale || 1.15;
    const profileIdSet = new Set(this.profileSeeds.map(s => s.id));
    const negativeSeedsList = this.options.negativeSeeds || [];
    const negativeIdSet = new Set(negativeSeedsList.map(s => s.id));
    const rawAdj = new Map();
    const seedNeighbors = new Map();

    rawNodes.forEach(n => {
      rawAdj.set(n.id, new Set());
      seedNeighbors.set(n.id, new Set());
    });

    rawLinks.forEach(l => {
      const sId = typeof l.source === 'object' ? l.source.id : l.source;
      const tId = typeof l.target === 'object' ? l.target.id : l.target;
      if (rawAdj.has(sId) && rawAdj.has(tId) && sId !== tId) {
        rawAdj.get(sId).add(tId);
        rawAdj.get(tId).add(sId);
        if (profileIdSet.has(sId)) seedNeighbors.get(tId).add(sId);
        if (profileIdSet.has(tId)) seedNeighbors.get(sId).add(tId);
      }
    });

    // Filter nodes by active options
    const filteredNodes = rawNodes.filter(n => {
      if (negativeIdSet.has(n.id) || n.type === 'anti_seed' || n.type === 'antiSeed') return false;
      const isSeed = n.type === 'seed' || profileIdSet.has(n.id);
      if (isSeed) return true;
      if (this.options.hideTrackNodes && n.type === 'track') return false;
      const isBridge = ['keyword', 'developer', 'person', 'author'].includes(n.type);
      if (this.options.onlySharedBridges && isBridge) {
        return (rawAdj.get(n.id) || new Set()).size >= 2;
      }
      return true;
    });

    const seedsList = filteredNodes.filter(n => n.type === 'seed' || profileIdSet.has(n.id));
    const seedIdSet = new Set(seedsList.map(s => s.id));
    const numSeeds = seedsList.length;

    const existingNodeMap = this.nodeMap || new Map();
    const nodeById = new Map();

    const createNodeRecord = (n, x, y, isSeed, isBridge, isTrack) => {
      const sNbrs = Array.from(seedNeighbors.get(n.id) || []);
      const isBridgeNode = isBridge && !isSeed;
      const cardW = isBridgeNode ? 116 : (isSeed ? 168 : 156);
      const cardH = isBridgeNode ? 28 : (isSeed ? 50 : 46);
      let r = 15;

      const existing = existingNodeMap.get(n.id);
      const posX = existing && typeof existing.x === 'number' && !isNaN(existing.x) ? existing.x : x;
      const posY = existing && typeof existing.y === 'number' && !isNaN(existing.y) ? existing.y : y;

      return {
        ...n,
        rawType: getRawType(n),
        isSeed,
        isAntiSeed: false,
        isBridge,
        isTrack,
        sharedSeedsCount: sNbrs.length,
        seedNbrs: sNbrs,
        cardW,
        cardH,
        r,
        x: posX,
        y: posY,
        vx: existing ? existing.vx || 0 : 0,
        vy: existing ? existing.vy || 0 : 0,
      };
    };

    if (this.options.layoutMode === 'flow') {
      this.layoutFlowMode(filteredNodes, seedsList, seedNeighbors, rawAdj, scale, nodeById, createNodeRecord);
    } else {
      this.layoutGalaxyMode(filteredNodes, seedsList, seedIdSet, numSeeds, seedNeighbors, rawAdj, scale, nodeById, createNodeRecord);
    }

    // Assemble links
    const seenLinks = new Set();
    const links = [];
    const filteredAdj = new Map();
    nodeById.forEach((_, id) => filteredAdj.set(id, new Set()));

    rawLinks.forEach(l => {
      const sId = typeof l.source === 'object' ? l.source.id : l.source;
      const tId = typeof l.target === 'object' ? l.target.id : l.target;
      if (sId && tId && sId !== tId && nodeById.has(sId) && nodeById.has(tId)) {
        const key = [sId, tId].sort().join('__');
        if (!seenLinks.has(key)) {
          seenLinks.add(key);
          filteredAdj.get(sId).add(tId);
          filteredAdj.get(tId).add(sId);
          links.push({
            source: nodeById.get(sId),
            target: nodeById.get(tId),
            label: l.label,
            weight: l.weight || 1,
            isLeafTrackRel: (l.label === 'mesmo_álbum' || l.label === 'vibe_similar'),
          });
        }
      }
    });

    this.canvasNodes = Array.from(nodeById.values());
    this.canvasLinks = links;
    this.canvasAdj = filteredAdj;
    this.nodeMap = nodeById;

    this.fitView();
    if (this.options.layoutMode === 'galaxy') {
      this.wake(0.85);
    } else {
      this.scheduleDraw();
    }
  }

  // ---------------------------------------------------------------------------
  // MODE 1: Galaxy Clusters (Obsidian / Cosmograph)
  // Formatted with BFS cluster assignment, Golden-Angle spiral, and strict radial bounds
  // ---------------------------------------------------------------------------
  layoutGalaxyMode(filteredNodes, seedsList, seedIdSet, numSeeds, seedNeighbors, rawAdj, scale, nodeById, createNodeRecord) {
    const seedPositions = new Map();

    if (numSeeds === 1) {
      seedPositions.set(seedsList[0].id, { x: 0, y: 0 });
    } else if (numSeeds === 2) {
      const dist = 210 * scale;
      seedPositions.set(seedsList[0].id, { x: -dist, y: 0 });
      seedPositions.set(seedsList[1].id, { x: dist, y: 0 });
    } else {
      const rCenter = Math.max(140, 75 + numSeeds * 32) * scale;
      seedsList.forEach((s, idx) => {
        const a = (2 * Math.PI * idx) / numSeeds - Math.PI / 2;
        seedPositions.set(s.id, {
          x: Math.cos(a) * rCenter,
          y: Math.sin(a) * rCenter,
        });
      });
    }

    // BFS Shortest-Path: Every node in the universe belongs to its closest seed cluster!
    // This completely eliminates runaway "orphan" nodes!
    const nodeCluster = new Map();
    const distToSeed = new Map();
    const queue = [];

    seedsList.forEach(s => {
      nodeCluster.set(s.id, s.id);
      distToSeed.set(s.id, 0);
      queue.push(s.id);
    });

    while (queue.length > 0) {
      const u = queue.shift();
      const cSeed = nodeCluster.get(u);
      const d = distToSeed.get(u);
      const nbrs = rawAdj.get(u) || new Set();
      for (const v of nbrs) {
        if (!distToSeed.has(v)) {
          distToSeed.set(v, d + 1);
          nodeCluster.set(v, cSeed);
          queue.push(v);
        }
      }
    }

    this.seedPositions = seedPositions;
    this.nodeCluster = nodeCluster;

    const clusterChildren = new Map();
    seedsList.forEach(s => clusterChildren.set(s.id, []));
    const corridorBridges = [];

    filteredNodes.forEach(n => {
      if (seedIdSet.has(n.id)) return;

      const sNbrs = Array.from(seedNeighbors.get(n.id) || []);
      const isBridge = ['keyword', 'developer', 'person', 'author'].includes(n.type);

      // Multi-seed bridge -> place in central corridor between galaxies
      if (isBridge && sNbrs.length >= 2) {
        corridorBridges.push(n);
      } else {
        const assignedSeed = nodeCluster.get(n.id) || seedsList[0]?.id;
        if (assignedSeed && clusterChildren.has(assignedSeed)) {
          clusterChildren.get(assignedSeed).push(n);
        } else {
          clusterChildren.get(seedsList[0].id).push(n);
        }
      }
    });

    // Register seeds
    seedsList.forEach(s => {
      const pos = seedPositions.get(s.id) || { x: 0, y: 0 };
      nodeById.set(s.id, createNodeRecord(s, pos.x, pos.y, true, false, false));
    });

    // 2. Distribute children using natural Golden-Angle Fermat Spiral around each seed
    const GOLDEN_ANGLE = 2.39996; // 137.5 degrees (nature's optimal 2D packing)

    seedsList.forEach(s => {
      const pos = seedPositions.get(s.id) || { x: 0, y: 0 };
      const children = clusterChildren.get(s.id) || [];
      const m = children.length;
      if (m === 0) return;

      children.forEach((c, i) => {
        const angle = (i + 1) * GOLDEN_ANGLE;
        // Natural staggered orbits: spacious core and breathing room for cards
        const rOrbit = (130 + Math.sqrt(i + 1) * 60) * scale;
        const x = pos.x + Math.cos(angle) * rOrbit;
        const y = pos.y + Math.sin(angle) * rOrbit;
        const isBridge = ['keyword', 'developer', 'person', 'author'].includes(c.type);
        const isTrack = c.type === 'track';
        nodeById.set(c.id, createNodeRecord(c, x, y, false, isBridge, isTrack));
      });
    });

    // 3. Place corridor bridges vertically in center corridor
    const bCount = corridorBridges.length;
    corridorBridges.forEach((b, i) => {
      const y = (i - (bCount - 1) / 2) * 65 * scale;
      const x = (i % 2 - 0.5) * 45 * scale;
      nodeById.set(b.id, createNodeRecord(b, x, y, false, true, false));
    });

    // 4. Bounded Relaxation: strictly prevent overlapping cards and keep clusters comfortable
    const allNodes = Array.from(nodeById.values());
    const minSepX = 220 * scale;
    const minSepY = 80 * scale;
    const maxClusterRadius = Math.max(550, 240 + Math.sqrt(allNodes.length) * 55) * scale;

    for (let iter = 0; iter < 60; iter++) {
      // Repulsion between overlapping pairs (corrected normalized elliptical distance)
      for (let i = 0; i < allNodes.length; i++) {
        for (let j = i + 1; j < allNodes.length; j++) {
          const a = allNodes[i];
          const b = allNodes[j];
          let dx = b.x - a.x;
          let dy = b.y - a.y;
          let nx = dx / minSepX;
          let ny = dy / minSepY;
          let normDist = Math.hypot(nx, ny);
          if (normDist < 0.05) {
            dx = (i - j) * 1.5;
            dy = ((i + j) % 3 - 1) * 1.5;
            nx = dx / minSepX;
            ny = dy / minSepY;
            normDist = Math.hypot(nx, ny) || 0.1;
          }
          if (normDist < 1.0) {
            const push = (1.0 - normDist) * 0.65;
            const px = (nx / normDist) * push * (minSepX * 0.5);
            const py = (ny / normDist) * push * (minSepY * 0.5);
            if (!a.isSeed) { a.x -= px; a.y -= py; }
            if (!b.isSeed) { b.x += px; b.y += py; }
          }
        }
      }

      // Strict Cluster Clamping: Enforces that each discovery stays within its galaxy boundary!
      for (const n of allNodes) {
        if (n.isSeed) continue;
        const sNbrs = n.seedNbrs || [];
        if (sNbrs.length >= 2) {
          // Corridor bridge: clamp horizontal drift
          if (Math.abs(n.x) > 130 * scale) {
            n.x = Math.sign(n.x) * 130 * scale;
          }
        } else {
          // Cluster node: clamp to max radius from parent seed
          const assignedSeedId = nodeCluster.get(n.id);
          const pSeedPos = assignedSeedId ? seedPositions.get(assignedSeedId) : null;
          if (pSeedPos) {
            const dx = n.x - pSeedPos.x;
            const dy = n.y - pSeedPos.y;
            const dist = Math.hypot(dx, dy) || 1;
            if (dist > maxClusterRadius) {
              n.x = pSeedPos.x + (dx / dist) * maxClusterRadius;
              n.y = pSeedPos.y + (dy / dist) * maxClusterRadius;
            }
          }
        }
      }
    }
  }

  // ---------------------------------------------------------------------------
  // MODE 2: Flow / Sankey Columns (Sementes -> Pontes -> Descobertas)
  // ---------------------------------------------------------------------------
  layoutFlowMode(filteredNodes, seedsList, seedNeighbors, rawAdj, scale, nodeById, createNodeRecord) {
    const seedIdSet = new Set(seedsList.map(s => s.id));
    const bridgeList = [];
    const discoveryList = [];

    filteredNodes.forEach(n => {
      if (seedIdSet.has(n.id)) return;
      const isBridge = ['keyword', 'developer', 'person', 'author'].includes(n.type);
      if (isBridge) {
        bridgeList.push(n);
      } else {
        discoveryList.push(n);
      }
    });

    discoveryList.sort((a, b) => {
      const typePrio = (t) => {
        if (t === 'artist') return 1;
        if (t === 'track') return 2;
        if (t === 'game') return 3;
        if (t === 'movie') return 4;
        if (t === 'series') return 5;
        return 6;
      };
      return typePrio(a.type) - typePrio(b.type);
    });

    const colX_Seeds = -340 * scale;
    const colX_Bridges = -20 * scale;
    const colX_Discs = 340 * scale;

    const sCount = seedsList.length;
    const sSpacing = Math.max(85, 480 / Math.max(1, sCount)) * scale;
    seedsList.forEach((s, i) => {
      const y = (i - (sCount - 1) / 2) * sSpacing;
      nodeById.set(s.id, createNodeRecord(s, colX_Seeds, y, true, false, false));
    });

    const bCount = bridgeList.length;
    const bSpacing = Math.min(52, 540 / Math.max(1, bCount)) * scale;
    bridgeList.forEach((b, i) => {
      const y = (i - (bCount - 1) / 2) * bSpacing;
      nodeById.set(b.id, createNodeRecord(b, colX_Bridges, y, false, true, false));
    });

    const dCount = discoveryList.length;
    const dSpacing = Math.min(50, 560 / Math.max(1, dCount)) * scale;
    discoveryList.forEach((d, i) => {
      const y = (i - (dCount - 1) / 2) * dSpacing;
      nodeById.set(d.id, createNodeRecord(d, colX_Discs, y, false, false, d.type === 'track'));
    });
  }

  fitView() {
    if (!this.canvasEl) return;
    const rect = this.canvasEl.getBoundingClientRect();
    if (rect.width === 0 || rect.height === 0) return;

    if (this.canvasNodes.length === 0) {
      this.viewTransform = { x: rect.width / 2, y: rect.height / 2, k: 1 };
      this.scheduleDraw();
      return;
    }

    let minX = Infinity, maxX = -Infinity, minY = Infinity, maxY = -Infinity;
    for (const n of this.canvasNodes) {
      if (typeof n.x !== 'number' || isNaN(n.x)) continue;
      const padR = 100;
      if (n.x - padR < minX) minX = n.x - padR;
      if (n.x + padR > maxX) maxX = n.x + padR;
      if (n.y - padR < minY) minY = n.y - padR;
      if (n.y + padR > maxY) maxY = n.y + padR;
    }

    const pad = 100;
    const spanX = Math.max(340, (maxX - minX) + pad * 2);
    const spanY = Math.max(240, (maxY - minY) + pad * 2);

    // Natural scale framing (comfortable overview, never cramped)
    const k = Math.min(1.05, Math.max(0.42, Math.min(rect.width / spanX, rect.height / spanY)));
    const cx = (minX + maxX) / 2;
    const cy = (minY + maxY) / 2;

    this.viewTransform = {
      x: rect.width / 2 - cx * k,
      y: rect.height / 2 - cy * k,
      k,
    };
    this.scheduleDraw();
  }

  focusNode(nodeId) {
    const node = this.canvasNodes.find(n => n.id === nodeId);
    if (node && this.canvasEl) {
      const rect = this.canvasEl.getBoundingClientRect();
      this.viewTransform.k = 1.25;
      this.viewTransform.x = rect.width / 2 - node.x * this.viewTransform.k;
      this.viewTransform.y = rect.height / 2 - node.y * this.viewTransform.k;
      this.scheduleDraw();
    }
  }

  draw() {
    if (!this.canvasEl || !this.ctx) return;
    const dpr = window.devicePixelRatio || 1;
    const rect = this.canvasEl.getBoundingClientRect();
    if (rect.width === 0 || rect.height === 0) return;

    if (this.canvasEl.width !== Math.round(rect.width * dpr) || this.canvasEl.height !== Math.round(rect.height * dpr)) {
      this.canvasEl.width = Math.round(rect.width * dpr);
      this.canvasEl.height = Math.round(rect.height * dpr);
    }

    const ctx = this.ctx;
    ctx.save();
    ctx.setTransform(dpr, 0, 0, dpr, 0, 0);

    // Apple porcelain canvas background
    ctx.fillStyle = '#fbfbfd';
    ctx.fillRect(0, 0, rect.width, rect.height);

    if (this.canvasNodes.length === 0) {
      ctx.restore();
      return;
    }

    ctx.translate(this.viewTransform.x, this.viewTransform.y);
    ctx.scale(this.viewTransform.k, this.viewTransform.k);

    // Apple subtle canvas dot grid
    const gridSize = 32;
    const startX = Math.floor((-this.viewTransform.x / this.viewTransform.k) / gridSize) * gridSize;
    const startY = Math.floor((-this.viewTransform.y / this.viewTransform.k) / gridSize) * gridSize;
    const endX = startX + (rect.width / this.viewTransform.k) + gridSize * 2;
    const endY = startY + (rect.height / this.viewTransform.k) + gridSize * 2;

    ctx.fillStyle = 'rgba(0, 0, 0, 0.035)';
    const dotR = 1.0 / this.viewTransform.k;
    for (let gx = startX; gx <= endX; gx += gridSize) {
      for (let gy = startY; gy <= endY; gy += gridSize) {
        ctx.beginPath();
        ctx.arc(gx, gy, dotR, 0, Math.PI * 2);
        ctx.fill();
      }
    }

    const validSelected = this.selectedNodeIds.filter(id => this.nodeMap && this.nodeMap.has(id));
    const focusSet = new Set(validSelected);
    if (this.hoveredId && this.nodeMap && this.nodeMap.has(this.hoveredId)) {
      focusSet.add(this.hoveredId);
    }
    const hasFocus = focusSet.size > 0;
    const isFlowMode = this.options.layoutMode === 'flow';

    // 1. Draw Links (Apple smooth curves)
    for (const l of this.canvasLinks) {
      const s = l.source;
      const t = l.target;
      if (typeof s.x !== 'number' || typeof t.x !== 'number') continue;

      const sId = s.id;
      const tId = t.id;
      const bothFocused = hasFocus && focusSet.has(sId) && focusSet.has(tId);
      const oneFocused = hasFocus && (focusSet.has(sId) || focusSet.has(tId));

      ctx.beginPath();
      if (isFlowMode) {
        const cpX = (s.x + t.x) / 2;
        ctx.moveTo(s.x, s.y);
        ctx.bezierCurveTo(cpX, s.y, cpX, t.y, t.x, t.y);
      } else {
        const dx = t.x - s.x;
        const dy = t.y - s.y;
        const dist = Math.hypot(dx, dy);
        if (dist > 35) {
          const mx = (s.x + t.x) / 2;
          const my = (s.y + t.y) / 2;
          const nx = (-dy / dist) * 7;
          const ny = (dx / dist) * 7;
          ctx.moveTo(s.x, s.y);
          ctx.quadraticCurveTo(mx + nx, my + ny, t.x, t.y);
        } else {
          ctx.moveTo(s.x, s.y);
          ctx.lineTo(t.x, t.y);
        }
      }

      if (!hasFocus) {
        if (l.label === 'repelled_by' || s.isAntiSeed || t.isAntiSeed) {
          ctx.strokeStyle = 'rgba(239, 68, 68, 0.45)';
          ctx.lineWidth = 1.0 / this.viewTransform.k;
          ctx.setLineDash([3, 4]);
        } else if (l.isLeafTrackRel) {
          ctx.strokeStyle = 'rgba(203, 213, 225, 0.35)';
          ctx.lineWidth = 0.65 / this.viewTransform.k;
          ctx.setLineDash([3, 4]);
        } else {
          const isBridgeLink = s.isBridge || t.isBridge;
          ctx.strokeStyle = isBridgeLink ? 'rgba(203, 213, 225, 0.40)' : 'rgba(148, 163, 184, 0.30)';
          ctx.lineWidth = (isBridgeLink ? 0.75 : 1.1) / this.viewTransform.k;
          ctx.setLineDash([]);
        }
      } else if (bothFocused) {
        ctx.strokeStyle = '#0071e3';
        ctx.lineWidth = 2.2 / this.viewTransform.k;
        ctx.setLineDash([]);
      } else if (oneFocused) {
        ctx.strokeStyle = '#1d1d1f';
        ctx.lineWidth = 1.4 / this.viewTransform.k;
        ctx.setLineDash([]);
      } else {
        ctx.strokeStyle = 'rgba(226, 232, 240, 0.12)';
        ctx.lineWidth = 0.5 / this.viewTransform.k;
        ctx.setLineDash([]);
      }
      ctx.stroke();
    }
    ctx.setLineDash([]);

    // 2. Draw Apple HIG Floating Cards
    for (const n of this.canvasNodes) {
      if (typeof n.x !== 'number' || isNaN(n.x)) continue;

      const isSelected = focusSet.has(n.id);
      let isNeighbor = false;
      if (hasFocus && !isSelected) {
        const nbrs = this.canvasAdj.get(n.id);
        if (nbrs) {
          for (const fId of focusSet) {
            if (nbrs.has(fId)) {
              isNeighbor = true;
              break;
            }
          }
        }
      }
      const isHighlighted = !hasFocus || isSelected || isNeighbor;

      const isBridgeNode = n.isBridge && !n.isSeed;
      const cardW = n.cardW || (isBridgeNode ? 116 : (n.isSeed ? 168 : 156));
      const cardH = n.cardH || (isBridgeNode ? 28 : (n.isSeed ? 50 : 46));
      const x0 = n.x - cardW / 2;
      const y0 = n.y - cardH / 2;

      ctx.save();
      if (hasFocus && !isHighlighted) {
        ctx.globalAlpha = 0.14;
      }

      if (isBridgeNode) {
        // Tag / Bridge Pill
        ctx.save();
        ctx.shadowColor = 'rgba(0, 0, 0, 0.03)';
        ctx.shadowBlur = 4;
        ctx.shadowOffsetY = 1;

        ctx.beginPath();
        drawRoundRect(ctx, x0, y0, cardW, cardH, 7);
        ctx.fillStyle = isSelected ? '#ffffff' : (n.sharedSeedsCount >= 2 ? '#f1f5f9' : '#f8fafc');
        ctx.fill();
        ctx.restore();

        ctx.beginPath();
        drawRoundRect(ctx, x0, y0, cardW, cardH, 7);
        ctx.strokeStyle = isSelected ? '#0071e3' : (n.sharedSeedsCount >= 2 ? '#cbd5e1' : '#e2e8f0');
        ctx.lineWidth = (isSelected ? 1.8 : 1.0) / this.viewTransform.k;
        ctx.stroke();

        ctx.textAlign = 'center';
        ctx.textBaseline = 'middle';
        ctx.font = '500 9.5px -apple-system, BlinkMacSystemFont, "SF Pro Text", system-ui, sans-serif';
        ctx.fillStyle = isSelected ? '#0071e3' : (n.sharedSeedsCount >= 2 ? '#1e293b' : '#64748b');
        const tagText = (n.sharedSeedsCount >= 2) ? `${n.label} (${n.sharedSeedsCount})` : `# ${n.label}`;
        const truncatedTag = truncateText(ctx, tagText, cardW - 12);
        ctx.fillText(truncatedTag, n.x, n.y);
      } else {
        // Floating Apple Work / Seed Card
        const radius = 10;

        // Card Drop Shadow
        ctx.save();
        if (isSelected) {
          ctx.shadowColor = 'rgba(0, 113, 227, 0.22)';
          ctx.shadowBlur = 12;
          ctx.shadowOffsetY = 3;
        } else if (n.isSeed) {
          ctx.shadowColor = 'rgba(0, 0, 0, 0.08)';
          ctx.shadowBlur = 10;
          ctx.shadowOffsetY = 3;
        } else {
          ctx.shadowColor = 'rgba(0, 0, 0, 0.04)';
          ctx.shadowBlur = 6;
          ctx.shadowOffsetY = 2;
        }

        ctx.beginPath();
        drawRoundRect(ctx, x0, y0, cardW, cardH, radius);
        ctx.fillStyle = '#ffffff';
        ctx.fill();
        ctx.restore();

        // Card Border
        ctx.save();
        ctx.beginPath();
        drawRoundRect(ctx, x0, y0, cardW, cardH, radius);
        if (isSelected) {
          ctx.strokeStyle = '#0071e3';
          ctx.lineWidth = 2.0 / this.viewTransform.k;
        } else if (n.isAntiSeed) {
          ctx.strokeStyle = '#f87171';
          ctx.lineWidth = 1.4 / this.viewTransform.k;
        } else if (n.isSeed) {
          ctx.strokeStyle = '#1d1d1f';
          ctx.lineWidth = 1.5 / this.viewTransform.k;
        } else {
          ctx.strokeStyle = 'rgba(0, 0, 0, 0.08)';
          ctx.lineWidth = 1.0 / this.viewTransform.k;
        }
        ctx.stroke();
        ctx.restore();

        // Domain Badge
        const badge = getDomainBadgeInfo(n.rawType, n.isSeed, n.isAntiSeed);
        ctx.font = '700 7.5px -apple-system, BlinkMacSystemFont, "SF Pro Text", system-ui, sans-serif';
        const badgeMetrics = ctx.measureText(badge.text);
        const badgeW = badgeMetrics.width + 8;
        const badgeH = 12;
        const badgeX = x0 + 8;
        const badgeY = y0 + 6;

        ctx.save();
        ctx.beginPath();
        drawRoundRect(ctx, badgeX, badgeY, badgeW, badgeH, 3);
        ctx.fillStyle = badge.bg;
        ctx.fill();
        ctx.textAlign = 'center';
        ctx.textBaseline = 'middle';
        ctx.fillStyle = badge.color;
        ctx.fillText(badge.text, badgeX + badgeW / 2, badgeY + badgeH / 2 + 0.5);
        ctx.restore();

        // Work Title
        ctx.save();
        ctx.textAlign = 'left';
        ctx.textBaseline = 'middle';
        ctx.font = '600 11px -apple-system, BlinkMacSystemFont, "SF Pro Text", system-ui, sans-serif';
        ctx.fillStyle = '#1d1d1f';
        const maxTitleW = cardW - 16;
        const titleText = truncateText(ctx, n.label || '', maxTitleW);
        ctx.fillText(titleText, x0 + 8, y0 + 26);

        // Subtitle (Year · Creator / Subtitle)
        let subText = '';
        if (n.metadata) {
          const parts = [];
          if (n.metadata.year) parts.push(n.metadata.year);
          if (n.metadata.director) parts.push(`Dir. ${n.metadata.director}`);
          else if (n.metadata.developer) parts.push(n.metadata.developer);
          else if (n.metadata.album) parts.push(n.metadata.album);
          else if (n.metadata.author) parts.push(n.metadata.author);
          subText = parts.join(' · ');
        }
        if (!subText && n.desc) subText = n.desc;
        if (!subText && n.subtitle) subText = n.subtitle;

        if (subText) {
          ctx.font = '400 9px -apple-system, BlinkMacSystemFont, "SF Pro Text", system-ui, sans-serif';
          ctx.fillStyle = '#86868b';
          const subStr = truncateText(ctx, String(subText), maxTitleW);
          ctx.fillText(subStr, x0 + 8, y0 + 39);
        }
        ctx.restore();
      }

      ctx.restore();
    }

    ctx.restore();
  }

  screenToWorld(sx, sy) {
    return {
      x: (sx - this.viewTransform.x) / this.viewTransform.k,
      y: (sy - this.viewTransform.y) / this.viewTransform.k,
    };
  }

  findNodeAt(wx, wy) {
    const hitPad = 6 / this.viewTransform.k;
    for (let i = this.canvasNodes.length - 1; i >= 0; i--) {
      const n = this.canvasNodes[i];
      const isBridgeNode = n.isBridge && !n.isSeed;
      const halfW = (n.cardW || (isBridgeNode ? 116 : 156)) / 2 + hitPad;
      const halfH = (n.cardH || (isBridgeNode ? 28 : 46)) / 2 + hitPad;
      if (Math.abs(wx - n.x) <= halfW && Math.abs(wy - n.y) <= halfH) {
        return n;
      }
    }
    return null;
  }

  initEvents() {
    if (!this.canvasEl) return;

    this.resizeObserver = new ResizeObserver(() => {
      this.scheduleDraw();
    });
    this.resizeObserver.observe(this.canvasEl);

    this.canvasEl.addEventListener('pointerdown', (e) => {
      const rect = this.canvasEl.getBoundingClientRect();
      const sx = e.clientX - rect.left;
      const sy = e.clientY - rect.top;
      this.pointerDownX = sx;
      this.pointerDownY = sy;
      const world = this.screenToWorld(sx, sy);
      const hit = this.findNodeAt(world.x, world.y);

      if (hit) {
        this.draggingNode = hit;
        hit.vx = 0;
        hit.vy = 0;
      } else {
        this.isPanning = true;
        this.panStartX = sx - this.viewTransform.x;
        this.panStartY = sy - this.viewTransform.y;
      }
    });

    window.addEventListener('pointermove', (e) => {
      if (!this.canvasEl) return;
      const rect = this.canvasEl.getBoundingClientRect();
      const sx = e.clientX - rect.left;
      const sy = e.clientY - rect.top;

      if (this.draggingNode) {
        const world = this.screenToWorld(sx, sy);
        this.draggingNode.x = world.x;
        this.draggingNode.y = world.y;
        this.draggingNode.vx = 0;
        this.draggingNode.vy = 0;
        if (this.options.layoutMode === 'galaxy') {
          this.wake(0.35);
        } else {
          this.scheduleDraw();
        }
        return;
      }

      if (this.isPanning) {
        this.viewTransform.x = sx - this.panStartX;
        this.viewTransform.y = sy - this.panStartY;
        this.scheduleDraw();
        return;
      }

      if (sx >= 0 && sy >= 0 && sx <= rect.width && sy <= rect.height) {
        const world = this.screenToWorld(sx, sy);
        const hit = this.findNodeAt(world.x, world.y);
        const newHover = hit ? hit.id : null;
        if (newHover !== this.hoveredId) {
          this.hoveredId = newHover;
          this.canvasEl.style.cursor = hit ? 'pointer' : 'grab';
          this.callbacks.onHoverChange(newHover);
          this.scheduleDraw();
        }
      }
    });

    window.addEventListener('pointerup', (e) => {
      if (!this.canvasEl) return;
      const rect = this.canvasEl.getBoundingClientRect();
      const sx = e.clientX - rect.left;
      const sy = e.clientY - rect.top;
      const movedDist = Math.hypot(sx - this.pointerDownX, sy - this.pointerDownY);

      if (this.draggingNode && movedDist < 5) {
        const clicked = this.draggingNode;
        const now = Date.now();
        if (this.lastClickedNodeId === clicked.id && (now - this.lastClickTime) < 320) {
          this.callbacks.onNodeDoubleClick(clicked);
          this.lastClickTime = 0;
        } else {
          this.lastClickTime = now;
          this.lastClickedNodeId = clicked.id;
          const isMulti = e.shiftKey || e.ctrlKey || e.metaKey;
          this.callbacks.onNodeClick(clicked, isMulti);
        }
      } else if (this.isPanning && movedDist < 4) {
        this.callbacks.onBackgroundClick();
      }

      if (this.draggingNode) {
        if (movedDist >= 5 && this.options.layoutMode === 'galaxy') {
          this.wake(0.25);
        }
        this.draggingNode = null;
      }
      this.isPanning = false;
    });

    this.canvasEl.addEventListener('wheel', (e) => {
      e.preventDefault();
      const rect = this.canvasEl.getBoundingClientRect();
      const sx = e.clientX - rect.left;
      const sy = e.clientY - rect.top;
      const zoomFactor = e.deltaY < 0 ? 1.12 : 0.89;
      const nextK = Math.min(3.0, Math.max(0.35, this.viewTransform.k * zoomFactor));
      this.viewTransform.x = sx - (sx - this.viewTransform.x) * (nextK / this.viewTransform.k);
      this.viewTransform.y = sy - (sy - this.viewTransform.y) * (nextK / this.viewTransform.k);
      this.viewTransform.k = nextK;
      this.scheduleDraw();
    }, { passive: false });
  }

  destroy() {
    if (this.resizeObserver) {
      this.resizeObserver.disconnect();
    }
  }
}

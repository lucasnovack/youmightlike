// Dual-Mode Visualization Engine: Galaxy (Obsidian) & Flow (Sankey)
// Formatted with BFS cluster classification, Golden-Angle spiral dispersion, and strict bounding
import { DOMAIN_COLORS, getRawType } from './config.js';

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
    const idealDist = 95 * scale;
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

    // 2. Collision / Elliptical Repulsion between pairs
    const minSepX = 115 * scale;
    const minSepY = 52 * scale;
    const repStrength = 0.35 * this.alpha;

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
    const maxClusterRadius = 240 * scale;
    for (const n of this.canvasNodes) {
      if (n === this.draggingNode || n.isSeed) continue;

      if (n.isBridge && n.seedNbrs && n.seedNbrs.length >= 2) {
        // Corridor bridge: attract towards center X = 0
        n.vx -= n.x * 0.06 * this.alpha;
        if (Math.abs(n.x) > 95 * scale) {
          n.x = Math.sign(n.x) * 95 * scale;
          n.vx *= 0.5;
        }
      } else {
        const assignedSeedId = this.nodeCluster.get(n.id);
        const pSeedPos = assignedSeedId ? this.seedPositions.get(assignedSeedId) : null;
        if (pSeedPos) {
          const dx = n.x - pSeedPos.x;
          const dy = n.y - pSeedPos.y;
          const dist = Math.hypot(dx, dy) || 1;
          if (dist > maxClusterRadius * 0.70) {
            const pull = (dist - maxClusterRadius * 0.70) * 0.035 * this.alpha;
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
      let r = 11.5;
      if (isSeed) r = 15;
      else if (isBridge) r = sNbrs.length >= 2 ? 6.5 : 5.0;
      else if (isTrack) r = 8.5;

      const existing = existingNodeMap.get(n.id);
      const posX = existing && typeof existing.x === 'number' && !isNaN(existing.x) ? existing.x : x;
      const posY = existing && typeof existing.y === 'number' && !isNaN(existing.y) ? existing.y : y;

      return {
        ...n,
        rawType: getRawType(n),
        isSeed,
        isAntiSeed,
        isBridge,
        isTrack,
        sharedSeedsCount: sNbrs.length,
        seedNbrs: sNbrs,
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
        // Natural staggered orbits: compact core, spacious exterior
        const rOrbit = (90 + Math.sqrt(i + 1) * 36) * scale;
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
      const y = (i - (bCount - 1) / 2) * 52 * scale;
      const x = (i % 2 - 0.5) * 35 * scale;
      nodeById.set(b.id, createNodeRecord(b, x, y, false, true, false));
    });

    // 4. Bounded Relaxation: strictly prevent overlapping pills and keep clusters compact
    const allNodes = Array.from(nodeById.values());
    const minSepX = 115 * scale;
    const minSepY = 52 * scale;
    const maxClusterRadius = 240 * scale;

    for (let iter = 0; iter < 35; iter++) {
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
            const push = (1.0 - normDist) * 0.40;
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
          if (Math.abs(n.x) > 95 * scale) {
            n.x = Math.sign(n.x) * 95 * scale;
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
      const padR = 60;
      if (n.x - padR < minX) minX = n.x - padR;
      if (n.x + padR > maxX) maxX = n.x + padR;
      if (n.y - padR < minY) minY = n.y - padR;
      if (n.y + padR > maxY) maxY = n.y + padR;
    }

    const pad = 80;
    const spanX = Math.max(300, (maxX - minX) + pad * 2);
    const spanY = Math.max(220, (maxY - minY) + pad * 2);

    // Natural 1.0x scale framing (never shrink into micro-specks)
    const k = Math.min(1.15, Math.max(0.72, Math.min(rect.width / spanX, rect.height / spanY)));
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
    ctx.fillStyle = '#f8fafc';
    ctx.fillRect(0, 0, rect.width, rect.height);

    if (this.canvasNodes.length === 0) {
      ctx.restore();
      return;
    }

    ctx.translate(this.viewTransform.x, this.viewTransform.y);
    ctx.scale(this.viewTransform.k, this.viewTransform.k);

    const focusSet = new Set(this.selectedNodeIds);
    if (this.hoveredId) focusSet.add(this.hoveredId);
    const hasFocus = focusSet.size > 0;
    const isFlowMode = this.options.layoutMode === 'flow';

    // 1. Draw Links
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
        ctx.moveTo(s.x, s.y);
        ctx.lineTo(t.x, t.y);
      }

      if (!hasFocus) {
        if (l.label === 'repelled_by' || s.isAntiSeed || t.isAntiSeed) {
          ctx.strokeStyle = 'rgba(239, 68, 68, 0.50)';
          ctx.lineWidth = 1.1 / this.viewTransform.k;
          ctx.setLineDash([3, 4]);
        } else if (l.isLeafTrackRel) {
          ctx.strokeStyle = 'rgba(203, 213, 225, 0.35)';
          ctx.lineWidth = 0.65 / this.viewTransform.k;
          ctx.setLineDash([3, 4]);
        } else {
          const isBridgeLink = s.isBridge || t.isBridge;
          ctx.strokeStyle = isBridgeLink ? 'rgba(203, 213, 225, 0.45)' : 'rgba(148, 163, 184, 0.40)';
          ctx.lineWidth = (isBridgeLink ? 0.85 : 1.25) / this.viewTransform.k;
          ctx.setLineDash([]);
        }
      } else if (bothFocused) {
        ctx.strokeStyle = '#6366f1';
        ctx.lineWidth = 2.6 / this.viewTransform.k;
        ctx.setLineDash([]);
      } else if (oneFocused) {
        ctx.strokeStyle = '#334155';
        ctx.lineWidth = 1.7 / this.viewTransform.k;
        ctx.setLineDash([]);
      } else {
        ctx.strokeStyle = 'rgba(226, 232, 240, 0.12)';
        ctx.lineWidth = 0.5 / this.viewTransform.k;
        ctx.setLineDash([]);
      }
      ctx.stroke();
    }
    ctx.setLineDash([]);

    // 2. Draw Nodes
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

      let fill = '#64748b';
      if (n.isSeed) fill = DOMAIN_COLORS.seed;
      else if (n.rawType === 'game') fill = DOMAIN_COLORS.game;
      else if (n.rawType === 'artist') fill = DOMAIN_COLORS.artist;
      else if (n.rawType === 'track') fill = DOMAIN_COLORS.track;
      else if (n.rawType === 'movie') fill = DOMAIN_COLORS.movie;
      else if (n.rawType === 'series') fill = DOMAIN_COLORS.series;
      else if (n.rawType === 'developer') fill = DOMAIN_COLORS.developer;
      else if (n.rawType === 'keyword') fill = n.sharedSeedsCount >= 2 ? (DOMAIN_COLORS.bridgeShared || '#334155') : DOMAIN_COLORS.keyword;

      ctx.save();
      if (hasFocus && !isHighlighted) {
        ctx.globalAlpha = 0.14;
      }

      // Soft cosmic glowing aura for seeds
      if (n.isSeed) {
        const glowR = n.r * 2.8;
        const grad = ctx.createRadialGradient(n.x, n.y, n.r * 0.4, n.x, n.y, glowR);
        grad.addColorStop(0, 'rgba(99, 102, 241, 0.30)');
        grad.addColorStop(0.5, 'rgba(99, 102, 241, 0.10)');
        grad.addColorStop(1, 'rgba(99, 102, 241, 0)');
        ctx.fillStyle = grad;
        ctx.beginPath();
        ctx.arc(n.x, n.y, glowR, 0, Math.PI * 2);
        ctx.fill();
      }

      // Halo for Seeds & Focused Nodes
      if (n.isSeed || isSelected) {
        ctx.beginPath();
        const haloR = n.r + (isSelected ? 6.5 : 5.0) / this.viewTransform.k;
        ctx.arc(n.x, n.y, haloR, 0, Math.PI * 2);
        ctx.fillStyle = isSelected ? 'rgba(99, 102, 241, 0.28)' : 'rgba(99, 102, 241, 0.18)';
        ctx.fill();
        ctx.strokeStyle = isSelected ? '#4f46e5' : '#6366f1';
        ctx.lineWidth = (isSelected ? 2.4 : 1.8) / this.viewTransform.k;
        ctx.stroke();
      }

      // Diamond for Bridges vs Circle for Discoveries & Seeds
      if (n.isBridge && !n.isSeed) {
        const d = n.r * 1.15;
        ctx.beginPath();
        ctx.moveTo(n.x, n.y - d);
        ctx.lineTo(n.x + d, n.y);
        ctx.lineTo(n.x, n.y + d);
        ctx.lineTo(n.x - d, n.y);
        ctx.closePath();
        ctx.fillStyle = fill;
        ctx.fill();
        ctx.strokeStyle = '#ffffff';
        ctx.lineWidth = 1.2 / this.viewTransform.k;
        ctx.stroke();
      } else {
        ctx.beginPath();
        ctx.arc(n.x, n.y, n.r, 0, Math.PI * 2);
        ctx.fillStyle = n.isAntiSeed ? '#ef4444' : fill;
        ctx.fill();
        ctx.strokeStyle = n.isAntiSeed ? '#b91c1c' : (n.isSeed ? '#4f46e5' : '#ffffff');
        ctx.lineWidth = (n.isSeed || n.isAntiSeed ? 2.4 : 1.6) / this.viewTransform.k;
        ctx.stroke();
      }

      // 3. Clear, High-Legibility Labels (ALWAYS visible, clean rounded pill backdrop)
      const fontSize = Math.min(13, Math.max(9.5, (n.isSeed || n.isAntiSeed || isSelected ? 11.5 : 10.0) / Math.pow(this.viewTransform.k, 0.25)));
      ctx.font = `${n.isSeed || n.isAntiSeed || isSelected ? '700' : '600'} ${fontSize}px "JetBrains Mono", monospace`;

      let rawLabel = (n.isBridge && n.sharedSeedsCount >= 2)
        ? `${n.label} (${n.sharedSeedsCount})`
        : (n.label || '');

      if (!isSelected && this.hoveredId !== n.id && rawLabel.length > 18) {
        rawLabel = rawLabel.slice(0, 17) + '…';
      }

      const textMetrics = ctx.measureText(rawLabel);
      const textWidth = textMetrics.width;
      const textY = n.y + n.r + 4.0 / this.viewTransform.k;
      const pillX = n.x - textWidth / 2 - 4.5;
      const pillY = textY - 1;
      const pillW = textWidth + 9;
      const pillH = fontSize + 4;

      ctx.save();
      ctx.fillStyle = 'rgba(255, 255, 255, 0.94)';
      ctx.strokeStyle = isSelected ? '#818cf8' : (n.isAntiSeed ? '#fca5a5' : 'rgba(226, 232, 240, 0.9)');
      ctx.lineWidth = 1 / this.viewTransform.k;

      if (typeof ctx.roundRect === 'function') {
        ctx.beginPath();
        ctx.roundRect(pillX, pillY, pillW, pillH, 4);
        ctx.fill();
        ctx.stroke();
      } else {
        ctx.fillRect(pillX, pillY, pillW, pillH);
        ctx.strokeRect(pillX, pillY, pillW, pillH);
      }
      ctx.restore();

      ctx.textAlign = 'center';
      ctx.textBaseline = 'top';
      ctx.fillStyle = isSelected
        ? '#4f46e5'
        : (n.isAntiSeed ? '#b91c1c' : (n.isSeed ? '#1e1b4b' : (n.isBridge ? '#475569' : '#0f172a')));
      ctx.fillText(rawLabel, n.x, textY + 1);

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
    const hitPad = 8 / this.viewTransform.k;
    for (let i = this.canvasNodes.length - 1; i >= 0; i--) {
      const n = this.canvasNodes[i];
      if (Math.hypot(wx - n.x, wy - n.y) <= n.r + hitPad) {
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

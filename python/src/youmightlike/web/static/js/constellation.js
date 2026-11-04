// Deterministic, zero-freeze 3-ring radial constellation Canvas engine
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

    // Graph state (non-reactive for maximum performance)
    this.fullServerGraph = { nodes: [], links: [] };
    this.profileSeeds = [];
    this.options = {
      onlySharedBridges: false,
      hideTrackNodes: false,
    };

    this.canvasNodes = [];
    this.canvasLinks = [];
    this.canvasAdj = new Map();

    // Interaction & View Transform
    this.viewTransform = { x: 0, y: 0, k: 1 };
    this.selectedNodeIds = [];
    this.hoveredId = null;

    this.draggingNode = null;
    this.isPanning = false;
    this.panStartX = 0;
    this.panStartY = 0;
    this.pointerDownX = 0;
    this.pointerDownY = 0;
    this.lastClickTime = 0;
    this.lastClickedNodeId = null;
    this.rafPending = false;

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

  rebuild() {
    const rawNodes = this.fullServerGraph.nodes || [];
    const rawLinks = this.fullServerGraph.links || [];

    if (rawNodes.length === 0) {
      this.canvasNodes = [];
      this.canvasLinks = [];
      this.canvasAdj = new Map();
      this.scheduleDraw();
      return;
    }

    const profileIdSet = new Set(this.profileSeeds.map(s => s.id));
    const adj = new Map();
    const seedNeighbors = new Map();

    rawNodes.forEach(n => {
      adj.set(n.id, new Set());
      seedNeighbors.set(n.id, new Set());
    });

    rawLinks.forEach(l => {
      const sId = typeof l.source === 'object' ? l.source.id : l.source;
      const tId = typeof l.target === 'object' ? l.target.id : l.target;
      if (adj.has(sId) && adj.has(tId) && sId !== tId) {
        adj.get(sId).add(tId);
        adj.get(tId).add(sId);
        if (profileIdSet.has(sId)) seedNeighbors.get(tId).add(sId);
        if (profileIdSet.has(tId)) seedNeighbors.get(sId).add(tId);
      }
    });

    const filteredNodes = rawNodes.filter(n => {
      const isSeed = n.type === 'seed' || profileIdSet.has(n.id);
      if (isSeed) return true;
      if (this.options.hideTrackNodes && n.type === 'track') return false;
      const isBridge = ['keyword', 'developer', 'person', 'author'].includes(n.type);
      if (this.options.onlySharedBridges && isBridge) {
        return (adj.get(n.id) || new Set()).size >= 2;
      }
      return true;
    });

    const nodeById = new Map();
    filteredNodes.forEach(n => {
      const isSeed = n.type === 'seed' || profileIdSet.has(n.id);
      const isBridge = !isSeed && ['keyword', 'developer', 'person', 'author'].includes(n.type);
      const seedNbrs = Array.from(seedNeighbors.get(n.id) || []);
      const clone = {
        ...n,
        rawType: getRawType(n),
        isSeed,
        isBridge,
        sharedSeedsCount: seedNbrs.length,
        seedNbrs,
        ring: isSeed ? 0 : (isBridge ? 1 : 2),
        x: 0,
        y: 0,
        r: isSeed ? 12 : (isBridge ? (seedNbrs.length >= 2 ? 9 : 7) : 9),
      };
      nodeById.set(clone.id, clone);
    });

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
          });
        }
      }
    });

    // 1. Place Ring 0 (Profile Seeds) evenly on inner ring
    const seeds = Array.from(nodeById.values()).filter(n => n.ring === 0);
    const bridges = Array.from(nodeById.values()).filter(n => n.ring === 1);
    const discoveries = Array.from(nodeById.values()).filter(n => n.ring === 2);

    const R_SEED = seeds.length <= 1 ? 0 : Math.min(130, 75 + seeds.length * 14);
    const R_BRIDGE = 215;
    const R_DISC = 355;

    const nodeAngles = new Map();
    seeds.forEach((s, idx) => {
      const angle = (2 * Math.PI * idx) / Math.max(1, seeds.length) - Math.PI / 2;
      nodeAngles.set(s.id, angle);
      s.x = Math.cos(angle) * R_SEED;
      s.y = Math.sin(angle) * R_SEED;
    });

    // 2. Place Ring 1 (Bridges: #tags, studios, directors)
    const sharedBridges = bridges.filter(b => b.sharedSeedsCount >= 2);
    const singleBridges = bridges.filter(b => b.sharedSeedsCount < 2);

    sharedBridges.forEach((b, idx) => {
      let sx = 0, sy = 0;
      b.seedNbrs.forEach(sid => {
        const a = nodeAngles.get(sid) ?? 0;
        sx += Math.cos(a);
        sy += Math.sin(a);
      });
      sx /= b.seedNbrs.length;
      sy /= b.seedNbrs.length;
      const norm = Math.hypot(sx, sy);
      const spreadAngle = (2 * Math.PI * idx) / Math.max(1, sharedBridges.length);
      if (norm < 0.15) {
        const rInner = sharedBridges.length === 1 ? 0 : 48;
        b.x = Math.cos(spreadAngle) * rInner;
        b.y = Math.sin(spreadAngle) * rInner;
        nodeAngles.set(b.id, spreadAngle);
      } else {
        const baseAngle = Math.atan2(sy, sx) + ((idx % 5) - 2) * 0.22;
        const rMid = Math.max(55, R_SEED * 0.65);
        b.x = Math.cos(baseAngle) * rMid;
        b.y = Math.sin(baseAngle) * rMid;
        nodeAngles.set(b.id, baseAngle);
      }
    });

    singleBridges.forEach((b, idx) => {
      let angle;
      if (b.seedNbrs.length === 1 && nodeAngles.has(b.seedNbrs[0])) {
        const parentAngle = nodeAngles.get(b.seedNbrs[0]);
        angle = parentAngle + ((idx % 7) - 3) * 0.24;
      } else {
        angle = (2 * Math.PI * idx) / Math.max(1, singleBridges.length);
      }
      nodeAngles.set(b.id, angle);
      b.x = Math.cos(angle) * R_BRIDGE;
      b.y = Math.sin(angle) * R_BRIDGE;
    });

    // 3. Place Ring 2 (Discoveries) angularly aligned with their connected bridges/seeds
    discoveries.forEach((d, idx) => {
      const nbrs = Array.from(filteredAdj.get(d.id) || []);
      let sx = 0, sy = 0, count = 0;
      nbrs.forEach(nid => {
        if (nodeAngles.has(nid)) {
          const a = nodeAngles.get(nid);
          sx += Math.cos(a);
          sy += Math.sin(a);
          count++;
        }
      });
      let angle;
      if (count > 0 && Math.hypot(sx, sy) > 0.05) {
        angle = Math.atan2(sy, sx) + ((idx % 5) - 2) * 0.16;
      } else {
        angle = (2 * Math.PI * idx) / Math.max(1, discoveries.length);
      }
      nodeAngles.set(d.id, angle);
      d.x = Math.cos(angle) * R_DISC;
      d.y = Math.sin(angle) * R_DISC;
    });

    // 4. Fast, deterministic collision-free relaxation (45 iterations, < 1.5ms total)
    const allNodes = Array.from(nodeById.values());
    const minSep = 54;
    for (let iter = 0; iter < 45; iter++) {
      for (let i = 0; i < allNodes.length; i++) {
        for (let j = i + 1; j < allNodes.length; j++) {
          const a = allNodes[i];
          const b = allNodes[j];
          let dx = b.x - a.x;
          let dy = b.y - a.y;
          let dist = Math.hypot(dx, dy);
          if (dist < 0.1) {
            dx = (i - j) * 0.5;
            dy = ((i + j) % 3 - 1) * 0.5;
            dist = Math.hypot(dx, dy) || 1;
          }
          if (dist < minSep) {
            const push = ((minSep - dist) / dist) * 0.42;
            const px = dx * push;
            const py = dy * push;
            if (!a.isSeed) { a.x -= px; a.y -= py; }
            if (!b.isSeed) { b.x += px; b.y += py; }
          }
        }
      }
      for (const l of links) {
        const dx = l.target.x - l.source.x;
        const dy = l.target.y - l.source.y;
        const dist = Math.hypot(dx, dy) || 1;
        const ideal = (l.source.isSeed && l.target.isSeed) ? R_SEED * 1.4 : 135;
        const pull = ((dist - ideal) / dist) * 0.04;
        if (!l.source.isSeed) {
          l.source.x += dx * pull;
          l.source.y += dy * pull;
        }
        if (!l.target.isSeed) {
          l.target.x -= dx * pull;
          l.target.y -= dy * pull;
        }
      }
      for (const n of allNodes) {
        if (n.isSeed) continue;
        const targetR = n.ring === 1
          ? (n.sharedSeedsCount >= 2 ? Math.max(55, R_SEED * 0.65) : R_BRIDGE)
          : R_DISC;
        const r = Math.hypot(n.x, n.y) || 1;
        const kR = n.ring === 1 && n.sharedSeedsCount >= 2 ? 0.08 : 0.22;
        const factor = 1 + ((targetR - r) / r) * kR;
        n.x *= factor;
        n.y *= factor;
      }
    }

    this.canvasNodes = allNodes;
    this.canvasLinks = links;
    this.canvasAdj = filteredAdj;
    this.fitView();
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
      if (n.x < minX) minX = n.x;
      if (n.x > maxX) maxX = n.x;
      if (n.y < minY) minY = n.y;
      if (n.y > maxY) maxY = n.y;
    }
    const pad = 110;
    const spanX = Math.max(200, (maxX - minX) + pad * 2);
    const spanY = Math.max(200, (maxY - minY) + pad * 2);
    const k = Math.min(1.5, Math.max(0.45, Math.min(rect.width / spanX, rect.height / spanY)));
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
      this.viewTransform.k = 1.35;
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

    // Subtle concentric orbital guide rings
    ctx.save();
    ctx.strokeStyle = 'rgba(203, 213, 225, 0.55)';
    ctx.setLineDash([4, 6]);
    ctx.lineWidth = 1 / this.viewTransform.k;
    for (const radius of [110, 215, 355]) {
      ctx.beginPath();
      ctx.arc(0, 0, radius, 0, Math.PI * 2);
      ctx.stroke();
    }
    ctx.restore();

    const focusSet = new Set(this.selectedNodeIds);
    if (this.hoveredId) focusSet.add(this.hoveredId);
    const hasFocus = focusSet.size > 0;

    // Draw Links
    for (const l of this.canvasLinks) {
      const sId = l.source.id;
      const tId = l.target.id;
      const bothFocused = hasFocus && focusSet.has(sId) && focusSet.has(tId);
      const oneFocused = hasFocus && (focusSet.has(sId) || focusSet.has(tId));

      ctx.beginPath();
      ctx.moveTo(l.source.x, l.source.y);
      ctx.lineTo(l.target.x, l.target.y);

      if (!hasFocus) {
        ctx.strokeStyle = 'rgba(148, 163, 184, 0.45)';
        ctx.lineWidth = 1.2 / this.viewTransform.k;
      } else if (bothFocused) {
        ctx.strokeStyle = '#e11d48';
        ctx.lineWidth = 2.8 / this.viewTransform.k;
      } else if (oneFocused) {
        ctx.strokeStyle = '#0f172a';
        ctx.lineWidth = 1.9 / this.viewTransform.k;
      } else {
        ctx.strokeStyle = 'rgba(203, 213, 225, 0.22)';
        ctx.lineWidth = 0.9 / this.viewTransform.k;
      }
      ctx.stroke();
    }

    // Draw Nodes
    for (const n of this.canvasNodes) {
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
      if (n.rawType === 'game') fill = DOMAIN_COLORS.game;
      else if (n.rawType === 'artist') fill = DOMAIN_COLORS.artist;
      else if (n.rawType === 'track') fill = DOMAIN_COLORS.track;
      else if (n.rawType === 'movie') fill = DOMAIN_COLORS.movie;
      else if (n.rawType === 'series') fill = DOMAIN_COLORS.series;
      else if (n.rawType === 'developer') fill = DOMAIN_COLORS.developer;
      else if (n.rawType === 'keyword') fill = n.sharedSeedsCount >= 2 ? DOMAIN_COLORS.keyword : '#64748b';
      else if (n.isSeed) fill = DOMAIN_COLORS.seed;

      ctx.save();
      if (hasFocus && !isHighlighted) {
        ctx.globalAlpha = 0.22;
      }

      // Halo for Seeds & Focused Nodes
      if (n.isSeed || isSelected) {
        ctx.beginPath();
        ctx.arc(n.x, n.y, n.r + 5.5 / this.viewTransform.k, 0, Math.PI * 2);
        ctx.fillStyle = isSelected ? 'rgba(225, 29, 72, 0.25)' : 'rgba(225, 29, 72, 0.16)';
        ctx.fill();
        ctx.strokeStyle = '#e11d48';
        ctx.lineWidth = (isSelected ? 2.2 : 1.6) / this.viewTransform.k;
        ctx.stroke();
      }

      // Diamond for Bridges vs Circle for Seeds & Discoveries
      if (n.isBridge && !n.isSeed) {
        const d = n.r * 1.18;
        ctx.beginPath();
        ctx.moveTo(n.x, n.y - d);
        ctx.lineTo(n.x + d, n.y);
        ctx.lineTo(n.x, n.y + d);
        ctx.lineTo(n.x - d, n.y);
        ctx.closePath();
        ctx.fillStyle = fill;
        ctx.fill();
        ctx.strokeStyle = '#ffffff';
        ctx.lineWidth = 1.4 / this.viewTransform.k;
        ctx.stroke();
      } else {
        ctx.beginPath();
        ctx.arc(n.x, n.y, n.r, 0, Math.PI * 2);
        ctx.fillStyle = fill;
        ctx.fill();
        ctx.strokeStyle = n.isSeed ? '#e11d48' : '#ffffff';
        ctx.lineWidth = (n.isSeed ? 2.2 : 1.6) / this.viewTransform.k;
        ctx.stroke();
      }

      // Crisp Typography Label
      const fontSize = Math.max(9.5, (n.isSeed ? 11.5 : 10.2) / Math.sqrt(this.viewTransform.k));
      ctx.font = `${n.isSeed || isSelected ? '700' : '600'} ${fontSize}px "JetBrains Mono", monospace`;
      const labelText = (n.isBridge && n.sharedSeedsCount >= 2)
        ? `${n.label} (${n.sharedSeedsCount})`
        : n.label;

      ctx.textAlign = 'center';
      ctx.textBaseline = 'top';
      ctx.fillStyle = isSelected ? '#e11d48' : (n.isSeed ? '#0f172a' : '#1e293b');
      ctx.fillText(labelText, n.x, n.y + n.r + 3.5 / this.viewTransform.k);
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
        this.scheduleDraw();
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

      this.draggingNode = null;
      this.isPanning = false;
    });

    this.canvasEl.addEventListener('wheel', (e) => {
      e.preventDefault();
      const rect = this.canvasEl.getBoundingClientRect();
      const sx = e.clientX - rect.left;
      const sy = e.clientY - rect.top;
      const zoomFactor = e.deltaY < 0 ? 1.12 : 0.89;
      const nextK = Math.min(3.5, Math.max(0.25, this.viewTransform.k * zoomFactor));
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
